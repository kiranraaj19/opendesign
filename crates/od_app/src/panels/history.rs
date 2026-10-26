//! Undo history: applied steps, then undone steps that can still be redone.

use crate::app::Session;

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(s.history.can_undo(), egui::Button::new("Undo"))
            .clicked()
        {
            s.undo();
        }
        if ui
            .add_enabled(s.history.can_redo(), egui::Button::new("Redo"))
            .clicked()
        {
            s.redo();
        }
    });
    ui.separator();

    let done = s.history.done_labels();
    let undone = s.history.undone_labels();
    if done.is_empty() && undone.is_empty() {
        ui.weak("No changes yet.");
        return;
    }
    for (i, label) in done.iter().enumerate() {
        let current = i + 1 == done.len();
        ui.label(if current {
            format!("▶ {label}")
        } else {
            format!("   {label}")
        });
    }
    for label in &undone {
        ui.weak(format!("   {label}"));
    }
}
