//! Indexed triangle meshes in metres, Y up, plus the bounds and the
//! measurements the tests use to check them.

use glam::Vec3;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// One unit normal per position.
    pub normals: Vec<Vec3>,
    /// Counter-clockwise triangles when seen from the side the normals face.
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.indices.chunks_exact(3).map(|t| {
            [
                self.positions[t[0] as usize],
                self.positions[t[1] as usize],
                self.positions[t[2] as usize],
            ]
        })
    }

    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::from_points(&self.positions)
    }

    /// Smooth normals: each vertex gets the area-weighted average of the
    /// faces around it.
    pub fn compute_normals(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for t in self.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| self.positions[i as usize]);
            // Unnormalised: the length is twice the face area.
            let n = (b - a).cross(c - a);
            for i in t {
                normals[*i as usize] += n;
            }
        }
        self.normals = normals
            .into_iter()
            .map(|n| n.try_normalize().unwrap_or(Vec3::Y))
            .collect();
    }

    /// Append `other`, keeping its triangles separate.
    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    pub fn merged(parts: &[Mesh]) -> Mesh {
        let mut mesh = Mesh::default();
        for part in parts {
            mesh.append(part);
        }
        mesh
    }

    /// Scale about the origin, then move.
    pub fn scale_translate(&mut self, scale: f32, offset: Vec3) {
        for p in &mut self.positions {
            *p = *p * scale + offset;
        }
    }

    /// Signed enclosed volume (divergence theorem). Positive when a closed
    /// mesh's triangles face outward.
    pub fn signed_volume(&self) -> f32 {
        self.triangles()
            .map(|[a, b, c]| a.dot(b.cross(c)) / 6.0)
            .sum()
    }

    /// True when every edge is shared by exactly two triangles that use it
    /// in opposite directions: closed, with consistent winding.
    pub fn is_watertight(&self) -> bool {
        use std::collections::HashMap;
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in self.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if a == b {
                    return false;
                }
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        edges
            .iter()
            .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
    }
}

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_points(points: &[Vec3]) -> Option<Aabb> {
        let first = *points.first()?;
        Some(points.iter().fold(
            Aabb {
                min: first,
                max: first,
            },
            |b, &p| Aabb {
                min: b.min.min(p),
                max: b.max.max(p),
            },
        ))
    }

    pub fn union(self, other: Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

/// Perimeter of a closed polygon.
pub fn perimeter(points: &[Vec3]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.distance(*b))
        .sum()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A unit cube, outward-facing.
    pub fn cube() -> Mesh {
        let positions = (0..8)
            .map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32))
            .collect();
        #[rustfmt::skip]
        let indices = vec![
            0, 2, 1, 1, 2, 3, // z = 0
            4, 5, 6, 5, 7, 6, // z = 1
            0, 1, 4, 1, 5, 4, // y = 0
            2, 6, 3, 3, 6, 7, // y = 1
            0, 4, 2, 2, 4, 6, // x = 0
            1, 3, 5, 3, 7, 5, // x = 1
        ];
        let mut mesh = Mesh {
            positions,
            normals: vec![],
            indices,
        };
        mesh.compute_normals();
        mesh
    }

    #[test]
    fn cube_is_closed_and_outward() {
        let cube = cube();
        assert!(cube.is_watertight());
        assert!((cube.signed_volume() - 1.0).abs() < 1e-6);
        // Corner normals point away from the centre.
        let c = Vec3::splat(0.5);
        for (p, n) in cube.positions.iter().zip(&cube.normals) {
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert!(n.dot(*p - c) > 0.0);
        }
    }

    #[test]
    fn open_or_flipped_meshes_are_detected() {
        let mut open = cube();
        open.indices.truncate(open.indices.len() - 3);
        assert!(!open.is_watertight());

        let mut flipped = cube();
        flipped.indices[..3].reverse();
        assert!(!flipped.is_watertight(), "inconsistent winding");
    }

    #[test]
    fn append_offsets_indices() {
        let mut m = cube();
        m.append(&cube());
        assert_eq!(m.positions.len(), 16);
        assert_eq!(m.triangle_count(), 24);
        assert_eq!(*m.indices.iter().max().unwrap(), 15);
        assert!(m.is_watertight());
    }

    #[test]
    fn bounds_and_perimeter() {
        let b = cube().bounds().unwrap();
        assert_eq!(b.min, Vec3::ZERO);
        assert_eq!(b.size(), Vec3::ONE);
        let square = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z];
        assert!((perimeter(&square) - 4.0).abs() < 1e-6);
    }
}
