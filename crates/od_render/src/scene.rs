//! What to draw this frame, in GPU-ready form: vertex types, the uniform
//! block with the light rig, the floor grid and the contact shadow. All of
//! it is plain data, so it can be built and tested without a GPU.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

use crate::camera::OrbitCamera;
use crate::mesh::Mesh;

/// Linear-light RGBA.
pub type Color = [f32; 4];

/// A vertex of a shaded surface.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LitVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: Color,
}

/// A vertex of an unlit line or overlay.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct FlatVertex {
    pub position: [f32; 3],
    pub color: Color,
}

impl FlatVertex {
    fn new(p: Vec3, color: Color) -> Self {
        Self {
            position: p.to_array(),
            color,
        }
    }
}

/// The uniform block; must match `Globals` in `scene.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    /// xyz: unit direction towards the key light; w: intensity.
    pub key: [f32; 4],
    /// The fill light, likewise.
    pub fill: [f32; 4],
    /// Ambient light from above and below.
    pub sky: [f32; 4],
    pub ground: [f32; 4],
    pub eye: [f32; 4],
    /// x: 1 when the shader must encode its output to sRGB itself.
    pub params: [f32; 4],
}

impl Globals {
    /// A key light above and to the left of the viewer and a dimmer fill
    /// from the right. The rig turns with the camera, so whatever side you
    /// look at is lit.
    pub fn new(camera: &OrbitCamera, aspect: f32, encode_srgb: bool) -> Self {
        let (right, up) = camera.basis();
        let back = camera.back();
        let key = (-right * 0.55 + up * 0.75 + back * 0.6).normalize();
        let fill = (right * 0.8 + up * 0.1 + back * 0.45).normalize();
        Self {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            key: key.extend(0.85).to_array(),
            fill: fill.extend(0.35).to_array(),
            sky: [0.24, 0.24, 0.26, 0.0],
            ground: [0.10, 0.09, 0.09, 0.0],
            eye: camera.eye().extend(1.0).to_array(),
            params: [f32::from(u8::from(encode_srgb)), 0.0, 0.0, 0.0],
        }
    }
}

/// sRGB byte to linear light.
pub fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// An sRGB colour (straight alpha) as linear-light RGBA.
pub fn linear([r, g, b, a]: [u8; 4]) -> Color {
    [
        srgb_to_linear(r),
        srgb_to_linear(g),
        srgb_to_linear(b),
        f32::from(a) / 255.0,
    ]
}

/// The avatar changes rarely, so the renderer keeps its buffers until `key`
/// changes.
#[derive(Debug, Clone)]
pub struct AvatarDraw {
    pub key: u64,
    pub mesh: Arc<Mesh>,
    pub color: Color,
}

/// Everything for one frame of the 3D view.
#[derive(Debug, Clone)]
pub struct Frame {
    pub globals: Globals,
    pub avatar: Option<AvatarDraw>,
    /// Opaque shaded triangles besides the avatar (the patterns).
    pub surfaces: Vec<LitVertex>,
    pub surface_indices: Vec<u32>,
    /// Line list: grid and outlines.
    pub lines: Vec<FlatVertex>,
    /// Alpha-blended triangles on the floor.
    pub shadow: Vec<FlatVertex>,
}

impl Frame {
    pub fn new(globals: Globals) -> Self {
        Self {
            globals,
            avatar: None,
            surfaces: Vec::new(),
            surface_indices: Vec::new(),
            lines: Vec::new(),
            shadow: Vec::new(),
        }
    }

    pub fn add_surface(&mut self, mesh: &Mesh, color: Color) {
        let base = self.surfaces.len() as u32;
        self.surfaces.extend(lit_vertices(mesh, color));
        self.surface_indices
            .extend(mesh.indices.iter().map(|i| i + base));
    }

    /// Like [`Self::add_surface`], but with one color per vertex (e.g. a
    /// strain heatmap) instead of a single fill. `colors` must be the
    /// same length as `mesh.positions`.
    pub fn add_surface_varying(&mut self, mesh: &Mesh, colors: &[Color]) {
        let base = self.surfaces.len() as u32;
        self.surfaces
            .extend(mesh.positions.iter().zip(&mesh.normals).zip(colors).map(
                |((p, n), &color)| LitVertex {
                    position: p.to_array(),
                    normal: n.to_array(),
                    color,
                },
            ));
        self.surface_indices
            .extend(mesh.indices.iter().map(|i| i + base));
    }

    /// A closed outline, drawn just off both faces of a flat piece so the
    /// surface never hides it.
    pub fn add_outline(&mut self, outline: &[Vec3], normal: Vec3, color: Color) {
        for side in [1.0, -1.0] {
            let offset = normal * (0.0015 * side);
            for (a, b) in outline.iter().zip(outline.iter().cycle().skip(1)) {
                self.lines.push(FlatVertex::new(*a + offset, color));
                self.lines.push(FlatVertex::new(*b + offset, color));
            }
        }
    }
}

pub fn lit_vertices(mesh: &Mesh, color: Color) -> Vec<LitVertex> {
    mesh.positions
        .iter()
        .zip(&mesh.normals)
        .map(|(p, n)| LitVertex {
            position: p.to_array(),
            normal: n.to_array(),
            color,
        })
        .collect()
}

pub struct GridColors {
    pub minor: Color,
    pub major: Color,
    pub axis: Color,
}

/// Floor grid on y = 0 as a line list: a line every `step` metres out to
/// `half_extent`, every `major_every`-th one stronger, and the X and Z axes
/// in the axis colour.
pub fn floor_grid(
    half_extent: f32,
    step: f32,
    major_every: i32,
    colors: &GridColors,
) -> Vec<FlatVertex> {
    let n = (half_extent / step).round() as i32;
    let mut lines = Vec::with_capacity((2 * n as usize + 1) * 4);
    for i in -n..=n {
        let t = i as f32 * step;
        let color = if i == 0 {
            colors.axis
        } else if i % major_every == 0 {
            colors.major
        } else {
            colors.minor
        };
        lines.push(FlatVertex::new(Vec3::new(t, 0.0, -half_extent), color));
        lines.push(FlatVertex::new(Vec3::new(t, 0.0, half_extent), color));
        lines.push(FlatVertex::new(Vec3::new(-half_extent, 0.0, t), color));
        lines.push(FlatVertex::new(Vec3::new(half_extent, 0.0, t), color));
    }
    lines
}

/// A soft dark ellipse on the floor under the avatar: a cheap stand-in for
/// a real shadow. Triangle list, opaque at the centre, clear at the rim.
pub fn blob_shadow(center: Vec2, radii: Vec2, color: Color) -> Vec<FlatVertex> {
    const SEGMENTS: usize = 40;
    const RINGS: usize = 4;
    let y = 0.002; // Just above the grid.
    let point = |ring: usize, seg: usize| {
        let r = ring as f32 / RINGS as f32;
        let a = seg as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let p = center + Vec2::new(a.cos(), a.sin()) * radii * r;
        // Smooth falloff towards the rim.
        let alpha = color[3] * (1.0 - r * r).powi(2);
        FlatVertex::new(
            Vec3::new(p.x, y, p.y),
            [color[0], color[1], color[2], alpha],
        )
    };
    let mut tris = Vec::with_capacity(SEGMENTS * (RINGS * 2 - 1) * 3);
    for s in 0..SEGMENTS {
        tris.extend([point(0, 0), point(1, s + 1), point(1, s)]);
        for r in 1..RINGS {
            let (a, b) = (point(r, s), point(r, s + 1));
            let (c, d) = (point(r + 1, s), point(r + 1, s + 1));
            tris.extend([a, b, c, b, d, c]);
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_conversion() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert_eq!(srgb_to_linear(255), 1.0);
        assert!((srgb_to_linear(188) - 0.5).abs() < 0.01);
        assert_eq!(linear([255, 0, 0, 51])[3], 0.2);
    }

    #[test]
    fn lights_are_unit_and_face_the_viewer() {
        let cam = OrbitCamera::default();
        let g = Globals::new(&cam, 1.5, true);
        for light in [g.key, g.fill] {
            let dir = Vec3::new(light[0], light[1], light[2]);
            assert!((dir.length() - 1.0).abs() < 1e-5);
            assert!(dir.dot(cam.back()) > 0.0, "lights the visible side");
        }
        assert!(g.key[3] > g.fill[3], "key brighter than fill");
        assert_eq!(g.params[0], 1.0);
        assert_eq!(Globals::new(&cam, 1.5, false).params[0], 0.0);
    }

    #[test]
    fn grid_covers_the_floor() {
        let colors = GridColors {
            minor: [0.1; 4],
            major: [0.2; 4],
            axis: [0.3; 4],
        };
        let grid = floor_grid(2.0, 0.1, 5, &colors);
        assert_eq!(grid.len(), 41 * 4);
        assert!(grid.iter().all(|v| v.position[1] == 0.0));
        assert!(grid.iter().all(|v| v.position[0].abs() <= 2.0 + 1e-5));
        assert_eq!(grid.iter().filter(|v| v.color == colors.axis).count(), 4);
        assert_eq!(
            grid.iter().filter(|v| v.color == colors.major).count(),
            8 * 4
        );
    }

    #[test]
    fn shadow_fades_to_the_rim() {
        let tris = blob_shadow(Vec2::ZERO, Vec2::new(0.4, 0.3), [0.0, 0.0, 0.0, 0.3]);
        assert_eq!(tris.len() % 3, 0);
        let max = tris.iter().map(|v| v.color[3]).fold(0.0, f32::max);
        assert!((max - 0.3).abs() < 1e-6);
        for v in &tris {
            let r = Vec2::new(v.position[0] / 0.4, v.position[2] / 0.3).length();
            assert!(r <= 1.0 + 1e-5);
            if r > 0.999 {
                assert!(v.color[3] < 1e-6);
            }
        }
    }

    #[test]
    fn frame_offsets_surface_indices() {
        let mesh = crate::mesh::tests::cube();
        let mut frame = Frame::new(Globals::new(&OrbitCamera::default(), 1.0, false));
        frame.add_surface(&mesh, [1.0; 4]);
        frame.add_surface(&mesh, [0.5; 4]);
        assert_eq!(frame.surfaces.len(), 16);
        assert_eq!(*frame.surface_indices.iter().max().unwrap(), 15);
        frame.add_outline(&[Vec3::ZERO, Vec3::X, Vec3::Y], Vec3::Z, [1.0; 4]);
        assert_eq!(frame.lines.len(), 12, "3 edges, both sides");
    }

    #[test]
    fn surface_varying_colors_each_vertex_separately() {
        let mesh = crate::mesh::tests::cube();
        let colors: Vec<Color> = (0..mesh.positions.len()).map(|i| [i as f32; 4]).collect();
        let mut frame = Frame::new(Globals::new(&OrbitCamera::default(), 1.0, false));
        frame.add_surface_varying(&mesh, &colors);
        assert_eq!(frame.surfaces.len(), mesh.positions.len());
        for (v, &c) in frame.surfaces.iter().zip(&colors) {
            assert_eq!(v.color, c);
        }
        assert_eq!(frame.surface_indices, mesh.indices);
    }
}
