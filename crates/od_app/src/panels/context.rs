//! The right-click menu shared by the 2D and 3D views.

use od_core::{Axis, PatternFlag};

use crate::app::Session;

/// Right-clicking a pattern that isn't selected selects just it, so the
/// menu acts on what was clicked.
pub fn select_for_menu(s: &mut Session, hit: Option<od_core::PatternId>) {
    match hit {
        Some(id) if !s.selection.contains(id) => {
            s.selection.set(Some(id));
            s.selected_seam = None;
            s.selected_tack = None;
        }
        Some(_) => {}
        None => s.selection.clear(),
    }
}

/// The menu's items. Each acts on the selection, as the Edit menu and
/// Properties do.
pub fn menu(ui: &mut egui::Ui, s: &mut Session) {
    let any = s.selection.primary().is_some();
    let pasting = s.clipboard.is_some();
    if !any {
        if ui
            .add_enabled(pasting, egui::Button::new("Paste"))
            .clicked()
        {
            s.paste(None);
        }
        if ui.button("Select all").clicked() {
            s.select_all();
        }
        return;
    }
    let item = |label, keys| egui::Button::new(label).shortcut_text(keys);
    if ui.add(item("Cut", "Ctrl+X")).clicked() {
        s.cut_selected();
    }
    if ui.add(item("Copy", "Ctrl+C")).clicked() {
        s.copy_selected();
    }
    if ui.add_enabled(pasting, item("Paste", "Ctrl+V")).clicked() {
        s.paste(None);
    }
    if ui.add(item("Duplicate", "Ctrl+D")).clicked() {
        s.duplicate_selected(None);
    }
    ui.separator();
    let ids = s.selection.ids().to_vec();
    if ui.button("Hide").clicked() {
        s.set_flag(ids.clone(), PatternFlag::Hidden, true);
    }
    let locked = ids
        .iter()
        .all(|&id| s.doc.pattern(id).is_some_and(|p| p.locked));
    if locked {
        if ui.button("Unlock").clicked() {
            s.set_flag(ids, PatternFlag::Locked, false);
        }
    } else if ui.button("Lock").clicked() {
        s.set_flag(ids, PatternFlag::Locked, true);
    }
    ui.separator();
    if ui.button("Rotate 90°").clicked() {
        s.rotate_selected(90.0);
    }
    if ui.button("Flip left-right").clicked() {
        s.mirror_selected(Axis::Vertical);
    }
    if ui.button("Turn 3D +45°").clicked() {
        crate::panels::gizmo::turn_selected(s, 45.0);
    }
    if ui.button("Reset 3D position").clicked() {
        crate::panels::gizmo::reset_selected(s);
    }
    ui.separator();
    if ui.add(item("Zoom to", "F")).clicked() {
        s.frame_selection();
    }
    if ui.add(item("Delete", "Del")).clicked() {
        s.delete_selected();
    }
}
