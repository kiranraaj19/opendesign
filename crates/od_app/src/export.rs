//! Exporting the draped garment (OBJ, glTF) and the document's patterns
//! (DXF-AAMA) to interchange formats, via save-file dialogs.
//!
//! Unlike `files.rs`'s project save/open, these are one-way: nothing is
//! read back, and nothing here becomes "the" file for the document (so
//! there's no path to remember, and no dirty-tracking to update).

use std::path::{Path, PathBuf};

use od_render::mesh::Mesh;

use crate::app::Session;

/// The garment mesh to export: every draped pattern piece merged into one
/// mesh. `None` while nothing has been draped — there's no other 3D asset
/// to export, since avatars here are generated or imported for reference,
/// not round-tripped back out.
pub fn garment_mesh(s: &Session) -> Option<Mesh> {
    let drape = s.viewport.drape.as_ref()?;
    let placed = drape.placed_patterns();
    if placed.is_empty() {
        return None;
    }
    Some(Mesh::merged(
        &placed.into_iter().map(|p| p.mesh).collect::<Vec<_>>(),
    ))
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn pick_save(filter_name: &str, ext: &str, default_name: &str) -> Option<PathBuf> {
    let path = rfd::FileDialog::new()
        .add_filter(filter_name, &[ext])
        .set_file_name(format!("{default_name}.{ext}"))
        .save_file()?;
    Some(if has_extension(&path, ext) {
        path
    } else {
        path.with_extension(ext)
    })
}

/// Ask where to save, then export the current garment mesh as OBJ.
/// `None` if there's nothing to export or the dialog was cancelled; an
/// error message otherwise, to show if the write failed.
pub fn export_obj(s: &Session) -> Option<String> {
    let mesh = garment_mesh(s)?;
    let path = pick_save("Wavefront OBJ", "obj", &s.doc.name)?;
    od_render::export::write_obj(&mesh, &path)
        .err()
        .map(|e| format!("Couldn't export {}:\n{e}", path.display()))
}

/// Like [`export_obj`], but as binary glTF (`.glb`).
pub fn export_gltf(s: &Session) -> Option<String> {
    let mesh = garment_mesh(s)?;
    let path = pick_save("glTF binary", "glb", &s.doc.name)?;
    od_render::export::write_gltf(&mesh, &path)
        .err()
        .map(|e| format!("Couldn't export {}:\n{e}", path.display()))
}

/// Ask where to save, then export the document's patterns as DXF-AAMA.
pub fn export_dxf(s: &Session) -> Option<String> {
    let path = pick_save("DXF (AAMA)", "dxf", &s.doc.name)?;
    od_core::dxf::export(&s.doc, &path)
        .err()
        .map(|e| format!("Couldn't export {}:\n{e}", path.display()))
}

/// Ask where to save, then export the pattern pieces and bill of materials
/// (at the size currently being previewed) as a PDF.
pub fn export_pdf(s: &Session) -> Option<String> {
    let path = pick_save("PDF", "pdf", &s.doc.name)?;
    crate::pdf::export(&s.doc, s.preview_steps(), &path)
        .err()
        .map(|e| format!("Couldn't export {}:\n{e}", path.display()))
}
