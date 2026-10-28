//! Render the garment view offscreen to a PNG, to check the renderer on a
//! machine without a window (any wgpu adapter, software ones included).
//!
//! cargo run -p od_render --example snapshot -- out.png [female|male|kid] [dark|light] [front|back|left|right|top] [avatar]

use std::sync::Arc;

use od_render::avatar::{avatar_mesh, AvatarKind};
use od_render::camera::{OrbitCamera, ViewPreset};
use od_render::glam::Vec2;
use od_render::gpu::{render_to_rgba, Renderer, Targets};
use od_render::scene::{linear, AvatarDraw, GridColors};
use od_render::view::{compose_frame, place_patterns, scene_bounds, FrameInput, Theme};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map_or("snapshot.png", String::as_str);
    let kind = match args.get(1).map(String::as_str) {
        Some("male") => AvatarKind::Male,
        Some("kid") => AvatarKind::Kid,
        _ => AvatarKind::Female,
    };
    let light = args.get(2).is_some_and(|a| a == "light");
    let view = args.get(3).and_then(|v| {
        ViewPreset::ALL
            .into_iter()
            .find(|p| p.label().eq_ignore_ascii_case(v))
    });
    let avatar_only = args.iter().any(|a| a == "avatar");

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("no wgpu adapter");
    println!("adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();

    // The app's theme tokens (theme.rs), as sRGB bytes.
    let rgb = |c: u32| linear([(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    let (bg, theme) = if light {
        (
            0xF7F7F8,
            Theme {
                avatar: rgb(0x8A7A6C),
                pattern: rgb(0x3B6EB0),
                selected: rgb(0xD97706),
                outline: rgb(0x2C558C),
                grid: GridColors {
                    minor: rgb(0xE8E8EB),
                    major: rgb(0xD4D4D8),
                    axis: rgb(0xA1A1AA),
                },
                shadow: [0.0, 0.0, 0.0, 0.18],
            },
        )
    } else {
        (
            0x1E1E21,
            Theme {
                avatar: rgb(0x9C8F84),
                pattern: rgb(0x7A9CC6),
                selected: rgb(0xFFB03B),
                outline: rgb(0xA9C4E6),
                grid: GridColors {
                    minor: rgb(0x2A2A2E),
                    major: rgb(0x38383D),
                    axis: rgb(0x55555C),
                },
                shadow: [0.0, 0.0, 0.0, 0.35],
            },
        )
    };

    let mesh = Arc::new(avatar_mesh(kind, kind.default_measurements()));
    let body = mesh.bounds().unwrap();
    let blocks: Vec<(u32, Vec<Vec2>)> = vec![
        (
            1,
            vec![
                (0., 40.),
                (150., 0.),
                (350., 0.),
                (500., 40.),
                (480., 620.),
                (20., 620.),
            ],
        ),
        (2, vec![(0., 20.), (500., 20.), (480., 620.), (20., 620.)]),
        (
            3,
            vec![
                (60., 550.),
                (0., 180.),
                (250., 0.),
                (500., 180.),
                (440., 550.),
            ],
        ),
        (4, vec![(100., 0.), (400., 0.), (500., 650.), (0., 650.)]),
    ]
    .into_iter()
    .map(|(k, pts)| (k, pts.into_iter().map(|(x, y)| Vec2::new(x, y)).collect()))
    .collect();
    let placed = place_patterns(body, blocks.into_iter());

    let size = [960u32, 720];
    let aspect = size[0] as f32 / size[1] as f32;
    let placed = if avatar_only { Vec::new() } else { placed };
    let mut camera = OrbitCamera::default();
    if let Some(view) = view {
        camera.set_view(view);
    }
    camera.frame(scene_bounds(body, &placed), aspect);

    let targets = Targets {
        color: wgpu::TextureFormat::Rgba8Unorm,
        depth: Some(od_render::DEPTH_FORMAT),
        samples: u32::from(od_render::MSAA_SAMPLES),
    };
    let mut renderer = Renderer::new(&device, &queue, targets);
    let frame = compose_frame(FrameInput {
        camera: &camera,
        aspect,
        encode_srgb: renderer.encode_srgb(),
        avatar: Some(AvatarDraw {
            key: 1,
            mesh,
            color: theme.avatar,
        }),
        avatar_bounds: body,
        patterns: &placed,
        selected: &[1],
        theme: &theme,
        pattern_colors: None,
        pattern_materials: None,
        display: Default::default(),
    });
    // The app paints this background with egui before the 3D callback runs.
    let channel = |shift: u32| f64::from((bg >> shift) as u8) / 255.0;
    let clear = wgpu::Color {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    };
    let pixels = render_to_rgba(&device, &queue, &mut renderer, targets, &frame, size, clear);
    image::save_buffer(
        out,
        &pixels,
        size[0],
        size[1],
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    println!("wrote {out}");
}
