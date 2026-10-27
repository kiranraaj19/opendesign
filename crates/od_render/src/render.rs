//! A "final render" still of the scene, as opposed to the real-time
//! viewport: a higher resolution, more MSAA samples, and (when
//! [`StillSettings::quality`] is set) a soft rim light and a cheap
//! per-pixel ambient-occlusion stand-in baked into `scene.wgsl`'s
//! `fs_lit` (see `Globals::params`'s `y`). Built on the same offscreen
//! path tests and `examples/snapshot.rs` already use
//! ([`crate::gpu::render_to_rgba`]), just with a fresh one-shot
//! [`Renderer`] and different knobs — there's no separate "final render"
//! pipeline to maintain.
//!
//! Also used for a turntable or drape-animation export: call this once per
//! frame with a camera from [`crate::camera::OrbitCamera::turntable`] (or a
//! stepped [`crate::camera::OrbitCamera`]) and write each result out.

use crate::gpu::{render_to_rgba, Renderer, Targets};
use crate::view::{compose_frame, FrameInput};

/// Knobs for a still render, as opposed to the live viewport's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StillSettings {
    pub size: [u32; 2],
    /// MSAA sample count (`1` disables it).
    pub samples: u16,
    /// Turn on the extra rim light and ambient-occlusion approximation in
    /// `fs_lit` — off in the live viewport, which is tuned for its plain
    /// two-light rig.
    pub quality: bool,
}

impl Default for StillSettings {
    /// 1080p, 4x MSAA, quality lighting on.
    fn default() -> Self {
        Self {
            size: [1920, 1080],
            samples: 4,
            quality: true,
        }
    }
}

/// Render one still of `input`'s scene offscreen into RGBA8 bytes —
/// `settings.size[0] * settings.size[1] * 4` of them, row-major, top to
/// bottom. Builds its own one-shot [`Renderer`] each call, which is fine
/// for an occasional final render or a handful of turntable/animation
/// frames; the live viewport keeps its own persistent one instead (see
/// `od_app`'s paint callback).
pub fn render_still<K: PartialEq>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    input: FrameInput<'_, K>,
    settings: StillSettings,
    clear: wgpu::Color,
) -> Vec<u8> {
    let targets = Targets {
        color: wgpu::TextureFormat::Rgba8Unorm,
        depth: Some(crate::DEPTH_FORMAT),
        samples: u32::from(settings.samples),
    };
    let mut renderer = Renderer::new(device, queue, targets);
    let mut frame = compose_frame(input);
    frame.globals.params[1] = f32::from(settings.quality);
    render_to_rgba(
        device,
        queue,
        &mut renderer,
        targets,
        &frame,
        settings.size,
        clear,
    )
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
                selected: None,
                theme: &theme,
                pattern_colors: None,
                pattern_materials: None,
            },
            settings,
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
}
