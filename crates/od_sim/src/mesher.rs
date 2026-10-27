//! Meshes a flat pattern for simulation: the outline resampled so no edge
//! is longer than the particle distance, internal lines kept as mesh edges
//! (so a fold or dart line later bends along them), and the inside filled
//! with roughly equilateral triangles of about that size. Without interior
//! points a pattern would be a hollow fan of long, thin triangles that
//! can't drape.

use od_core::document::FlatPoint;
use od_core::geometry::{self, Vec2};
use spade::handles::FixedVertexHandle;
use spade::{
    AngleLimit, ConstrainedDelaunayTriangulation, Point2, RefinementParameters, Triangulation,
};
use std::collections::HashMap;

/// One mesh vertex, in the pattern's own mm coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    pub pos: Vec2,
    /// Where on the outline this vertex sits (see [`FlatPoint::param`]), or
    /// `None` for a vertex inside the pattern.
    pub param: Option<f32>,
}

/// A triangulated pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternMesh {
    pub vertices: Vec<MeshVertex>,
    pub faces: Vec<[u32; 3]>,
    /// The outline's vertices, in outline order.
    pub boundary: Vec<u32>,
}

impl PatternMesh {
    /// Which face contains `p`, and its barycentric weights there. A point
    /// outside every face (just off the outline, say) gets the nearest
    /// face, with weights clamped onto it.
    pub fn locate(&self, p: Vec2) -> Option<(usize, [f32; 3])> {
        let mut best: Option<(f32, usize, [f32; 3])> = None;
        for (i, &[a, b, c]) in self.faces.iter().enumerate() {
            let tri = [a, b, c].map(|v| self.vertices[v as usize].pos);
            let (w, dist) = barycentric_clamped(tri, p);
            if dist <= 1e-4 {
                return Some((i, w));
            }
            if best.is_none_or(|(d, _, _)| dist < d) {
                best = Some((dist, i, w));
            }
        }
        best.map(|(_, i, w)| (i, w))
    }

    /// Area of each vertex's share of the surface (a third of every face
    /// touching it), in mm².
    pub fn vertex_areas(&self) -> Vec<f32> {
        let mut areas = vec![0.0; self.vertices.len()];
        for &[a, b, c] in &self.faces {
            let (pa, pb, pc) = (
                self.vertices[a as usize].pos,
                self.vertices[b as usize].pos,
                self.vertices[c as usize].pos,
            );
            let third = ((pb - pa).cross(pc - pa)).abs() / 6.0;
            for v in [a, b, c] {
                areas[v as usize] += third;
            }
        }
        areas
    }
}

/// Barycentric weights of `p` in `tri`, clamped onto the triangle, and how
/// far `p` is from the clamped point (0 inside).
pub fn barycentric_clamped(tri: [Vec2; 3], p: Vec2) -> ([f32; 3], f32) {
    let [a, b, c] = tri;
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let den = v0.cross(v1);
    if den.abs() > 1e-12 {
        let v = v2.cross(v1) / den;
        let w = v0.cross(v2) / den;
        let u = 1.0 - v - w;
        if u >= -1e-6 && v >= -1e-6 && w >= -1e-6 {
            return ([u.max(0.0), v.max(0.0), w.max(0.0)], 0.0);
        }
    }
    // Outside (or degenerate): the closest point on one of the edges.
    let mut best = ([1.0, 0.0, 0.0], f32::INFINITY);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        let (q, t) = geometry::closest_on_segment(tri[i], tri[j], p);
        let d = q.distance(p);
        if d < best.1 {
            let mut w = [0.0; 3];
            w[i] = 1.0 - t;
            w[j] = t;
            best = (w, d);
        }
    }
    best
}

/// The outline with extra points added so no edge is longer than
/// `spacing`, each with its outline parameter. `outline_len` is the number
/// of outline points the parameters count (they wrap at it).
pub fn resample_outline(flat: &[FlatPoint], outline_len: usize, spacing: f32) -> Vec<FlatPoint> {
    let n = flat.len();
    let wrap = outline_len.max(1) as f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (flat[i], flat[(i + 1) % n]);
        let mut b_param = b.param;
        if b_param <= a.param {
            b_param += wrap;
        }
        out.push(a);
        let steps = (a.pos.distance(b.pos) / spacing).ceil() as usize;
        for k in 1..steps {
            let t = k as f32 / steps as f32;
            out.push(FlatPoint {
                pos: a.pos.lerp(b.pos, t),
                param: (a.param + (b_param - a.param) * t).rem_euclid(wrap),
            });
        }
    }
    out
}

/// An open polyline with extra points so no segment is longer than `spacing`.
fn resample_open(points: &[Vec2], spacing: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    for w in points.windows(2) {
        out.push(w[0]);
        let steps = (w[0].distance(w[1]) / spacing).ceil() as usize;
        for k in 1..steps {
            out.push(w[0].lerp(w[1], k as f32 / steps as f32));
        }
    }
    out.extend(points.last());
    out
}

type Cdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

fn point(p: Vec2) -> Point2<f64> {
    Point2::new(f64::from(p.x), f64::from(p.y))
}

/// Mesh a pattern from its flattened outline (see
/// [`od_core::document::Pattern::flatten`]) and internal lines, aiming for
/// triangle edges about `spacing` mm long and adding at most
/// `max_interior` points inside. `None` if the outline can't be meshed (too
/// few points, no area).
pub fn mesh_pattern(
    flat: &[FlatPoint],
    outline_len: usize,
    internal_lines: &[Vec<Vec2>],
    spacing: f32,
    max_interior: usize,
) -> Option<PatternMesh> {
    let polygon: Vec<Vec2> = flat.iter().map(|f| f.pos).collect();
    if flat.len() < 3 || geometry::area(&polygon) <= 1e-3 {
        return None;
    }
    let ring = resample_outline(flat, outline_len, spacing);

    let mut cdt = Cdt::new();
    let mut params: HashMap<FixedVertexHandle, f32> = HashMap::new();
    let mut ring_handles: Vec<FixedVertexHandle> = Vec::with_capacity(ring.len());
    for f in &ring {
        let h = cdt.insert(point(f.pos)).ok()?;
        // Two outline points at the same spot (a degenerate edge) share a
        // vertex; it keeps the first parameter.
        if ring_handles.last() != Some(&h) && ring_handles.first() != Some(&h) {
            params.entry(h).or_insert(f.param);
            ring_handles.push(h);
        }
    }
    if ring_handles.len() < 3 {
        return None;
    }
    for i in 0..ring_handles.len() {
        let (a, b) = (ring_handles[i], ring_handles[(i + 1) % ring_handles.len()]);
        // A self-crossing outline can't be fully constrained; mesh what we can.
        cdt.try_add_constraint(a, b);
    }
    // An equilateral triangle with sides `spacing` has area √3/4·s²; allow
    // a little more so refinement stops near that size, not well below it.
    // Only the outline constrains this pass: spade can't tell inside from
    // outside once an open internal line is also a constraint.
    let s = f64::from(spacing);
    cdt.refine(
        RefinementParameters::<f64>::new()
            .exclude_outer_faces(true)
            .keep_constraint_edges()
            .with_angle_limit(AngleLimit::from_deg(25.0))
            .with_max_allowed_area(0.5 * s * s)
            .with_min_required_area(0.02 * s * s)
            .with_max_additional_vertices(max_interior),
    );
    // Internal lines go in afterwards, as edges of the refined mesh.
    for line in internal_lines {
        let mut prev: Option<FixedVertexHandle> = None;
        for p in resample_open(line, spacing) {
            if !geometry::contains(&polygon, p) {
                prev = None;
                continue;
            }
            let Ok(h) = cdt.insert(point(p)) else {
                prev = None;
                continue;
            };
            if let Some(a) = prev.filter(|&a| a != h) {
                cdt.try_add_constraint(a, h);
            }
            prev = Some(h);
        }
    }

    let mut index: HashMap<FixedVertexHandle, u32> = HashMap::new();
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    let mut vertex_of = |h: FixedVertexHandle, cdt: &Cdt| -> u32 {
        *index.entry(h).or_insert_with(|| {
            let p = cdt.vertex(h).position();
            vertices.push(MeshVertex {
                pos: Vec2::new(p.x as f32, p.y as f32),
                param: params.get(&h).copied(),
            });
            (vertices.len() - 1) as u32
        })
    };
    // Outline vertices first, in order, so a pattern's boundary is a
    // prefix of its vertices.
    let boundary: Vec<u32> = ring_handles.iter().map(|&h| vertex_of(h, &cdt)).collect();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let centroid = {
            let [pa, pb, pc] = [a, b, c].map(|v| v.position());
            Vec2::new(
                ((pa.x + pb.x + pc.x) / 3.0) as f32,
                ((pa.y + pb.y + pc.y) / 3.0) as f32,
            )
        };
        // Faces between the outline and its convex hull are outside.
        if !geometry::contains(&polygon, centroid) {
            continue;
        }
        faces.push([a.fix(), b.fix(), c.fix()].map(|h| vertex_of(h, &cdt)));
    }
    if faces.is_empty() {
        return None;
    }
    Some(PatternMesh {
        vertices,
        faces,
        boundary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f32, h: f32) -> Vec<FlatPoint> {
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| FlatPoint {
                pos: Vec2::new(x, y),
                param: i as f32,
            })
            .collect()
    }

    fn edges(mesh: &PatternMesh) -> Vec<f32> {
        mesh.faces
            .iter()
            .flat_map(|&[a, b, c]| [(a, b), (b, c), (c, a)])
            .map(|(a, b)| {
                mesh.vertices[a as usize]
                    .pos
                    .distance(mesh.vertices[b as usize].pos)
            })
            .collect()
    }

    #[test]
    fn a_rectangle_fills_with_points_about_the_spacing_apart() {
        let mesh = mesh_pattern(&rect(200.0, 300.0), 4, &[], 20.0, 10_000).unwrap();
        let interior = mesh.vertices.iter().filter(|v| v.param.is_none()).count();
        assert!(interior > 60, "{interior} interior points");
        assert!(
            mesh.vertices.len() > 150,
            "{} vertices",
            mesh.vertices.len()
        );
        let longest = edges(&mesh).into_iter().fold(0.0, f32::max);
        assert!(longest <= 1.6 * 20.0, "longest edge {longest}");
        // The faces cover the rectangle exactly.
        let area: f32 = mesh.vertex_areas().iter().sum();
        assert!((area - 200.0 * 300.0).abs() < 1.0, "{area}");
    }

    #[test]
    fn outline_vertices_come_first_in_order_with_their_parameters() {
        let mesh = mesh_pattern(&rect(200.0, 300.0), 4, &[], 20.0, 10_000).unwrap();
        // 1000 mm of outline at 20 mm: 50 points.
        assert_eq!(mesh.boundary.len(), 50);
        assert_eq!(mesh.boundary, (0..50).collect::<Vec<u32>>());
        let params: Vec<f32> = mesh
            .boundary
            .iter()
            .map(|&i| mesh.vertices[i as usize].param.unwrap())
            .collect();
        assert!(params.windows(2).all(|w| w[1] > w[0]), "{params:?}");
        assert_eq!(params[0], 0.0);
        // Corner 1 at (200, 0) is ten points along.
        assert_eq!(mesh.vertices[10].pos, Vec2::new(200.0, 0.0));
        assert_eq!(params[10], 1.0);
        assert!((params[5] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn a_large_spacing_keeps_just_the_outline() {
        let mesh = mesh_pattern(&rect(200.0, 300.0), 4, &[], 1000.0, 10_000).unwrap();
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.faces.len(), 2);
    }

    #[test]
    fn internal_lines_become_mesh_edges() {
        let line = vec![Vec2::new(100.0, 40.0), Vec2::new(100.0, 260.0)];
        let mesh = mesh_pattern(&rect(200.0, 300.0), 4, &[line], 20.0, 10_000).unwrap();
        let on_line: Vec<u32> = (0..mesh.vertices.len() as u32)
            .filter(|&i| {
                let p = mesh.vertices[i as usize].pos;
                (p.x - 100.0).abs() < 1e-3 && p.y >= 40.0 - 1e-3 && p.y <= 260.0 + 1e-3
            })
            .collect();
        // 220 mm at 20 mm: 12 points, each joined to the next by an edge.
        assert_eq!(on_line.len(), 12);
        let has_edge = |a: u32, b: u32| mesh.faces.iter().any(|f| f.contains(&a) && f.contains(&b));
        let mut sorted = on_line.clone();
        sorted.sort_by(|&a, &b| {
            mesh.vertices[a as usize]
                .pos
                .y
                .total_cmp(&mesh.vertices[b as usize].pos.y)
        });
        assert!(sorted.windows(2).all(|w| has_edge(w[0], w[1])));
    }

    #[test]
    fn the_interior_point_budget_is_respected() {
        let mesh = mesh_pattern(&rect(1000.0, 1000.0), 4, &[], 10.0, 100).unwrap();
        let interior = mesh.vertices.iter().filter(|v| v.param.is_none()).count();
        assert!(interior <= 100, "{interior}");
    }

    #[test]
    fn locate_finds_the_face_and_weights_of_a_point() {
        let mesh = mesh_pattern(&rect(200.0, 300.0), 4, &[], 20.0, 10_000).unwrap();
        let p = Vec2::new(73.0, 151.0);
        let (face, w) = mesh.locate(p).unwrap();
        let back = mesh.faces[face]
            .iter()
            .zip(w)
            .fold(Vec2::ZERO, |acc, (&v, w)| {
                acc + mesh.vertices[v as usize].pos * w
            });
        assert!(back.distance(p) < 1e-3, "{back:?}");
        // Off the pattern: clamped onto the nearest face.
        let (_, w) = mesh.locate(Vec2::new(-5.0, 150.0)).unwrap();
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_degenerate_outline_has_no_mesh() {
        let flat = vec![
            FlatPoint {
                pos: Vec2::ZERO,
                param: 0.0,
            },
            FlatPoint {
                pos: Vec2::new(10.0, 0.0),
                param: 1.0,
            },
        ];
        assert!(mesh_pattern(&flat, 2, &[], 20.0, 100).is_none());
    }
}
