//! Properties of the current selection.

use od_core::{PatternId, RenamePattern};

use crate::app::Session;

/// Name being edited, so typing doesn't create one undo step per key.
#[derive(Default)]
pub struct PropertiesState {
    editing: Option<(PatternId, String)>,
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let Some(pattern) = s.selection.and_then(|id| s.doc.pattern(id)) else {
        ui.weak("Nothing selected.");
        ui.add_space(8.0);
        ui.label(format!("Document: {}", s.doc.name));
        ui.label(format!("Patterns: {}", s.doc.patterns.len()));
        return;
    };
    let id = pattern.id;

    ui.heading("Pattern");
    let name = match &mut s.props.editing {
        Some((editing_id, name)) if *editing_id == id => name,
        editing => &mut editing.insert((id, pattern.name.clone())).1,
    };
    let response = ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(name)
    });
    let committed = response.inner.lost_focus();
    let new_name = name.trim().to_owned();

    egui::Grid::new("pattern_props")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Points");
            ui.label(pattern.outline.len().to_string());
            ui.end_row();
            if let Some((min, max)) = pattern.bounds() {
                ui.label("Size");
                ui.label(format!("{:.0} × {:.0} mm", max.x - min.x, max.y - min.y));
                ui.end_row();
            }
            ui.label("Area");
            ui.label(format!("{:.1} cm²", pattern.area() / 100.0));
            ui.end_row();
            ui.label("Perimeter");
            ui.label(format!("{:.0} mm", pattern.perimeter()));
            ui.end_row();
        });

    let unchanged = new_name.is_empty() || new_name == pattern.name;
    if committed {
        s.props.editing = None;
        if !unchanged {
            s.execute(RenamePattern::new(id, new_name));
        }
    }

    ui.add_space(8.0);
    if ui.button("Delete pattern").clicked() {
        s.delete_selected();
    }
}
