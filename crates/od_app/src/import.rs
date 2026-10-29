//! Importing other tools' files into the current document: DXF-AAMA
//! pattern pieces, via an open-file dialog or dropped onto the window.
//!
//! Unlike `files.rs`'s project open, an import adds to the document (as one
//! undo step) rather than replacing it, so nothing here changes which file
//! the document is saved to.

use std::path::{Path, PathBuf};

use od_core::{dxf_import, Vec2};

use crate::app::Session;

/// Gap between pieces already in the document and imported ones, in mm.
const IMPORT_GAP: f32 = 50.0;

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Ask for a DXF file, then import its pieces. `None` if the dialog was
/// cancelled or the import worked; an error message otherwise.
pub fn import_dxf_dialog(s: &mut Session) -> Option<String> {
    let path = rfd::FileDialog::new()
        .add_filter("DXF (AAMA)", &["dxf"])
        .pick_file()?;
    import_dxf(s, &path).err()
}

/// Add the pieces in the DXF file at `path` as one undo step, select them
/// and fit them in view. If the document already has pieces, the imported
/// ones go to their right instead of on top of them.
pub fn import_dxf(s: &mut Session, path: &Path) -> Result<(), String> {
    let patterns = dxf_import::import(path)
        .map_err(|e| format!("Couldn't import {}:\n{e}", path.display()))?;
    let offset = placement(s, &patterns);
    let name = path
        .file_name()
        .map_or("DXF".into(), |n| n.to_string_lossy().into_owned());
    let count = s.doc.patterns.len();
    s.execute(dxf_import::import_command(
        format!("Import {name}"),
        patterns,
        offset,
    ));
    let ids: Vec<_> = s.doc.patterns[count..].iter().map(|p| p.id).collect();
    s.selection.set_all(ids);
    s.selected_seam = None;
    s.selected_tack = None;
    s.frame_selection();
    Ok(())
}

/// How far to move imported pieces so they sit beside the document's.
fn placement(s: &Session, patterns: &[od_core::Pattern]) -> Vec2 {
    let bounds = |ps: &[od_core::Pattern]| {
        let points: Vec<Vec2> = ps
            .iter()
            .filter_map(|p| p.bounds())
            .flat_map(|(min, max)| [min, max])
            .collect();
        od_core::geometry::bounds(&points)
    };
    match (bounds(&s.doc.patterns), bounds(patterns)) {
        (Some((have_min, have_max)), Some((new_min, _))) => {
            Vec2::new(have_max.x + IMPORT_GAP - new_min.x, have_min.y - new_min.y)
        }
        _ => Vec2::ZERO,
    }
}

/// The first dropped DXF file, if any.
pub fn dropped_dxf(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path())
            .find(|p| has_extension(p, "dxf"))
            .map(Path::to_owned)
    })
}
