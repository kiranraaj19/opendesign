//! Collision shapes cloth particles are kept clear of. Resolution is a
//! direct position push-out (as PBD expects), not a force.

use glam::Vec3;
use od_render::mesh::Mesh;

/// A shape particles may not penetrate.
#[derive(Debug, Clone)]
pub enum Collider {
    /// An infinite horizontal plane at world height `y`.
    Ground {
        y: f32,
    },
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// A closed triangle mesh (typically the avatar). Push-out uses the
    /// closest triangle's plane, which is an approximation that can miss
    /// thin or highly concave features; fine for an avatar-sized body, not
    /// meant for arbitrary geometry.
    Mesh {
        mesh: Mesh,
        thickness: f32,
    },
}

impl Collider {
    pub fn ground(y: f32) -> Self {
        Collider::Ground { y }
    }

    pub fn sphere(center: Vec3, radius: f32) -> Self {
        Collider::Sphere { center, radius }
    }

    pub fn mesh(mesh: Mesh, thickness: f32) -> Self {
        Collider::Mesh { mesh, thickness }
    }

    /// Where `p` ends up after this collider pushes it clear, if it was
    /// penetrating. Returns `p` unchanged otherwise.
    pub fn resolve(&self, p: Vec3) -> Vec3 {
        match self {
            Collider::Ground { y } => Vec3::new(p.x, p.y.max(*y), p.z),
            Collider::Sphere { center, radius } => {
                let offset = p - *center;
                let dist = offset.length();
                if dist < *radius && dist > 1e-9 {
                    *center + offset / dist * *radius
                } else if dist <= 1e-9 {
                    *center + Vec3::Y * *radius
                } else {
                    p
                }
            }
            Collider::Mesh { mesh, thickness } => resolve_mesh(mesh, *thickness, p),
        }
    }
}

fn closest_on_triangle(a: Vec3, b: Vec3, c: Vec3, p: Vec3) -> Vec3 {
    // Standard point-to-triangle projection via barycentric clamping.
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}

/// Push `p` outside `mesh` by at least `thickness`, using whichever
/// triangle it is currently closest to. O(triangle count); fine for an
/// avatar-sized mesh, not for very dense ones.
fn resolve_mesh(mesh: &Mesh, thickness: f32, p: Vec3) -> Vec3 {
    let mut best: Option<(f32, Vec3, Vec3)> = None;
    for [a, b, c] in mesh.triangles() {
        let closest = closest_on_triangle(a, b, c, p);
        let dist = closest.distance_squared(p);
        if best.is_none_or(|(d, _, _)| dist < d) {
            let normal = (b - a).cross(c - a).normalize_or_zero();
            best = Some((dist, closest, normal));
        }
    }
    let Some((dist_sq, closest, normal)) = best else {
        return p;
    };
    let dist = dist_sq.sqrt();
    let inside = (p - closest).dot(normal) < 0.0;
    if inside || dist < thickness {
        closest + normal * thickness
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_pushes_particles_back_above() {
        let ground = Collider::ground(0.5);
        assert_eq!(
            ground.resolve(Vec3::new(1.0, -2.0, 3.0)),
            Vec3::new(1.0, 0.5, 3.0)
        );
        assert_eq!(
            ground.resolve(Vec3::new(1.0, 5.0, 3.0)),
            Vec3::new(1.0, 5.0, 3.0)
        );
    }

    #[test]
    fn sphere_pushes_particles_to_its_surface() {
        let sphere = Collider::sphere(Vec3::ZERO, 1.0);
        let resolved = sphere.resolve(Vec3::new(0.5, 0.0, 0.0));
        assert!((resolved.length() - 1.0).abs() < 1e-5);
        assert_eq!(
            sphere.resolve(Vec3::new(2.0, 0.0, 0.0)),
            Vec3::new(2.0, 0.0, 0.0)
        );
    }

    fn flat_square_mesh() -> Mesh {
        // A 2x2 quad in the XZ plane at y=0, normal pointing +Y.
        let positions = vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
        ];
        let normals = vec![Vec3::Y; 4];
        Mesh {
            positions,
            normals,
            indices: vec![0, 2, 1, 0, 3, 2],
        }
    }

    #[test]
    fn mesh_collider_pushes_a_particle_above_the_surface() {
        let collider = Collider::mesh(flat_square_mesh(), 0.1);
        let resolved = collider.resolve(Vec3::new(0.0, -0.3, 0.0));
        assert!((resolved.y - 0.1).abs() < 1e-5, "resolved to {resolved:?}");
    }

    #[test]
    fn mesh_collider_leaves_clear_particles_alone() {
        let collider = Collider::mesh(flat_square_mesh(), 0.1);
        let far = Vec3::new(0.0, 5.0, 0.0);
        assert_eq!(collider.resolve(far), far);
    }
}
