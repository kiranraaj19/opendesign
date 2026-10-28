//! wgpu pipelines and buffers for a [`Frame`]. The renderer draws into a
//! render pass someone else owns (egui's, in the app), so it only needs to
//! know that pass's formats.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use crate::scene::{FlatVertex, Frame, Globals, LitVertex};

pub const SHADER: &str = include_str!("scene.wgsl");

/// The render pass the renderer draws into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Targets {
    pub color: wgpu::TextureFormat,
    /// Without depth the view still draws, but nothing hides what's behind.
    pub depth: Option<wgpu::TextureFormat>,
    pub samples: u32,
}

pub struct Renderer {
    lit: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    encode_srgb: bool,
    avatar: Option<(u64, MeshBuffers)>,
    surfaces: Growable,
    surface_indices: Growable,
    translucent_vertices: Growable,
    translucent_indices: Growable,
    line_vertices: Growable,
    shadow_vertices: Growable,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound for the avatar and the plain `surfaces` batch, which have no
    /// texture of their own: a solid white pixel, so sampling it in the
    /// shader is the same as not texturing at all.
    default_texture: wgpu::BindGroup,
    /// One bind group per distinct [`crate::material::TextureImage::id`]
    /// seen so far, kept across frames so the same fabric's texture isn't
    /// re-uploaded to the GPU every frame.
    texture_cache: HashMap<u64, wgpu::BindGroup>,
    /// This frame's textured draws, rebuilt in [`Self::prepare`] (their
    /// geometry can change every frame, e.g. while draping).
    textured: Vec<TexturedBatch>,
}

struct TexturedBatch {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    bind_group: wgpu::BindGroup,
}

struct MeshBuffers {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

/// A buffer rewritten every frame, replaced only when it has to grow.
struct Growable {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: Option<wgpu::Buffer>,
    /// Elements written this frame.
    count: u32,
}

impl Growable {
    fn new(label: &'static str, usage: wgpu::BufferUsages) -> Self {
        Self {
            label,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            buffer: None,
            count: 0,
        }
    }

    fn write<T: bytemuck::Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[T]) {
        self.count = data.len() as u32;
        if data.is_empty() {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let size = bytes.len() as u64;
        if self.buffer.as_ref().is_none_or(|b| b.size() < size) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: size.next_power_of_two().max(1024),
                usage: self.usage,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, 0, bytes);
        }
    }

    fn slice(&self) -> Option<wgpu::BufferSlice<'_>> {
        let buffer = self.buffer.as_ref()?;
        (self.count > 0).then(|| buffer.slice(..))
    }
}

const LIT_ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2];
const FLAT_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, targets: Targets) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("od_scene"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("od_globals"),
            contents: bytemuck::bytes_of(&Globals::new(&Default::default(), 1.0, false)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("od_globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<Globals>() as u64),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("od_globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("od_texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("od_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let default_texture = make_texture_bind_group(
            device,
            queue,
            &texture_layout,
            &sampler,
            1,
            1,
            &[255, 255, 255, 255],
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("od_scene"),
            bind_group_layouts: &[Some(&layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let pipeline = |label: &str, kind: Kind| {
            let (vs, fs, stride, attributes): (_, _, _, &[wgpu::VertexAttribute]) = match kind {
                Kind::Lit | Kind::Translucent => {
                    ("vs_lit", "fs_lit", size_of::<LitVertex>(), &LIT_ATTRIBUTES)
                }
                Kind::Lines | Kind::Shadow => (
                    "vs_flat",
                    "fs_flat",
                    size_of::<FlatVertex>(),
                    &FLAT_ATTRIBUTES,
                ),
            };
            let depth_stencil = targets.depth.map(|format| wgpu::DepthStencilState {
                format,
                // Only solid surfaces hide things; lines and the shadow
                // are tested against them but don't occlude.
                depth_write_enabled: Some(kind == Kind::Lit),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: if kind == Kind::Lines {
                        wgpu::PrimitiveTopology::LineList
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: (kind == Kind::Lit).then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil,
                multisample: wgpu::MultisampleState {
                    count: targets.samples.max(1),
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: targets.color,
                        blend: Some(if kind == Kind::Lit {
                            wgpu::BlendState::REPLACE
                        } else {
                            wgpu::BlendState::ALPHA_BLENDING
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        use wgpu::BufferUsages as U;
        Self {
            lit: pipeline("od_lit", Kind::Lit),
            translucent: pipeline("od_translucent", Kind::Translucent),
            lines: pipeline("od_lines", Kind::Lines),
            shadow: pipeline("od_shadow", Kind::Shadow),
            globals,
            bind_group,
            encode_srgb: !targets.color.is_srgb(),
            avatar: None,
            surfaces: Growable::new("od_surfaces", U::VERTEX),
            surface_indices: Growable::new("od_surface_indices", U::INDEX),
            translucent_vertices: Growable::new("od_translucent", U::VERTEX),
            translucent_indices: Growable::new("od_translucent_indices", U::INDEX),
            line_vertices: Growable::new("od_lines", U::VERTEX),
            shadow_vertices: Growable::new("od_shadow", U::VERTEX),
            texture_layout,
            sampler,
            default_texture,
            texture_cache: HashMap::new(),
            textured: Vec::new(),
        }
    }

    /// Whether colours must be sRGB-encoded in the shader, for
    /// [`Globals::new`].
    pub fn encode_srgb(&self) -> bool {
        self.encode_srgb
    }

    /// Upload this frame's data. Call before [`Renderer::paint`].
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) {
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&frame.globals));

        match &frame.avatar {
            Some(avatar) if self.avatar.as_ref().map(|a| a.0) != Some(avatar.key) => {
                let vertices = crate::scene::lit_vertices(&avatar.mesh, avatar.color);
                let buffers = MeshBuffers {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("od_avatar"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("od_avatar_indices"),
                        contents: bytemuck::cast_slice(&avatar.mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    count: avatar.mesh.indices.len() as u32,
                };
                self.avatar = Some((avatar.key, buffers));
            }
            Some(_) => {}
            None => self.avatar = None,
        }

        self.surfaces.write(device, queue, &frame.surfaces);
        self.surface_indices
            .write(device, queue, &frame.surface_indices);
        self.translucent_vertices
            .write(device, queue, &frame.translucent);
        self.translucent_indices
            .write(device, queue, &frame.translucent_indices);
        self.line_vertices.write(device, queue, &frame.lines);
        self.shadow_vertices.write(device, queue, &frame.shadow);

        // Rebuilt every frame: a textured surface's geometry can change
        // (e.g. while draping), but its texture is cached by id below so
        // re-uploading the pixels only happens the first time that id is
        // seen.
        self.textured = frame
            .textured
            .iter()
            .filter(|t| !t.indices.is_empty())
            .map(|textured| {
                let bind_group = match &textured.material.texture {
                    Some(tex) => self
                        .texture_cache
                        .entry(tex.id)
                        .or_insert_with(|| {
                            make_texture_bind_group(
                                device,
                                queue,
                                &self.texture_layout,
                                &self.sampler,
                                tex.width,
                                tex.height,
                                &tex.rgba,
                            )
                        })
                        .clone(),
                    None => self.default_texture.clone(),
                };
                TexturedBatch {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("od_textured"),
                        contents: bytemuck::cast_slice(&textured.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("od_textured_indices"),
                        contents: bytemuck::cast_slice(&textured.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    count: textured.indices.len() as u32,
                    bind_group,
                }
            })
            .collect();
    }

    /// Draw into a pass whose viewport is already set to the 3D view.
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &self.default_texture, &[]);

        pass.set_pipeline(&self.lit);
        if let Some((_, avatar)) = &self.avatar {
            if avatar.count > 0 {
                pass.set_vertex_buffer(0, avatar.vertices.slice(..));
                pass.set_index_buffer(avatar.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..avatar.count, 0, 0..1);
            }
        }
        if let (Some(vertices), Some(indices)) =
            (self.surfaces.slice(), self.surface_indices.slice())
        {
            pass.set_vertex_buffer(0, vertices);
            pass.set_index_buffer(indices, wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.surface_indices.count, 0, 0..1);
        }

        for batch in &self.textured {
            pass.set_bind_group(1, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            pass.set_index_buffer(batch.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.count, 0, 0..1);
        }

        // See-through surfaces after the solids, so the solids show
        // through them; they don't write depth, so they hide nothing.
        if let (Some(vertices), Some(indices)) = (
            self.translucent_vertices.slice(),
            self.translucent_indices.slice(),
        ) {
            pass.set_pipeline(&self.translucent);
            pass.set_bind_group(1, &self.default_texture, &[]);
            pass.set_vertex_buffer(0, vertices);
            pass.set_index_buffer(indices, wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.translucent_indices.count, 0, 0..1);
        }

        // After the solids, so depth testing hides lines behind them.
        if let Some(lines) = self.line_vertices.slice() {
            pass.set_pipeline(&self.lines);
            pass.set_vertex_buffer(0, lines);
            pass.draw(0..self.line_vertices.count, 0..1);
        }
        if let Some(shadow) = self.shadow_vertices.slice() {
            pass.set_pipeline(&self.shadow);
            pass.set_vertex_buffer(0, shadow);
            pass.draw(0..self.shadow_vertices.count, 0..1);
        }
    }
}

/// Draw `frame` into an offscreen image and read it back as RGBA8 rows,
/// for tests and snapshots. `targets.color` must be a 4-byte RGBA format.
pub fn render_to_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    targets: Targets,
    frame: &Frame,
    [width, height]: [u32; 2],
    clear: wgpu::Color,
) -> Vec<u8> {
    let texture = |format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("od_offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    use wgpu::TextureUsages as U;
    let samples = targets.samples.max(1);
    let resolved = texture(targets.color, 1, U::RENDER_ATTACHMENT | U::COPY_SRC);
    let multisampled = (samples > 1).then(|| texture(targets.color, samples, U::RENDER_ATTACHMENT));
    let depth = targets
        .depth
        .map(|format| texture(format, samples, U::RENDER_ATTACHMENT));

    renderer.prepare(device, queue, frame);
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let resolved_view = resolved.create_view(&Default::default());
        let ms_view = multisampled
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let depth_view = depth.as_ref().map(|t| t.create_view(&Default::default()));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("od_offscreen"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ms_view.as_ref().unwrap_or(&resolved_view),
                depth_slice: None,
                resolve_target: ms_view.as_ref().map(|_| &resolved_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth_view.as_ref().map(|view| {
                wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.paint(&mut pass);
    }

    // Rows are padded to the copy alignment.
    let row = width * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("od_readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        resolved.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let Ok(data) = readback.slice(..).get_mapped_range() else {
        return Vec::new();
    };
    data.chunks_exact(padded as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Lit,
    /// Shaded like `Lit`, but alpha-blended and hiding nothing.
    Translucent,
    Lines,
    Shadow,
}

/// Upload an RGBA8 image and wrap it (plus `sampler`) in a bind group
/// matching `layout`, group 1's shape in `scene.wgsl`.
fn make_texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::BindGroup {
    let size = wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("od_fabric_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size.width),
            rows_per_image: Some(size.height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("od_fabric_texture"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> naga::Module {
        naga::front::wgsl::parse_str(SHADER).expect("WGSL parses")
    }

    #[test]
    fn shader_is_valid() {
        let module = module();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("WGSL validates");
        for entry in ["vs_lit", "fs_lit", "vs_flat", "fs_flat"] {
            assert!(
                module.entry_points.iter().any(|e| e.name == entry),
                "missing {entry}"
            );
        }
    }

    /// The Rust uniform block and the WGSL one must have the same layout.
    #[test]
    fn globals_layout_matches_shader() {
        let module = module();
        let (_, var) = module
            .global_variables
            .iter()
            .find(|(_, v)| v.name.as_deref() == Some("g"))
            .unwrap();
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).unwrap();
        assert_eq!(layouter[var.ty].size as usize, size_of::<Globals>());
        let naga::TypeInner::Struct { members, .. } = &module.types[var.ty].inner else {
            panic!("Globals is a struct");
        };
        let names: Vec<_> = members.iter().map(|m| m.name.clone().unwrap()).collect();
        assert_eq!(
            names,
            ["view_proj", "key", "fill", "sky", "ground", "eye", "params"]
        );
    }

    /// Vertex strides and attribute offsets match the Rust structs.
    #[test]
    fn vertex_layouts_match_structs() {
        assert_eq!(size_of::<LitVertex>(), 48);
        assert_eq!(LIT_ATTRIBUTES[1].offset, 12);
        assert_eq!(LIT_ATTRIBUTES[2].offset, 24);
        assert_eq!(LIT_ATTRIBUTES[3].offset, 40);
        assert_eq!(size_of::<FlatVertex>(), 28);
        assert_eq!(FLAT_ATTRIBUTES[1].offset, 12);
    }

    /// Renders a frame offscreen when this machine has any GPU adapter
    /// (including a software one); skips otherwise.
    #[test]
    fn renders_offscreen_when_a_gpu_is_available() {
        use crate::avatar::{avatar_mesh, AvatarKind};
        use crate::camera::{OrbitCamera, ViewPreset};
        use crate::scene::AvatarDraw;
        use std::sync::Arc;

        let Some((device, queue)) = headless_device() else {
            eprintln!("no GPU adapter; skipping the offscreen render");
            return;
        };
        const SIZE: u32 = 64;
        let targets = Targets {
            color: wgpu::TextureFormat::Rgba8Unorm,
            depth: Some(crate::DEPTH_FORMAT),
            samples: u32::from(crate::MSAA_SAMPLES),
        };
        let mut renderer = Renderer::new(&device, &queue, targets);
        let kind = AvatarKind::Female;
        let mesh = Arc::new(avatar_mesh(kind, kind.default_measurements()));
        let mut camera = OrbitCamera::default();
        camera.set_view(ViewPreset::Front);
        camera.frame(mesh.bounds().unwrap(), 1.0);
        let mut frame = Frame::new(Globals::new(&camera, 1.0, renderer.encode_srgb()));
        frame.avatar = Some(AvatarDraw {
            key: 1,
            mesh,
            color: [0.8, 0.2, 0.1, 1.0],
        });
        let pixels = render_to_rgba(
            &device,
            &queue,
            &mut renderer,
            targets,
            &frame,
            [SIZE; 2],
            wgpu::Color::BLACK,
        );

        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        // The torso fills the middle, lit and reddish; the corner stays clear.
        let [r, g, b] = at(SIZE / 2, SIZE / 2);
        assert!(r > 60 && r > g + 20 && r > b + 20, "centre {r} {g} {b}");
        assert_eq!(at(1, 1), [0, 0, 0]);
    }

    fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        eprintln!("rendering on {:?}", adapter.get_info());
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }
}
