//! Measuring the avatar: cutting its mesh with a plane, the girth around
//! the body at a height (as a tape measure takes it, bridging hollows),
//! and a tape laid over the surface between points.
//!
//! All in metres, like the meshes.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::mesh::Mesh;

/// One connected piece of a mesh's cross-section: a polyline in the
/// cutting plane, closed (the last point joins the first) where it goes
/// all the way round a closed part.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub points: Vec<Vec3>,
    pub closed: bool,
}

impl Section {
    /// The polyline's length, including the closing segment if closed.
    pub fn length(&self) -> f32 {
        let open: f32 = self.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        match (self.closed, self.points.first(), self.points.last()) {
            (true, Some(first), Some(last)) => open + last.distance(*first),
            _ => open,
        }
    }

    fn segment_count(&self) -> usize {
        match self.points.len() {
            0 | 1 => 0,
            n if self.closed => n,
            n => n - 1,
        }
    }

    fn segment(&self, i: usize) -> (Vec3, Vec3) {
        let n = self.points.len();
        (self.points[i], self.points[(i + 1) % n])
    }

    /// Where on the section `p` is nearest: the segment, how far along
    /// it (0..1), and the distance from `p`.
    fn locate(&self, p: Vec3) -> Option<Spot> {
        (0..self.segment_count())
            .map(|i| {
                let (a, b) = self.segment(i);
                let t = closest_on_segment(a, b, p);
                Spot {
                    segment: i,
                    t,
                    gap: a.lerp(b, t).distance(p),
                }
            })
            .min_by(|a, b| a.gap.total_cmp(&b.gap))
    }

    /// Distance along the section from its first point to `spot`.
    fn arc_at(&self, spot: Spot) -> f32 {
        let before: f32 = (0..spot.segment)
            .map(|i| {
                let (a, b) = self.segment(i);
                a.distance(b)
            })
            .sum();
        let (a, b) = self.segment(spot.segment);
        before + a.distance(b) * spot.t
    }

    /// The section's points from `from` to `to`, going forwards (towards
    /// later points) or backwards, starting and ending exactly there.
    fn run(&self, from: Spot, to: Spot, forward: bool) -> Vec<Vec3> {
        let at = |s: Spot| {
            let (a, b) = self.segment(s.segment);
            a.lerp(b, s.t)
        };
        let n = self.points.len();
        let mut out = vec![at(from)];
        if forward {
            let same = from.segment == to.segment && from.t <= to.t;
            if !same {
                // The end of each segment from `from`'s up to `to`'s.
                let mut i = from.segment;
                loop {
                    i = (i + 1) % n;
                    out.push(self.points[i]);
                    if i == to.segment {
                        break;
                    }
                }
            }
        } else {
            let same = from.segment == to.segment && from.t >= to.t;
            if !same {
                // The start of each segment from `from`'s back to the one
                // after `to`'s.
                let mut i = from.segment;
                loop {
                    out.push(self.points[i]);
                    if i == (to.segment + 1) % n {
                        break;
                    }
                    i = (i + n - 1) % n;
                }
            }
        }
        out.push(at(to));
        out
    }

    /// The part of the section between the points nearest `a` and `b`:
    /// the shorter way round if it is closed.
    pub fn between(&self, a: Vec3, b: Vec3) -> Option<Vec<Vec3>> {
        let (from, to) = (self.locate(a)?, self.locate(b)?);
        let (sa, sb) = (self.arc_at(from), self.arc_at(to));
        let forward = if self.closed {
            let ahead = (sb - sa).rem_euclid(self.length());
            ahead <= self.length() - ahead
        } else {
            sb >= sa
        };
        Some(self.run(from, to, forward))
    }
}

#[derive(Debug, Clone, Copy)]
struct Spot {
    segment: usize,
    t: f32,
    gap: f32,
}

fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let d = b - a;
    let len2 = d.length_squared();
    if len2 < 1e-18 {
        return 0.0;
    }
    ((p - a).dot(d) / len2).clamp(0.0, 1.0)
}

/// Positions rounded to a tenth of a millimetre, so the same corner shared
/// by triangles that don't share an index (as in an OBJ file whose faces
/// each have their own vertices) is still recognised as one.
type Key = [i64; 3];

fn key(p: Vec3) -> Key {
    let q = (p * 1e4).round();
    [q.x as i64, q.y as i64, q.z as i64]
}

/// Cut `mesh` with the plane through `origin` facing `normal`: every
/// connected piece of the cut, as polylines in the plane.
pub fn slice(mesh: &Mesh, origin: Vec3, normal: Vec3) -> Vec<Section> {
    let Some(normal) = normal.try_normalize() else {
        return Vec::new();
    };
    // Each triangle the plane crosses gives a segment between two of its
    // edges; segments of neighbouring triangles meet on the edge they
    // share, named by its two corners.
    let side = |p: Vec3| (p - origin).dot(normal);
    let mut points: HashMap<(Key, Key), Vec3> = HashMap::new();
    let mut segments: Vec<[(Key, Key); 2]> = Vec::new();
    for tri in mesh.triangles() {
        let d = tri.map(side);
        let mut ends = Vec::with_capacity(2);
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            // A corner exactly on the plane counts as below it.
            if (d[i] > 0.0) == (d[j] > 0.0) {
                continue;
            }
            let (ki, kj) = (key(tri[i]), key(tri[j]));
            if ki == kj {
                continue;
            }
            let edge = if ki < kj { (ki, kj) } else { (kj, ki) };
            let t = d[i] / (d[i] - d[j]);
            points.entry(edge).or_insert_with(|| tri[i].lerp(tri[j], t));
            ends.push(edge);
        }
        if let [a, b] = ends[..] {
            if a != b {
                segments.push([a, b]);
            }
        }
    }

    let mut by_edge: HashMap<(Key, Key), Vec<usize>> = HashMap::new();
    for (i, seg) in segments.iter().enumerate() {
        for edge in seg {
            by_edge.entry(*edge).or_default().push(i);
        }
    }
    let mut used = vec![false; segments.len()];
    // From `edge`, follow unused segments as far as they go.
    let walk = |start: usize, edge: (Key, Key), used: &mut Vec<bool>| {
        let mut chain = Vec::new();
        let (mut at, mut current) = (edge, start);
        loop {
            let [a, b] = segments[current];
            let next_edge = if a == at { b } else { a };
            chain.push(next_edge);
            let next = by_edge
                .get(&next_edge)
                .and_then(|list| list.iter().copied().find(|&k| !used[k]));
            match next {
                Some(k) => {
                    used[k] = true;
                    at = next_edge;
                    current = k;
                }
                None => return chain,
            }
        }
    };
    let mut out = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let [a, b] = segments[start];
        // Forwards from `a` through `b`, then backwards from `a` if the
        // cut is open.
        let mut chain = vec![a];
        chain.extend(walk(start, a, &mut used));
        let closed = chain.len() > 2 && chain.first() == chain.last();
        if closed {
            chain.pop();
        } else {
            let mut back = walk(start, b, &mut used);
            // `walk` from `b` starts by stepping to `a` again.
            back.remove(0);
            back.reverse();
            back.extend(chain);
            chain = back;
        }
        out.push(Section {
            points: chain.iter().map(|e| points[e]).collect(),
            closed,
        });
    }
    out
}

/// A girth around the body at one height.
#[derive(Debug, Clone, PartialEq)]
pub struct Girth {
    /// The cross-section the girth goes around.
    pub section: Section,
    /// How long a tape pulled snug around it is: the length of its convex
    /// hull, which bridges hollows (between the breasts, say) the way a
    /// real tape measure does.
    pub tape: f32,
}

/// The girth around the closed part of the avatar's cross-section at
/// height `y` that passes nearest `near` (a clicked point on the body),
/// so a click on an arm measures the arm and a click on the chest the
/// chest.
pub fn girth(mesh: &Mesh, y: f32, near: Vec3) -> Option<Girth> {
    let sections = slice(mesh, Vec3::new(0.0, y, 0.0), Vec3::Y);
    let section = sections
        .into_iter()
        .filter(|s| s.closed && s.points.len() >= 3)
        .filter_map(|s| Some((s.locate(near)?.gap, s)))
        .min_by(|a, b| a.0.total_cmp(&b.0))?
        .1;
    let flat: Vec<Vec2> = section.points.iter().map(|p| Vec2::new(p.x, p.z)).collect();
    Some(Girth {
        tape: hull_perimeter(&flat),
        section,
    })
}

/// Perimeter of the convex hull of `points` (Andrew's monotone chain).
pub fn hull_perimeter(points: &[Vec2]) -> f32 {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 2 {
        return 0.0;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in ordered {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        // Each half ends where the other starts.
        hull.pop();
    }
    hull.iter()
        .zip(hull.iter().cycle().skip(1))
        .map(|(a, b)| a.distance(*b))
        .sum()
}

/// A tape laid over the surface from `a` to `b` (two points on `mesh`,
/// with `up` the averaged surface normal there): the shorter way along
/// the cut through both points that is square to the surface, which
/// follows the body's curve as a tape pressed onto it does. A straight
/// line where that cut doesn't join them (they are on different parts).
pub fn surface_path(mesh: &Mesh, a: Vec3, b: Vec3, up: Vec3) -> Vec<Vec3> {
    let straight = vec![a, b];
    let along = b - a;
    if along.length() < 1e-6 {
        return straight;
    }
    let Some(normal) = along.cross(up).try_normalize() else {
        return straight;
    };
    // The piece of the cut both points lie on.
    const REACH: f32 = 0.005;
    slice(mesh, a, normal)
        .into_iter()
        .filter_map(|s| {
            let gap = s.locate(a)?.gap.max(s.locate(b)?.gap);
            (gap < REACH).then_some((gap, s))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .and_then(|(_, s)| s.between(a, b))
        .unwrap_or(straight)
}

/// Total length of an open polyline.
pub fn path_length(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The surface normal of a triangle (by index into `mesh`'s triangle
/// list): its vertex normals blended by `weights`, or its face normal if
/// the mesh has none.
pub fn normal_at(mesh: &Mesh, triangle: usize, weights: [f32; 3]) -> Option<Vec3> {
    let idx = mesh.indices.get(triangle * 3..triangle * 3 + 3)?;
    let corners = [0, 1, 2].map(|k| idx[k] as usize);
    let blended = if corners.iter().all(|&i| i < mesh.normals.len()) {
        corners
            .iter()
            .zip(weights)
            .map(|(&i, w)| mesh.normals[i] * w)
            .sum::<Vec3>()
    } else {
        let [p, q, r] = corners.map(|i| mesh.positions[i]);
        (q - p).cross(r - p)
    };
    blended.try_normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{avatar_mesh, landmarks, AvatarKind};
    use crate::mesh::tests::cube;

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn a_cube_cut_level_is_one_closed_square() {
        let cube = cube();
        let cut = slice(&cube, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
        assert_eq!(cut.len(), 1);
        assert!(cut[0].closed);
        assert!(close(cut[0].length(), 4.0, 1e-5), "{}", cut[0].length());
        assert!(cut[0].points.iter().all(|p| close(p.y, 0.5, 1e-6)));
        // Through a corner and on a slant: still one loop, all on the plane.
        let n = Vec3::new(1.0, 1.0, 0.3);
        let cut = slice(&cube, Vec3::ZERO, n);
        assert!(cut
            .iter()
            .all(|s| s.points.iter().all(|p| close(p.dot(n), 0.0, 1e-5))));
        // Missing the cube altogether.
        assert!(slice(&cube, Vec3::new(0.0, 2.0, 0.0), Vec3::Y).is_empty());
    }

    #[test]
    fn faces_with_their_own_vertices_still_join_up() {
        // Every triangle with its own copy of its corners, as OBJ import
        // can give.
        let shared = cube();
        let mut split = Mesh::default();
        for tri in shared.triangles() {
            let base = split.positions.len() as u32;
            split.positions.extend(tri);
            split.indices.extend([base, base + 1, base + 2]);
        }
        let cut = slice(&split, Vec3::new(0.0, 0.25, 0.0), Vec3::Y);
        assert_eq!(cut.len(), 1);
        assert!(cut[0].closed);
        assert!(close(cut[0].length(), 4.0, 1e-5));
    }

    #[test]
    fn an_open_mesh_cuts_to_an_open_line() {
        // The cube without its x = 0 side: the cut is a U.
        let mut open = cube();
        open.indices.truncate(open.indices.len() - 12);
        open.indices.extend_from_slice(&[1, 3, 5, 3, 7, 5]);
        let cut = slice(&open, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
        assert_eq!(cut.len(), 1);
        assert!(!cut[0].closed);
        assert!(close(cut[0].length(), 3.0, 1e-5), "{}", cut[0].length());
    }

    #[test]
    fn hull_bridges_hollows() {
        let square = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| Vec2::new(x, y));
        assert!(close(hull_perimeter(&square), 4.0, 1e-6));
        // A dent in one side doesn't change the tape round it.
        let dented = [(0.0, 0.0), (0.5, 0.4), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            .map(|(x, y)| Vec2::new(x, y));
        assert!(close(hull_perimeter(&dented), 4.0, 1e-6));
    }

    #[test]
    fn the_avatar_waist_girth_matches_its_measurement() {
        for kind in AvatarKind::ALL {
            let m = kind.default_measurements();
            let mesh = avatar_mesh(kind, m);
            let marks = landmarks(kind, m);
            for (y, girth_cm) in [(marks.waist, m.waist), (marks.hip, m.hip)] {
                let front = Vec3::new(0.0, y, 1.0);
                let g = girth(&mesh, y, front).expect("a loop round the body");
                let want = girth_cm / 100.0;
                // The mesh is polygonal and smoothed between its rings.
                assert!(
                    close(g.tape, want, want * 0.03),
                    "{kind:?} at {y}: {} vs {want}",
                    g.tape
                );
                assert!(g.tape <= g.section.length() + 1e-5);
                assert!(g.section.points.iter().all(|p| close(p.y, y, 1e-5)));
            }
        }
    }

    #[test]
    fn a_click_on_an_arm_measures_the_arm() {
        let kind = AvatarKind::Female;
        let m = kind.default_measurements();
        let mesh = avatar_mesh(kind, m);
        let y = landmarks(kind, m).chest;
        let body = girth(&mesh, y, Vec3::new(0.0, y, 1.0)).unwrap();
        // The cross-section's right-most point is on the right arm.
        let cut = slice(&mesh, Vec3::new(0.0, y, 0.0), Vec3::Y);
        let side = cut
            .iter()
            .flat_map(|s| s.points.iter().copied())
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        let arm = girth(&mesh, y, side).unwrap();
        assert!(arm.tape < body.tape * 0.5, "{} vs {}", arm.tape, body.tape);
    }

    #[test]
    fn a_tape_over_a_cube_edge_follows_both_faces() {
        let cube = cube();
        // From the middle of the front face to the middle of the right one.
        let a = Vec3::new(0.5, 0.5, 1.0);
        let b = Vec3::new(1.0, 0.5, 0.5);
        let up = (Vec3::Z + Vec3::X).normalize();
        let path = surface_path(&cube, a, b, up);
        assert_eq!(path.first(), Some(&a));
        assert_eq!(path.last(), Some(&b));
        assert!(close(path_length(&path), 1.0, 1e-5), "{path:?}");
        // The other way round is the same tape, reversed.
        let back = surface_path(&cube, b, a, up);
        assert!(close(path_length(&back), 1.0, 1e-5));
        // Two points on one face: a straight tape.
        let c = Vec3::new(0.2, 0.3, 1.0);
        let flat = surface_path(&cube, a, c, Vec3::Z);
        assert!(close(path_length(&flat), a.distance(c), 1e-5));
    }

    #[test]
    fn a_tape_round_the_body_is_longer_than_the_chord() {
        let kind = AvatarKind::Male;
        let m = kind.default_measurements();
        let mesh = avatar_mesh(kind, m);
        let y = landmarks(kind, m).waist;
        // Front and side of the waist.
        let cut = slice(&mesh, Vec3::new(0.0, y, 0.0), Vec3::Y);
        let torso = cut
            .iter()
            .filter(|s| s.closed)
            .max_by(|a, b| a.length().total_cmp(&b.length()))
            .unwrap();
        let front = *torso
            .points
            .iter()
            .max_by(|a, b| a.z.total_cmp(&b.z))
            .unwrap();
        let side = *torso
            .points
            .iter()
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        // The body faces +Z at the front and +X at the side.
        let path = surface_path(&mesh, front, side, (Vec3::X + Vec3::Z).normalize());
        let len = path_length(&path);
        assert!(len > front.distance(side) * 1.02, "{len}");
        // About a quarter of the way round.
        assert!(
            close(len, torso.length() / 4.0, torso.length() * 0.05),
            "{len}"
        );
    }

    #[test]
    fn normals_are_blended_from_the_vertices() {
        let cube = cube();
        // Triangle 0 is on the z = 0 face; its corners' normals all lean
        // out of it, so the blend still points away from the cube.
        let n = normal_at(&cube, 0, [1.0 / 3.0; 3]).unwrap();
        assert!(n.z < 0.0 && close(n.length(), 1.0, 1e-5));
        assert_eq!(normal_at(&cube, 99, [1.0, 0.0, 0.0]), None);
    }
}
