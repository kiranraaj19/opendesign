//! A "final render" still of the scene, as opposed to the real-time
//! viewport: a higher resolution, more MSAA samples, lit by an environment
//! (see [`crate::light`]) and drawn in floating point, then exposed and
//! tone mapped to 8-bit sRGB, so bright cloth keeps its fold shading
//! instead of clipping to flat white. With [`StillSettings::quality`] the
//! shader also adds a cheap per-pixel ambient-occlusion stand-in (see
//! `Globals::params`'s `y`). It draws the same [`Frame`] as the viewport,
//! with a fresh one-shot [`Renderer`]; there's no separate scene pipeline
//! to maintain.
//!
//! Also used for a turntable or drape-animation export: call this once per
//! frame with a camera from [`crate::camera::OrbitCamera::turntable`] (or a
//! stepped [`crate::camera::OrbitCamera`]) and write each result out.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu::{draw_offscreen, read_rgba8, Renderer, Targets, HDR_FORMAT};
use crate::light::{LightPreset, Lighting};
use crate::scene::{srgb_to_linear, Frame};
use crate::view::{compose_frame, FrameInput};

pub const TONEMAP_SHADER: &str = include_str!("tonemap.wgsl");

/// How the HDR image is brought into display range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToneMap {
    /// Khronos PBR Neutral: true base colours, highlights rolled off. Made
    /// for product shots.
    #[default]
    Neutral,
    /// The ACES filmic curve: punchier contrast, shifts colours slightly.
    Aces,
}

impl ToneMap {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "neutral" => Some(ToneMap::Neutral),
            "aces" => Some(ToneMap::Aces),
            _ => None,
        }
    }
}

/// Knobs for a still render, as opposed to the live viewport's.
#[derive(Debug, Clone, PartialEq)]
pub struct StillSettings {
    pub size: [u32; 2],
    /// MSAA sample count (`1` disables it).
    pub samples: u16,
    /// Turn on the ambient-occlusion approximation in `fs_lit` (and, with
    /// no `lighting`, a rim light) — off in the live viewport.
    pub quality: bool,
    /// The environment and key light; `None` keeps the viewport's
    /// two-light rig.
    pub lighting: Option<Lighting>,
    /// Multiplies the light before tone mapping; 1 is neutral.
    pub exposure: f32,
    pub tone_map: ToneMap,
}

impl Default for StillSettings {
    /// 1080p, 4x MSAA, quality on, the soft studio preset.
    fn default() -> Self {
        Self {
            size: [1920, 1080],
            samples: 4,
            quality: true,
            lighting: Some(LightPreset::default().lighting()),
            exposure: 1.0,
            tone_map: ToneMap::default(),
        }
    }
}

/// Uniforms for `tonemap.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Post {
    background: [f32; 4],
    params: [f32; 4],
}

/// Render one still of `input`'s scene offscreen into RGBA8 bytes —
/// `settings.size[0] * settings.size[1] * 4` of them, row-major, top to
/// bottom, sRGB with straight alpha. `background` (sRGB, as for a clear
/// colour) shows wherever the scene doesn't cover; give it alpha 0 for a
/// transparent background. Builds its own one-shot [`Renderer`] each
/// call, which is fine for an occasional final render or a handful of
/// turntable/animation frames; the live viewport keeps its own persistent
/// one instead (see `od_app`'s paint callback).
pub fn render_still<K: PartialEq>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    input: FrameInput<'_, K>,
    settings: &StillSettings,
    background: wgpu::Color,
) -> Vec<u8> {
    let targets = Targets {
        color: HDR_FORMAT,
        depth: Some(crate::DEPTH_FORMAT),
        samples: u32::from(settings.samples),
    };
    let mut renderer = Renderer::new(device, queue, targets);
    let camera = *input.camera;
    let mut frame: Frame = compose_frame(input);
    // The post pass encodes; the scene stays linear.
    frame.globals.params[0] = 0.0;
    frame.globals.params[1] = f32::from(settings.quality);
    if let Some(lighting) = &settings.lighting {
        frame.globals.apply_lighting(&camera, lighting);
        renderer.set_lighting(device, queue, &lighting.ibl);
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    let scene = draw_offscreen(
        device,
        queue,
        &mut encoder,
        &mut renderer,
        targets,
        &frame,
        settings.size,
        wgpu::Color::TRANSPARENT,
    );
    let image = tone_map(device, &mut encoder, &scene, settings, background);
    read_rgba8(device, queue, encoder, &image)
}

/// Record the post pass from the HDR `scene` into a new RGBA8 texture.
fn tone_map(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    scene: &wgpu::Texture,
    settings: &StillSettings,
    background: wgpu::Color,
) -> wgpu::Texture {
    let to_linear = |c: f64| srgb_to_linear((c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let post = Post {
        background: [
            to_linear(background.r),
            to_linear(background.g),
            to_linear(background.b),
            background.a as f32,
        ],
        params: [
            settings.exposure.max(0.0),
            f32::from(u8::from(settings.tone_map == ToneMap::Aces)),
            0.0,
            0.0,
        ],
    };
    let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("od_post"),
        contents: bytemuck::bytes_of(&post),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("od_tonemap"),
        source: wgpu::ShaderSource::Wgsl(TONEMAP_SHADER.into()),
    });
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("od_tonemap"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_post"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_post"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("od_post"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &scene.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: uniforms.as_entire_binding(),
            },
        ],
    });
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("od_final"),
        size: scene.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    {
        let view = output.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("od_tonemap"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{avatar_mesh, AvatarKind};
    use crate::camera::{OrbitCamera, ViewPreset};
    use crate::scene::{AvatarDraw, GridColors};
    use crate::view::Theme;
    use std::sync::Arc;

    fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        eprintln!("rendering on {:?}", adapter.get_info());
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn theme() -> Theme {
        Theme {
            avatar: [0.8, 0.2, 0.1, 1.0],
            pattern: [0.2, 0.3, 0.6, 1.0],
            selected: [1.0, 0.6, 0.1, 1.0],
            outline: [0.6, 0.7, 0.9, 1.0],
            grid: GridColors {
                minor: [0.1; 4],
                major: [0.2; 4],
                axis: [0.3; 4],
            },
            shadow: [0.0, 0.0, 0.0, 0.3],
        }
    }

    /// Renders offscreen at a non-default size with the quality flag on,
    /// when this machine has any GPU adapter (including a software one);
    /// skips otherwise, same as the other GPU-dependent tests in this
    /// crate.
    #[test]
    fn still_render_is_lit_and_the_right_size() {
        let Some((device, queue)) = headless_device() else {
            eprintln!("no GPU adapter; skipping the still-render test");
            return;
        };
        let kind = AvatarKind::Female;
        let mesh = Arc::new(avatar_mesh(kind, kind.default_measurements()));
        let mut camera = OrbitCamera::default();
        camera.set_view(ViewPreset::Front);
        camera.frame(mesh.bounds().unwrap(), 16.0 / 9.0);
        let theme = theme();
        let bounds = mesh.bounds().unwrap();
        let settings = StillSettings {
            size: [64, 36],
            samples: 1,
            quality: true,
            lighting: None,
            ..Default::default()
        };
        let pixels = render_still::<u32>(
            &device,
            &queue,
            FrameInput {
                camera: &camera,
                aspect: 16.0 / 9.0,
                encode_srgb: true,
                avatar: Some(AvatarDraw {
                    key: 1,
                    mesh,
                    color: theme.avatar,
                }),
                avatar_bounds: bounds,
                patterns: &[],
                selected: &[],
                theme: &theme,
                pattern_colors: None,
                pattern_materials: None,
                display: Default::default(),
            },
            &settings,
            wgpu::Color::BLACK,
        );
        assert_eq!(
            pixels.len(),
            (settings.size[0] * settings.size[1] * 4) as usize
        );
        let i = ((settings.size[1] / 2 * settings.size[0] + settings.size[0] / 2) * 4) as usize;
        let [r, g, b] = [pixels[i], pixels[i + 1], pixels[i + 2]];
        assert!(r > 60 && r > g + 20 && r > b + 20, "centre {r} {g} {b}");
    }

    #[test]
    fn tonemap_shader_is_valid() {
        let module = naga::front::wgsl::parse_str(TONEMAP_SHADER).expect("WGSL parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("WGSL validates");
    }

    /// A white avatar under the studio preset, at three exposures: a
    /// normal exposure stays below clipping white with visible shading,
    /// more light is brighter, and the background composites in where
    /// the scene is empty (transparent when asked).
    #[test]
    fn studio_light_keeps_white_from_clipping() {
        let Some((device, queue)) = headless_device() else {
            eprintln!("no GPU adapter; skipping the studio-light test");
            return;
        };
        let kind = AvatarKind::Female;
        let mesh = Arc::new(avatar_mesh(kind, kind.default_measurements()));
        let bounds = mesh.bounds().unwrap();
        let mut camera = OrbitCamera::default();
        camera.set_view(ViewPreset::Front);
        camera.frame(bounds, 1.0);
        let theme = theme();
        let render = |exposure: f32, background: wgpu::Color| {
            render_still::<u32>(
                &device,
                &queue,
                FrameInput {
                    camera: &camera,
                    aspect: 1.0,
                    encode_srgb: true,
                    avatar: Some(AvatarDraw {
                        key: 1,
                        mesh: mesh.clone(),
                        color: [1.0, 1.0, 1.0, 1.0],
                    }),
                    avatar_bounds: bounds,
                    patterns: &[],
                    selected: &[],
                    theme: &theme,
                    pattern_colors: None,
                    pattern_materials: None,
                    display: crate::view::Display {
                        backdrop: crate::view::Backdrop::None,
                        ..Default::default()
                    },
                },
                &StillSettings {
                    size: [48, 48],
                    samples: 1,
                    exposure,
                    ..Default::default()
                },
                background,
            )
        };
        // The torso's column of pixels, from chest to hips.
        let column = |pixels: &[u8]| -> Vec<u8> {
            (14..30)
                .map(|y| pixels[((y * 48 + 24) * 4) as usize])
                .collect()
        };
        let normal = render(1.0, wgpu::Color::BLACK);
        let torso = column(&normal);
        let (lo, hi) = (*torso.iter().min().unwrap(), *torso.iter().max().unwrap());
        assert!(hi < 255, "white clipped: {torso:?}");
        assert!(lo > 90, "too dark: {torso:?}");
        let bright = render(2.0, wgpu::Color::BLACK);
        let sum = |c: Vec<u8>| c.iter().map(|&v| u32::from(v)).sum::<u32>();
        assert!(sum(column(&bright)) > sum(torso.clone()));

        // Empty corners: the background, opaque or see-through.
        assert_eq!(&normal[..4], &[0, 0, 0, 255]);
        let clear = render(1.0, wgpu::Color::TRANSPARENT);
        assert_eq!(clear[3], 0, "transparent corner");
        let centre = ((24 * 48 + 24) * 4 + 3) as usize;
        assert_eq!(clear[centre], 255, "the avatar stays opaque");
    }
}
