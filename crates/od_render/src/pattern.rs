//! Flat patterns in 3D: triangulated outlines placed on a ring around the
//! avatar, the way CLO lays pieces out before sewing.

use glam::{Vec2, Vec3};

use crate::mesh::{Aabb, Mesh};

/// Triangles of a simple polygon (no holes), as indices into `polygon`.
/// Returns nothing for degenerate outlines.
pub fn triangulate(polygon: &[Vec2]) -> Vec<[u32; 3]> {
    // A repeated closing point would make a zero-length edge.
    let n = match polygon {
        [first, .., last] if first == last => polygon.len() - 1,
        _ => polygon.len(),
    };
    if n < 3 {
        return Vec::new();
    }
    let flat: Vec<f64> = polygon[..n]
        .iter()
        .flat_map(|p| [f64::from(p.x), f64::from(p.y)])
        .collect();
    let Ok(indices) = earcutr::earcut(&flat, &[], 2) else {
        return Vec::new();
    };
    indices
        .chunks_exact(3)
        .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
        .collect()
}

/// Where patterns go: a horizontal ring around the avatar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingLayout {
    /// Metres from the vertical axis.
    pub radius: f32,
    /// Height of the pattern centres, metres.
    pub height: f32,
}

impl RingLayout {
    /// Just clear of the avatar, around chest height.
    pub fn around(avatar: Aabb) -> Self {
        let reach = [avatar.min.x, avatar.max.x, avatar.min.z, avatar.max.z]
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max);
        Self {
            radius: (reach + 0.12).max(0.3),
            height: avatar.min.y + avatar.size().y * 0.62,
        }
    }

    /// Slot `index` of `count`, evenly spaced, the first in front.
    pub fn placement(&self, index: usize, count: usize) -> Placement {
        let angle = index as f32 / count.max(1) as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        Placement {
            center: Vec3::new(self.radius * sin, self.height, self.radius * cos),
            right: Vec3::new(cos, 0.0, -sin),
            up: Vec3::Y,
        }
    }
}

/// A pattern's plane in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub center: Vec3,
    /// Pattern +x, in the world.
    pub right: Vec3,
    /// Pattern -y (2D y points down), in the world.
    pub up: Vec3,
}

impl Placement {
    /// The side facing away from the avatar.
    pub fn normal(&self) -> Vec3 {
        self.right.cross(self.up)
    }

    /// Map a 2D outline in millimetres to the world, centred on its bounds.
    pub fn place(&self, outline_mm: &[Vec2]) -> Vec<Vec3> {
        let Some((min, max)) = bounds_2d(outline_mm) else {
            return Vec::new();
        };
        let mid = (min + max) * 0.5;
        outline_mm
            .iter()
            .map(|p| {
                let d = (*p - mid) / 1000.0;
                self.center + self.right * d.x - self.up * d.y
            })
            .collect()
    }
}

fn bounds_2d(points: &[Vec2]) -> Option<(Vec2, Vec2)> {
    let first = *points.first()?;
    Some(
        points
            .iter()
            .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))),
    )
}

/// A double-sided mesh: the front faces along the placement's normal, and
/// a back copy faces the other way, so the piece shows from both sides even
/// with back-face culling.
pub fn pattern_mesh(outline_mm: &[Vec2], placement: &Placement) -> Option<Mesh> {
    let triangles = triangulate(outline_mm);
    if triangles.is_empty() {
        return None;
    }
    let world = placement.place(outline_mm);
    let normal = placement.normal();
    let n = world.len() as u32;

    let mut positions = world.clone();
    positions.extend_from_slice(&world);
    let mut normals = vec![normal; world.len()];
    normals.extend(std::iter::repeat_n(-normal, world.len()));

    let mut indices = Vec::with_capacity(triangles.len() * 6);
    for [a, b, c] in triangles {
        let (pa, pb, pc) = (world[a as usize], world[b as usize], world[c as usize]);
        // Earcut's winding depends on the outline's; make the front face out.
        let front = if (pb - pa).cross(pc - pa).dot(normal) >= 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        indices.extend(front);
        indices.extend([front[0] + n, front[2] + n, front[1] + n]);
    }
    Some(Mesh {
        positions,
        normals,
        indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pick::{ray_mesh, Ray};

    fn area(tris: &[[u32; 3]], pts: &[Vec2]) -> f32 {
        tris.iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (pts[a as usize], pts[b as usize], pts[c as usize]);
                (b - a).perp_dot(c - a).abs() * 0.5
            })
            .sum()
    }

    fn l_shape() -> Vec<Vec2> {
        [
            (0.0, 0.0),
            (300.0, 0.0),
            (300.0, 100.0),
            (100.0, 100.0),
            (100.0, 400.0),
            (0.0, 400.0),
        ]
        .map(|(x, y)| Vec2::new(x, y))
        .to_vec()
    }

    #[test]
    fn triangulates_concave_outlines_exactly() {
        let l = l_shape();
        let tris = triangulate(&l);
        assert_eq!(tris.len(), l.len() - 2);
        assert!((area(&tris, &l) - (300.0 * 100.0 + 100.0 * 300.0)).abs() < 1e-2);
        // Either winding works.
        let mut reversed = l.clone();
        reversed.reverse();
        assert_eq!(triangulate(&reversed).len(), 4);
    }

    #[test]
    fn degenerate_outlines_give_nothing() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[Vec2::ZERO, Vec2::X]).is_empty());
        // A closing duplicate is ignored rather than breaking the result.
        let mut closed = l_shape();
        closed.push(closed[0]);
        assert_eq!(triangulate(&closed).len(), 4);
    }

    #[test]
    fn ring_is_clear_of_the_avatar_and_starts_in_front() {
        let avatar = Aabb {
            min: Vec3::new(-0.35, 0.0, -0.15),
            max: Vec3::new(0.35, 1.7, 0.2),
        };
        let ring = RingLayout::around(avatar);
        assert!(ring.radius > 0.35);
        assert!(ring.height > 1.0 && ring.height < 1.3);
        let front = ring.placement(0, 4);
        assert!(front.center.z > 0.0 && front.center.x.abs() < 1e-6);
        assert!((front.normal() - Vec3::Z).length() < 1e-6, "faces outward");
        for i in 0..4 {
            let p = ring.placement(i, 4);
            assert!(
                (p.center.length_squared() - p.center.y.powi(2) - ring.radius.powi(2)).abs() < 1e-4
            );
            assert!(p.normal().dot(p.center.with_y(0.0)) > 0.0);
        }
        let side = ring.placement(1, 4);
        assert!(side.center.x > 0.0, "second slot to the avatar's left");
    }

    #[test]
    fn placing_keeps_size_and_orientation() {
        let front = RingLayout {
            radius: 0.4,
            height: 1.2,
        }
        .placement(0, 1);
        let world = front.place(&l_shape());
        // mm to m, centred on the bounds' midpoint.
        let b = Aabb::from_points(&world).unwrap();
        assert!((b.size().x - 0.3).abs() < 1e-5 && (b.size().y - 0.4).abs() < 1e-5);
        assert!(b.center().distance(front.center) < 1e-5);
        // 2D y points down, so the first point (top-left) is up and left.
        assert!(world[0].y > front.center.y && world[0].x < front.center.x);
    }

    #[test]
    fn pattern_mesh_is_flat_and_double_sided() {
        let front = RingLayout {
            radius: 0.4,
            height: 1.2,
        }
        .placement(0, 1);
        for outline in [l_shape(), l_shape().into_iter().rev().collect()] {
            let mesh = pattern_mesh(&outline, &front).unwrap();
            assert_eq!(mesh.positions.len(), 12);
            assert_eq!(mesh.triangle_count(), 8);
            for (i, t) in mesh.indices.chunks_exact(3).enumerate() {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                let face = (b - a).cross(c - a).normalize();
                let n = mesh.normals[t[0] as usize];
                assert!(face.dot(n) > 0.999, "triangle {i} winds with its normal");
                assert!((a.z - 0.4).abs() < 1e-6, "flat");
            }
            assert_eq!(mesh.normals[0], Vec3::Z);
            assert_eq!(mesh.normals[6], -Vec3::Z);
            // Clickable from either side.
            let target = Vec3::new(-0.1, 1.25, 0.4);
            assert!(ray_mesh(&Ray::new(target + Vec3::Z, -Vec3::Z), &mesh).is_some());
            assert!(ray_mesh(&Ray::new(target - Vec3::Z, Vec3::Z), &mesh).is_some());
        }
        assert!(pattern_mesh(&[Vec2::ZERO], &front).is_none());
    }
}
