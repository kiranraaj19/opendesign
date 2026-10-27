//! Turns simulated particle positions back into a renderable mesh, the
//! way [`od_render::pattern::pattern_mesh`] does for a flat pattern.

use glam::Vec3;
use od_render::mesh::Mesh;

/// A double-sided mesh over `positions`, using `faces` (indices into
/// `positions`) for both topology and smooth, area-weighted normals. `None`
/// if there are no faces.
pub fn draped_mesh(positions: &[Vec3], faces: &[[u32; 3]]) -> Option<Mesh> {
    if faces.is_empty() {
        return None;
    }
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for &[a, b, c] in faces {
        let (pa, pb, pc) = (
            positions[a as usize],
            positions[b as usize],
            positions[c as usize],
        );
        let face_normal = (pb - pa).cross(pc - pa);
        normals[a as usize] += face_normal;
        normals[b as usize] += face_normal;
        normals[c as usize] += face_normal;
    }
    for normal in &mut normals {
        *normal = normal.normalize_or_zero();
    }

    let n = positions.len() as u32;
    let mut out_positions = positions.to_vec();
    out_positions.extend_from_slice(positions);
    let mut out_normals = normals.clone();
    out_normals.extend(normals.iter().map(|v| -*v));

    let mut indices = Vec::with_capacity(faces.len() * 6);
    for &[a, b, c] in faces {
        indices.extend([a, b, c]);
        indices.extend([a + n, c + n, b + n]);
    }

    Some(Mesh {
        positions: out_positions,
        normals: out_normals,
        indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_faces_gives_nothing() {
        assert!(draped_mesh(&[Vec3::ZERO, Vec3::X, Vec3::Y], &[]).is_none());
    }

    #[test]
    fn builds_a_double_sided_mesh_with_opposite_normals() {
        let positions = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let mesh = draped_mesh(&positions, &[[0, 1, 2]]).unwrap();
        assert_eq!(mesh.positions.len(), 6);
        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(mesh.normals[0], -mesh.normals[3]);
        assert!((mesh.normals[0].length() - 1.0).abs() < 1e-5);
    }
}
