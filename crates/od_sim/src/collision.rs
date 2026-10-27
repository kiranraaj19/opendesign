//! Collision shapes cloth particles are kept clear of. Resolution is a
//! direct position push-out (as PBD expects), not a force.

use std::collections::HashMap;

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
    /// meant for arbitrary geometry. Triangles are bucketed into a grid
    /// once, at construction, so resolving a particle only tests nearby
    /// triangles rather than every one of them.
    Mesh {
        grid: TriangleGrid,
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
        Collider::Mesh {
            grid: TriangleGrid::build(&mesh),
            thickness,
        }
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
            Collider::Mesh { grid, thickness } => resolve_mesh(grid, *thickness, p),
        }
    }
}

type Cell = (i32, i32, i32);

/// Buckets a mesh's triangles into a uniform grid, so a query only has to
/// test the triangles near a point instead of all of them. Built once
/// when the collider is created — the mesh (typically the avatar) doesn't
/// change while draping.
#[derive(Debug, Clone)]
pub struct TriangleGrid {
    cell_size: f32,
    origin: Vec3,
    cells: HashMap<Cell, Vec<u32>>,
    triangles: Vec<[Vec3; 3]>,
}

impl TriangleGrid {
    pub fn build(mesh: &Mesh) -> Self {
        let triangles: Vec<[Vec3; 3]> = mesh.triangles().collect();
        let Some((min, max)) = bounds(&triangles) else {
            return Self {
                cell_size: 1.0,
                origin: Vec3::ZERO,
                cells: HashMap::new(),
                triangles,
            };
        };
        // Aim for roughly one triangle per cell on average.
        let diagonal = (max - min).length().max(1e-3);
        let cell_size = (diagonal / (triangles.len() as f32).cbrt().max(1.0)).max(1e-3);

        let mut cells: HashMap<Cell, Vec<u32>> = HashMap::new();
        for (i, &[a, b, c]) in triangles.iter().enumerate() {
            let lo = cell_of(a.min(b).min(c), min, cell_size);
            let hi = cell_of(a.max(b).max(c), min, cell_size);
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        cells.entry((x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        Self {
            cell_size,
            origin: min,
            cells,
            triangles,
        }
    }

    /// Triangle indices worth testing near `p`: the cells around it,
    /// widening the search ring until some turn up some triangles, so a
    /// point just outside the mesh's bounds (or in a sparser pocket of
    /// it) still finds its nearest one. Falls back to every triangle in
    /// the rare case nothing turns up nearby at all.
    fn candidates(&self, p: Vec3) -> Vec<u32> {
        if self.triangles.is_empty() {
            return Vec::new();
        }
        let center = cell_of(p, self.origin, self.cell_size);
        for radius in 0..4 {
            let mut found = Vec::new();
            for x in center.0 - radius..=center.0 + radius {
                for y in center.1 - radius..=center.1 + radius {
                    for z in center.2 - radius..=center.2 + radius {
                        if let Some(tris) = self.cells.get(&(x, y, z)) {
                            found.extend(tris);
                        }
                    }
                }
            }
            if !found.is_empty() {
                return found;
            }
        }
        (0..self.triangles.len() as u32).collect()
    }
}

fn cell_of(p: Vec3, origin: Vec3, cell_size: f32) -> Cell {
    let v = (p - origin) / cell_size;
    (v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32)
}

fn bounds(triangles: &[[Vec3; 3]]) -> Option<(Vec3, Vec3)> {
    let first = *triangles.first()?.first()?;
    Some(
        triangles
            .iter()
            .flatten()
            .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p))),
    )
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

/// Push `p` outside `grid`'s mesh by at least `thickness`, using
/// whichever nearby triangle it is currently closest to.
fn resolve_mesh(grid: &TriangleGrid, thickness: f32, p: Vec3) -> Vec3 {
    let mut best: Option<(f32, Vec3, Vec3)> = None;
    for i in grid.candidates(p) {
        let [a, b, c] = grid.triangles[i as usize];
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

    /// An `n` x `n` subdivided flat plane in the XZ plane at y = 0,
    /// `size` metres across: dense enough to exercise the grid's cell
    /// bucketing and ring expansion, not just a single triangle's cell.
    fn grid_mesh(n: usize, size: f32) -> Mesh {
        let mut positions = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                let x = (i as f32 / n as f32 - 0.5) * size;
                let z = (j as f32 / n as f32 - 0.5) * size;
                positions.push(Vec3::new(x, 0.0, z));
            }
        }
        let mut indices = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let i0 = (j * (n + 1) + i) as u32;
                let i1 = i0 + 1;
                let i2 = i0 + (n + 1) as u32;
                let i3 = i2 + 1;
                indices.extend([i0, i2, i1, i1, i2, i3]);
            }
        }
        let normals = vec![Vec3::Y; positions.len()];
        Mesh {
            positions,
            normals,
            indices,
        }
    }

    /// The pre-grid algorithm, scanning every triangle, kept here only to
    /// check the grid-accelerated one against it.
    fn resolve_mesh_bruteforce(mesh: &Mesh, thickness: f32, p: Vec3) -> Vec3 {
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

    #[test]
    fn grid_collider_matches_brute_force_on_a_dense_mesh() {
        let mesh = grid_mesh(10, 4.0);
        let collider = Collider::mesh(mesh.clone(), 0.05);
        for p in [
            Vec3::new(0.3, -0.2, 0.7),
            Vec3::new(-1.5, 0.3, 1.9),
            Vec3::new(1.98, 0.02, -1.98), // right at a corner
            Vec3::new(0.0, 5.0, 0.0),     // far above, untouched
            Vec3::new(10.0, 10.0, 10.0),  // well outside the grid entirely
        ] {
            let fast = collider.resolve(p);
            let brute = resolve_mesh_bruteforce(&mesh, 0.05, p);
            assert!(
                fast.distance(brute) < 1e-4,
                "{p:?}: grid gave {fast:?}, brute force gave {brute:?}"
            );
        }
    }
}
