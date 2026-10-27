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
    ray_mesh_triangle(ray, mesh).map(|(t, _)| t)
}

/// Nearest hit on a mesh, with which triangle (by index into its
/// triangle list) it landed on.
pub fn ray_mesh_triangle(ray: &Ray, mesh: &Mesh) -> Option<(f32, usize)> {
    mesh.triangles()
        .enumerate()
        .filter_map(|(i, tri)| ray_triangle(ray, tri).map(|t| (t, i)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// Barycentric weights of `p`, a point on the triangle `a b c`.
pub fn triangle_weights([a, b, c]: [Vec3; 3], p: Vec3) -> [f32; 3] {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11) = (v0.dot(v0), v0.dot(v1), v1.dot(v1));
    let (d20, d21) = (v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-18 {
        return [1.0, 0.0, 0.0];
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    [1.0 - v - w, v, w]
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
    fn a_mesh_hit_names_its_triangle_and_weights() {
        let mesh = cube();
        let ray = Ray::new(Vec3::new(0.25, 0.5, -10.0), Vec3::Z);
        let (t, i) = ray_mesh_triangle(&ray, &mesh).unwrap();
        assert!((t - 10.0).abs() < 1e-5);
        let tri = mesh.triangles().nth(i).unwrap();
        let w = triangle_weights(tri, ray.at(t));
        let back = tri[0] * w[0] + tri[1] * w[1] + tri[2] * w[2];
        assert!(back.distance(ray.at(t)) < 1e-5);
        assert!(w.iter().all(|&x| (-1e-5..=1.0 + 1e-5).contains(&x)));
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
