//! Importing other tools' files: DXF-AAMA pattern pieces into the current
//! document, via an open-file dialog or dropped onto the window, and a
//! garment model (OBJ or glTF) shown in the 3D view for reference.
//!
//! Unlike `files.rs`'s project open, a DXF import adds to the document (as
//! one undo step) rather than replacing it, so nothing here changes which
//! file the document is saved to.
//!
//! A garment model is a finished 3D surface: it has no flat pieces or
//! seams to simulate, and flattening it back into pieces is a project of
//! its own. So it comes in as reference geometry, drawn see-through around
//! the avatar to compare the drape with. Like a loaded avatar, it is not
//! part of the document and not saved with it, and the cloth doesn't
//! collide with it.

use std::path::{Path, PathBuf};

use od_core::{dxf_import, Vec2};
use od_render::mesh::Mesh;
use od_render::scene::{Color, Frame};

use crate::app::Session;

/// A garment model shown in the 3D view, not simulated.
pub struct Reference {
    /// The file name, for the menu.
    pub name: String,
    /// Metres, where the file put it.
    pub mesh: Mesh,
    pub shown: bool,
}

/// How solid the reference garment is drawn, 0 to 1.
const REFERENCE_ALPHA: f32 = 0.45;

/// Ask for a garment model, then show it in the 3D view. `None` if the
/// dialog was cancelled or the model loaded; an error message otherwise.
pub fn import_reference_dialog(s: &mut Session) -> Option<String> {
    let path = rfd::FileDialog::new()
        .add_filter("Garment (OBJ, glTF)", &od_render::import::EXTENSIONS)
        .pick_file()?;
    import_reference(s, &path).err()
}

/// Show the garment model at `path` in the 3D view, replacing any other.
pub fn import_reference(s: &mut Session, path: &Path) -> Result<(), String> {
    let mesh = od_render::import::load_reference(path)
        .map_err(|e| format!("Couldn't import {}:\n{e}", path.display()))?;
    let name = path
        .file_name()
        .map_or("Garment".into(), |n| n.to_string_lossy().into_owned());
    s.viewport.reference = Some(Reference {
        name,
        mesh,
        shown: true,
    });
    Ok(())
}

/// Add the reference garment, if shown, to a frame of the 3D view, in
/// `color` made see-through.
pub fn draw_reference(frame: &mut Frame, reference: Option<&Reference>, color: Color) {
    if let Some(r) = reference.filter(|r| r.shown) {
        let [red, green, blue, _] = color;
        frame.add_translucent(&r.mesh, [red, green, blue, REFERENCE_ALPHA]);
    }
}

/// The Import menu's entries for the reference garment, once one is
/// loaded.
pub fn reference_menu(ui: &mut egui::Ui, s: &mut Session) {
    let mut remove = false;
    if let Some(r) = &mut s.viewport.reference {
        ui.separator();
        ui.checkbox(&mut r.shown, format!("Show {}", r.name));
        remove = ui.button("Remove reference garment").clicked();
    }
    if remove {
        s.viewport.reference = None;
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use od_render::camera::OrbitCamera;
    use od_render::glam::Vec3;
    use od_render::scene::Globals;

    fn triangle() -> Reference {
        Reference {
            name: "shirt.obj".into(),
            mesh: Mesh {
                positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                normals: vec![Vec3::Z; 3],
                indices: vec![0, 1, 2],
            },
            shown: true,
        }
    }

    #[test]
    fn a_shown_reference_is_drawn_see_through() {
        let globals = Globals::new(&OrbitCamera::default(), 1.0, false);
        let mut frame = Frame::new(globals);
        let mut reference = triangle();
        draw_reference(&mut frame, Some(&reference), [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(frame.translucent.len(), 3);
        assert_eq!(frame.translucent_indices, vec![0, 1, 2]);
        assert!(frame.surfaces.is_empty(), "never solid");

        let mut frame = Frame::new(globals);
        reference.shown = false;
        draw_reference(&mut frame, Some(&reference), [1.0, 0.5, 0.0, 1.0]);
        draw_reference(&mut frame, None, [1.0, 0.5, 0.0, 1.0]);
        assert!(frame.translucent.is_empty());
    }
}
