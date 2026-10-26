//! Properties of the current selection.

use od_core::{
    Axis, Pattern, PatternId, PatternValue, PointKind, RemoveInternalLine, RenamePattern,
    SetPatternValue,
};

use crate::app::Session;

/// Edits in progress, so typing or dragging a value makes one undo step
/// when it is let go rather than one per keystroke or pixel.
#[derive(Default)]
pub struct PropertiesState {
    editing: Option<(PatternId, String)>,
    pending: Option<(PatternId, PatternValue, f32)>,
}

impl PropertiesState {
    /// A value being dragged or typed but not yet committed, so the 2D
    /// window can show it live.
    pub fn preview(&self, id: PatternId, field: PatternValue) -> Option<f32> {
        self.pending
            .filter(|(pid, f, _)| *pid == id && *f == field)
            .map(|(_, _, v)| v)
    }
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

    let curves = pattern
        .outline
        .iter()
        .filter(|p| p.kind == PointKind::Curve)
        .count();
    egui::Grid::new("pattern_props")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Points");
            ui.label(format!("{} ({curves} curve)", pattern.outline.len()));
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
            ui.label("Notches");
            ui.label(pattern.notches.len().to_string());
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
    egui::Grid::new("pattern_values")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Seam allowance");
            value_edit(ui, s, id, PatternValue::SeamAllowance, " mm", 0.0..=100.0);
            ui.end_row();
            ui.label("Grainline");
            value_edit(ui, s, id, PatternValue::GrainAngle, "°", -180.0..=180.0);
            ui.end_row();
        });

    if let Some(pattern) = s.doc.pattern(id) {
        point_section(ui, s, pattern.clone());
    }
    lines_section(ui, s, id);

    ui.add_space(8.0);
    ui.label("Symmetry");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Flip left-right").clicked() {
            s.mirror_selected(Axis::Vertical);
        }
        if ui.button("Flip up-down").clicked() {
            s.mirror_selected(Axis::Horizontal);
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Mirrored copy")
            .on_hover_text("Add a mirror image beside this pattern, like the other half of a fold")
            .clicked()
        {
            s.duplicate_selected(Some(Axis::Vertical));
        }
        if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
            s.duplicate_selected(None);
        }
    });

    ui.add_space(8.0);
    if ui.button("Delete pattern").clicked() {
        s.canvas.selected_point = None;
        s.delete_selected();
    }
}

/// A numeric property that commits one command when dragging or typing
/// ends.
fn value_edit(
    ui: &mut egui::Ui,
    s: &mut Session,
    id: PatternId,
    field: PatternValue,
    suffix: &str,
    range: std::ops::RangeInclusive<f32>,
) {
    let Some(current) = s.doc.pattern(id).map(|p| field.get(p)) else {
        return;
    };
    let mut value = s.props.preview(id, field).unwrap_or(current);
    let response = ui.add(
        egui::DragValue::new(&mut value)
            .range(range)
            .suffix(suffix)
            .speed(0.5)
            .max_decimals(1),
    );
    if response.changed() {
        s.props.pending = Some((id, field, value));
    }
    let busy = response.dragged() || response.has_focus();
    if !busy {
        if let Some(value) = s.props.preview(id, field) {
            s.props.pending = None;
            if value != current {
                s.execute(SetPatternValue::new(id, field, value));
            }
        }
    }
}

/// The point picked with the Edit points tool.
fn point_section(ui: &mut egui::Ui, s: &mut Session, pattern: Pattern) {
    let Some((pid, index)) = s.canvas.selected_point else {
        return;
    };
    let Some(point) = pattern.outline.get(index).filter(|_| pid == pattern.id) else {
        return;
    };
    ui.add_space(8.0);
    ui.label(format!(
        "Point {} at {:.1}, {:.1} mm",
        index + 1,
        point.pos.x,
        point.pos.y
    ));
    ui.horizontal(|ui| {
        let toggle = match point.kind {
            PointKind::Corner => "Make curve point",
            PointKind::Curve => "Make corner point",
        };
        if ui.button(toggle).on_hover_text("C").clicked() {
            s.toggle_selected_point();
        }
        if ui
            .add_enabled(pattern.outline.len() > 3, egui::Button::new("Delete point"))
            .clicked()
        {
            s.delete_selected();
        }
    });
}

fn lines_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(pattern) = s.doc.pattern(id) else {
        return;
    };
    if pattern.internal_lines.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label("Internal lines");
    let mut remove = None;
    for (i, line) in pattern.internal_lines.iter().enumerate() {
        let length: f32 = line.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        ui.horizontal(|ui| {
            ui.label(format!("Line {}: {length:.0} mm", i + 1));
            if ui.button(format!("Remove line {}", i + 1)).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.execute(RemoveInternalLine::new(id, i));
    }
}
