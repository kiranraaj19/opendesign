//! Load a user-supplied avatar from OBJ or glTF and bring it into the
//! scene's conventions: metres, standing on the floor, centred on the Y axis.

use std::path::Path;

use glam::{Mat3, Mat4, Vec3};

use crate::mesh::Mesh;

pub const EXTENSIONS: [&str; 3] = ["obj", "gltf", "glb"];

pub fn load_avatar(path: &Path) -> Result<Mesh, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mesh = match ext.as_str() {
        "obj" => load_obj(path)?,
        "gltf" | "glb" => load_gltf(path)?,
        _ => {
            return Err(format!(
                "Unsupported avatar file type \".{ext}\"; use OBJ or glTF."
            ))
        }
    };
    if mesh.triangle_count() == 0 {
        return Err("The file has no triangles.".into());
    }
    Ok(normalize(mesh))
}

fn load_obj(path: &Path) -> Result<Mesh, String> {
    let (models, _materials) = tobj::load_obj(path, &tobj::GPU_LOAD_OPTIONS)
        .map_err(|e| format!("Can't read OBJ: {e}"))?;
    let mut out = Mesh::default();
    let mut has_normals = true;
    for model in models {
        let m = model.mesh;
        let positions: Vec<Vec3> = m
            .positions
            .chunks_exact(3)
            .map(|p| Vec3::new(p[0], p[1], p[2]))
            .collect();
        let normals: Vec<Vec3> = m
            .normals
            .chunks_exact(3)
            .map(|n| Vec3::new(n[0], n[1], n[2]).normalize_or(Vec3::Y))
            .collect();
        has_normals &= normals.len() == positions.len();
        out.append(&Mesh {
            normals,
            positions,
            indices: m.indices,
        });
    }
    if !has_normals {
        out.compute_normals();
    }
    Ok(out)
}

fn load_gltf(path: &Path) -> Result<Mesh, String> {
    let (doc, buffers, _images) =
        gltf::import(path).map_err(|e| format!("Can't read glTF: {e}"))?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("The glTF file has no scene.")?;
    let mut out = Mesh::default();
    let mut has_normals = true;
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let normal_matrix = Mat3::from_mat4(transform).inverse().transpose();
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let positions: Vec<Vec3> = positions
                    .map(|p| transform.transform_point3(Vec3::from(p)))
                    .collect();
                let normals: Vec<Vec3> = reader
                    .read_normals()
                    .map(|n| {
                        n.map(|n| (normal_matrix * Vec3::from(n)).normalize_or(Vec3::Y))
                            .collect()
                    })
                    .unwrap_or_default();
                has_normals &= normals.len() == positions.len();
                let indices = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                // Mirroring transforms flip the winding.
                let mut indices: Vec<u32> = indices;
                if transform.determinant() < 0.0 {
                    for t in indices.chunks_exact_mut(3) {
                        t.swap(1, 2);
                    }
                }
                out.append(&Mesh {
                    positions,
                    normals,
                    indices,
                });
            }
        }
        stack.extend(node.children().map(|c| (c, transform)));
    }
    if !has_normals {
        out.compute_normals();
    }
    Ok(out)
}

/// Guess the unit from the size (a person is 0.5–2.5 m tall), then stand
/// the model on the floor, centred on the Y axis. Files are expected to be
/// Y-up, as glTF requires and most OBJ exporters do.
pub fn normalize(mut mesh: Mesh) -> Mesh {
    let Some(b) = mesh.bounds() else {
        return mesh;
    };
    let height = b.size().y;
    let scale = if height > 250.0 {
        0.001 // millimetres
    } else if height > 2.5 {
        0.01 // centimetres
    } else {
        1.0
    };
    let c = b.center();
    mesh.scale_translate(scale, Vec3::new(-c.x, -b.min.y, -c.z) * scale);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("od_render_{}_{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tetrahedron 1700 units tall, offset from the origin.
    const TETRA: [[f32; 3]; 4] = [
        [100.0, 50.0, 100.0],
        [400.0, 50.0, 100.0],
        [250.0, 50.0, 400.0],
        [250.0, 1750.0, 200.0],
    ];

    #[test]
    fn obj_in_millimetres_is_normalised() {
        let path = temp_dir("obj").join("avatar.obj");
        let mut obj = String::new();
        for [x, y, z] in TETRA {
            obj += &format!("v {x} {y} {z}\n");
        }
        obj += "f 1 2 3\nf 1 4 2\nf 2 4 3\nf 3 4 1\n";
        std::fs::write(&path, obj).unwrap();

        let mesh = load_avatar(&path).unwrap();
        assert_eq!(mesh.triangle_count(), 4);
        let b = mesh.bounds().unwrap();
        assert!((b.size().y - 1.7).abs() < 1e-4, "mm to m: {b:?}");
        assert!(b.min.y.abs() < 1e-6, "on the floor");
        assert!(b.center().x.abs() < 1e-6 && b.center().z.abs() < 1e-6);
        assert_eq!(mesh.normals.len(), 4, "normals computed");
        assert!(mesh.is_watertight());
        assert!(mesh.signed_volume() > 0.0);
    }

    #[test]
    fn gltf_with_node_transform_loads() {
        let dir = temp_dir("gltf");
        // Positions in centimetres, a node scaling them by 2.
        let positions: Vec<f32> = TETRA.iter().flat_map(|p| p.map(|v| v / 20.0)).collect();
        let indices: [u16; 12] = [0, 1, 2, 0, 3, 1, 1, 3, 2, 2, 3, 0];
        let mut bin: Vec<u8> = bytemuck::cast_slice(&positions).to_vec();
        bin.extend_from_slice(bytemuck::cast_slice(&indices));
        std::fs::write(dir.join("avatar.bin"), &bin).unwrap();
        let json = format!(
            r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0, "scale": [2, 2, 2]}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
  "buffers": [{{"uri": "avatar.bin", "byteLength": {len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 48, "byteLength": 24}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
      "min": [5, 2.5, 5], "max": [20, 87.5, 20]}},
    {{"bufferView": 1, "componentType": 5123, "count": 12, "type": "SCALAR"}}
  ]
}}"#,
            len = bin.len()
        );
        let path = dir.join("avatar.gltf");
        std::fs::write(&path, json).unwrap();

        let mesh = load_avatar(&path).unwrap();
        assert_eq!(mesh.triangle_count(), 4);
        let b = mesh.bounds().unwrap();
        // 85 units * 2 = 170 cm.
        assert!((b.size().y - 1.7).abs() < 1e-4, "{b:?}");
        assert!(mesh.signed_volume() > 0.0);
    }

    #[test]
    fn bad_files_give_errors_not_panics() {
        let dir = temp_dir("bad");
        let txt = dir.join("avatar.txt");
        std::fs::write(&txt, "hello").unwrap();
        assert!(load_avatar(&txt).unwrap_err().contains("Unsupported"));
        assert!(load_avatar(&dir.join("missing.obj")).is_err());
        let broken = dir.join("broken.gltf");
        std::fs::write(&broken, "{ not json").unwrap();
        assert!(load_avatar(&broken).is_err());
        let empty = dir.join("empty.obj");
        std::fs::write(&empty, "v 0 0 0\n").unwrap();
        assert!(load_avatar(&empty).is_err());
    }
}
