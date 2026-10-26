//! CPU ray casting for clicking things in the 3D view.

use glam::Vec3;

use crate::mesh::Mesh;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    /// Unit length.
    pub dir: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self {
            origin,
            dir: dir.normalize(),
        }
    }

    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

/// Distance along the ray to a triangle, hit from either side
/// (Möller–Trumbore).
pub fn ray_triangle(ray: &Ray, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = ray.dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None; // Parallel, or a degenerate triangle.
    }
    let inv = 1.0 / det;
    let s = ray.origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = ray.dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// Nearest hit on a mesh.
pub fn ray_mesh(ray: &Ray, mesh: &Mesh) -> Option<f32> {
    mesh.triangles()
        .filter_map(|tri| ray_triangle(ray, tri))
        .min_by(f32::total_cmp)
}

/// The item whose mesh the ray hits first, with the hit distance.
pub fn closest_hit<'a, K>(
    ray: &Ray,
    items: impl IntoIterator<Item = (K, &'a Mesh)>,
) -> Option<(K, f32)> {
    items
        .into_iter()
        .filter_map(|(key, mesh)| ray_mesh(ray, mesh).map(|t| (key, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::tests::cube;

    const TRI: [Vec3; 3] = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];

    #[test]
    fn hits_triangle_from_both_sides() {
        let front = Ray::new(Vec3::new(0.25, 0.25, 2.0), -Vec3::Z);
        assert!((ray_triangle(&front, TRI).unwrap() - 2.0).abs() < 1e-6);
        let back = Ray::new(Vec3::new(0.25, 0.25, -3.0), Vec3::Z);
        assert!((ray_triangle(&back, TRI).unwrap() - 3.0).abs() < 1e-6);
    }

    #[test]
    fn misses_outside_behind_and_parallel() {
        let outside = Ray::new(Vec3::new(0.8, 0.8, 1.0), -Vec3::Z);
        assert_eq!(ray_triangle(&outside, TRI), None);
        let behind = Ray::new(Vec3::new(0.25, 0.25, 1.0), Vec3::Z);
        assert_eq!(ray_triangle(&behind, TRI), None);
        let parallel = Ray::new(Vec3::new(-1.0, 0.25, 0.0), Vec3::X);
        assert_eq!(ray_triangle(&parallel, TRI), None);
    }

    #[test]
    fn nearest_mesh_wins() {
        let near = cube();
        let mut far = cube();
        far.scale_translate(1.0, Vec3::new(0.0, 0.0, -5.0));
        let ray = Ray::new(Vec3::new(0.5, 0.5, 10.0), -Vec3::Z);
        assert!((ray_mesh(&ray, &near).unwrap() - 9.0).abs() < 1e-5);
        let hit = closest_hit(&ray, [("far", &far), ("near", &near)]);
        assert_eq!(hit.map(|h| h.0), Some("near"));
        let miss = Ray::new(Vec3::new(5.0, 5.0, 10.0), -Vec3::Z);
        assert_eq!(closest_hit(&miss, [("near", &near)]), None);
    }
}
