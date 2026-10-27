//! Export a mesh (typically the draped garment) to OBJ or glTF, for
//! interop with other 3D tools.
//!
//! glTF is written as binary (`.glb`): a single self-contained file with
//! the JSON document and the vertex/index buffer packed into it, built by
//! hand rather than via the `gltf` crate (which this crate only uses for
//! *reading*, in [`crate::import`]) — the structure is simple enough
//! (one mesh, one primitive, positions + normals + indices) that writing
//! the chunks directly avoids a second dependency's worth of API surface.
//!
//! Positions are written in the mesh's own units (metres, as everywhere
//! else in this crate); no rescaling happens here, unlike
//! [`crate::import::normalize`] which guesses the incoming units.

use std::fs;
use std::io;
use std::path::Path;

use glam::Vec3;

use crate::mesh::Mesh;

/// Write `mesh` as a Wavefront OBJ file: `v`/`vn`/`f` lines, normals
/// omitted if the mesh doesn't have exactly one per vertex.
pub fn write_obj(mesh: &Mesh, path: &Path) -> io::Result<()> {
    let mut out = String::new();
    out.push_str("# Exported from OpenDesign\n");
    for p in &mesh.positions {
        out.push_str(&format!("v {} {} {}\n", p.x, p.y, p.z));
    }
    let has_normals = mesh.normals.len() == mesh.positions.len();
    if has_normals {
        for n in &mesh.normals {
            out.push_str(&format!("vn {} {} {}\n", n.x, n.y, n.z));
        }
    }
    for t in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (t[0] + 1, t[1] + 1, t[2] + 1);
        if has_normals {
            out.push_str(&format!("f {a}//{a} {b}//{b} {c}//{c}\n"));
        } else {
            out.push_str(&format!("f {a} {b} {c}\n"));
        }
    }
    fs::write(path, out)
}

/// Write `mesh` as binary glTF (`.glb`): one buffer holding positions, then
/// normals, then indices, referenced by one mesh with one triangle
/// primitive. Normals are filled with +Y if the mesh doesn't have exactly
/// one per vertex, so the file is always valid.
pub fn write_gltf(mesh: &Mesh, path: &Path) -> io::Result<()> {
    let n = mesh.positions.len();
    let mut bin = Vec::with_capacity(n * 24 + mesh.indices.len() * 4);

    for p in &mesh.positions {
        bin.extend_from_slice(&p.x.to_le_bytes());
        bin.extend_from_slice(&p.y.to_le_bytes());
        bin.extend_from_slice(&p.z.to_le_bytes());
    }
    let pos_len = bin.len();

    let fallback = vec![Vec3::Y; n];
    let normals = if mesh.normals.len() == n {
        &mesh.normals
    } else {
        &fallback
    };
    for v in normals {
        bin.extend_from_slice(&v.x.to_le_bytes());
        bin.extend_from_slice(&v.y.to_le_bytes());
        bin.extend_from_slice(&v.z.to_le_bytes());
    }
    let norm_len = bin.len() - pos_len;

    for &i in &mesh.indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let idx_len = bin.len() - pos_len - norm_len;

    let (min, max) = mesh
        .bounds()
        .map_or((Vec3::ZERO, Vec3::ZERO), |b| (b.min, b.max));
    let json = format!(
        concat!(
            r#"{{"asset":{{"version":"2.0","generator":"OpenDesign"}},"#,
            r#""scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],"#,
            r#""meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1}},"#,
            r#""indices":2,"mode":4}}]}}],"accessors":["#,
            r#"{{"bufferView":0,"componentType":5126,"count":{n},"type":"VEC3","#,
            r#""min":[{minx},{miny},{minz}],"max":[{maxx},{maxy},{maxz}]}},"#,
            r#"{{"bufferView":1,"componentType":5126,"count":{n},"type":"VEC3"}},"#,
            r#"{{"bufferView":2,"componentType":5125,"count":{idx_count},"type":"SCALAR"}}],"#,
            r#""bufferViews":["#,
            r#"{{"buffer":0,"byteOffset":0,"byteLength":{pos_len},"target":34962}},"#,
            r#"{{"buffer":0,"byteOffset":{pos_len},"byteLength":{norm_len},"target":34962}},"#,
            r#"{{"buffer":0,"byteOffset":{pos_norm_len},"byteLength":{idx_len},"target":34963}}],"#,
            r#""buffers":[{{"byteLength":{total}}}]}}"#
        ),
        n = n,
        minx = min.x,
        miny = min.y,
        minz = min.z,
        maxx = max.x,
        maxy = max.y,
        maxz = max.z,
        idx_count = mesh.indices.len(),
        pos_len = pos_len,
        norm_len = norm_len,
        pos_norm_len = pos_len + norm_len,
        idx_len = idx_len,
        total = bin.len(),
    );

    write_glb(path, &json, &bin)
}

/// Pack a JSON chunk and a binary chunk into a `.glb` file, per the glTF 2.0
/// binary container spec: a 12-byte header, then each chunk as a 4-byte
/// length, a 4-byte ASCII type, and the (4-byte-padded) payload.
fn write_glb(path: &Path, json: &str, bin: &[u8]) -> io::Result<()> {
    let mut json_bytes = json.as_bytes().to_vec();
    json_bytes.resize(json_bytes.len().next_multiple_of(4), b' ');
    let mut bin_bytes = bin.to_vec();
    bin_bytes.resize(bin_bytes.len().next_multiple_of(4), 0);

    let total_len = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());

    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);

    out.extend_from_slice(&(bin_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin_bytes);

    fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::tests::cube;

    fn temp_dir(test: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("od_render_export_{}_{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn obj_round_trips_the_triangle_count() {
        let mesh = cube();
        let path = temp_dir("obj").join("cube.obj");
        write_obj(&mesh, &path).unwrap();
        let (models, _materials) = tobj::load_obj(&path, &tobj::GPU_LOAD_OPTIONS).unwrap();
        let count: usize = models.iter().map(|m| m.mesh.indices.len() / 3).sum();
        assert_eq!(count, mesh.triangle_count());
    }

    #[test]
    fn obj_without_normals_omits_vn_but_still_has_faces() {
        let mesh = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![],
            indices: vec![0, 1, 2],
        };
        let path = temp_dir("obj_no_normals").join("tri.obj");
        write_obj(&mesh, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("vn "));
        assert!(content.contains("f 1 2 3"));
    }

    #[test]
    fn gltf_round_trips_positions_normals_and_indices() {
        let mesh = cube();
        let path = temp_dir("gltf").join("cube.glb");
        write_gltf(&mesh, &path).unwrap();

        let (doc, buffers, _images) = gltf::import(&path).unwrap();
        let gltf_mesh = doc.meshes().next().expect("one mesh");
        let primitive = gltf_mesh.primitives().next().expect("one primitive");
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));

        let positions: Vec<Vec3> = reader
            .read_positions()
            .expect("positions")
            .map(Vec3::from)
            .collect();
        assert_eq!(positions.len(), mesh.positions.len());
        for (a, b) in positions.iter().zip(&mesh.positions) {
            assert!(a.distance(*b) < 1e-5);
        }

        let normals: Vec<Vec3> = reader
            .read_normals()
            .expect("normals")
            .map(Vec3::from)
            .collect();
        assert_eq!(normals.len(), mesh.normals.len());

        let indices: Vec<u32> = reader.read_indices().expect("indices").into_u32().collect();
        assert_eq!(indices, mesh.indices);
    }

    #[test]
    fn gltf_write_does_not_panic_on_an_empty_mesh() {
        let mesh = Mesh::default();
        let path = temp_dir("gltf_empty").join("empty.glb");
        write_gltf(&mesh, &path).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }
}
