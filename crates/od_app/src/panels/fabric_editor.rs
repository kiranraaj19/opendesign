//! The fabric editor in Properties: a pattern's fabric's name, colour,
//! weave and physical values, each change one undo step.

use od_core::{EditFabric, Fabric, FabricTexture, FabricValue, PatternId, SetFabric};

use crate::app::Session;

/// A value being dragged or typed but not yet committed: pattern, which
/// value, and the value in the editor's units. Kept in egui's memory.
type Pending = Option<(PatternId, FabricValue, f32)>;

/// A colour being picked but not yet committed.
type PendingColor = Option<(PatternId, [u8; 3])>;

/// A name being typed.
type PendingName = Option<(PatternId, String)>;

fn memory_id() -> egui::Id {
    egui::Id::new("fabric_editor")
}

/// The editor for `id`'s fabric, collapsed until opened.
pub fn ui(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(fabric) = s.doc.pattern(id).map(|p| p.fabric.clone()) else {
        return;
    };
    egui::CollapsingHeader::new("Edit fabric")
        .id_salt("edit_fabric")
        .show(ui, |ui| editor(ui, s, id, &fabric));
}

fn editor(ui: &mut egui::Ui, s: &mut Session, id: PatternId, fabric: &Fabric) {
    // Edits made on a linked copy go to its source, which the copy follows.
    let (target, _) = s.edit_target(id);
    let mut commit: Option<(Fabric, String)> = None;

    ui.horizontal(|ui| {
        let name_id = memory_id().with("name");
        let mut pending: PendingName = ui.data(|d| d.get_temp(name_id)).flatten();
        let name = match &mut pending {
            Some((pid, name)) if *pid == id => name,
            other => &mut other.insert((id, fabric.name.clone())).1,
        };
        let label = ui.label("Name");
        let response = ui
            .add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
            .labelled_by(label.id);
        let typed = name.trim().to_owned();
        if response.lost_focus() && !typed.is_empty() && typed != fabric.name {
            let renamed = Fabric {
                name: typed,
                ..fabric.clone()
            };
            commit = Some((renamed, "name".into()));
        }
        // Only kept while typing, so an undo shows straight away.
        if !response.has_focus() {
            pending = None;
        }
        ui.data_mut(|d| d.insert_temp(name_id, pending));
    });

    ui.horizontal(|ui| {
        ui.label("Colour");
        let color_id = memory_id().with("color");
        let pending: PendingColor = ui.data(|d| d.get_temp(color_id)).flatten();
        let mut color = match pending {
            Some((pid, c)) if pid == id => c,
            _ => fabric.color,
        };
        let response = egui::widgets::color_picker::color_edit_button_srgb(ui, &mut color);
        let mut pending = pending.filter(|(pid, _)| *pid == id);
        if response.changed() {
            pending = Some((id, color));
        }
        // Picking drags across the colour square, or types a code: commit
        // once the pointer is up and nothing is being typed in.
        let busy = ui.input(|i| i.pointer.any_down()) || ui.memory(|m| m.focused().is_some());
        if !busy {
            if let Some((_, color)) = pending.take() {
                if color != fabric.color {
                    commit = Some((
                        Fabric {
                            color,
                            ..fabric.clone()
                        },
                        "colour".into(),
                    ));
                }
            }
        }
        ui.data_mut(|d| d.insert_temp(color_id, pending));
    });

    ui.horizontal_wrapped(|ui| {
        ui.label("Weave");
        for (texture, label) in [
            (FabricTexture::Flat, "Smooth"),
            (FabricTexture::Plain, "Plain weave"),
            (FabricTexture::Twill, "Twill"),
        ] {
            let selected = fabric.texture == texture;
            if ui.selectable_label(selected, label).clicked() && !selected {
                commit = Some((
                    Fabric {
                        texture,
                        ..fabric.clone()
                    },
                    "weave".into(),
                ));
            }
        }
    });

    egui::Grid::new("fabric_values")
        .num_columns(2)
        .show(ui, |ui| {
            for value in FabricValue::ALL {
                if let Some(edited) = value_row(ui, id, fabric, value) {
                    commit = Some((edited, value.label().to_lowercase()));
                }
                ui.end_row();
            }
        });
    if !fabric.is_isotropic() {
        ui.weak("Stretches and bends differently along and across the grainline.");
    }

    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Save fabric file…")
            .on_hover_text("Keep this fabric to use in other projects; it's listed in the Library")
            .clicked()
        {
            s.fabrics.save_dialog(fabric);
        }
        if ui
            .button("Load fabric file…")
            .on_hover_text("Use a fabric saved earlier (.odfab) on this pattern")
            .clicked()
        {
            if let Some(loaded) = s.fabrics.load_dialog() {
                commit = None;
                s.execute(SetFabric::new(target, loaded));
            }
        }
    });

    if let Some((edited, what)) = commit {
        s.execute(EditFabric::new(target, edited, what));
    }
}

/// One labelled value; the edited fabric once dragging or typing ends.
fn value_row(
    ui: &mut egui::Ui,
    id: PatternId,
    fabric: &Fabric,
    value: FabricValue,
) -> Option<Fabric> {
    let pending_id = memory_id().with("value");
    let current = value.get(fabric);
    let mut pending: Pending = ui.data(|d| d.get_temp(pending_id)).flatten();
    let mut shown = match pending {
        Some((pid, v, x)) if pid == id && v == value => x,
        _ => current,
    };
    let label = ui.label(value.label()).on_hover_text(value.help());
    let decimals = if value == FabricValue::Friction { 2 } else { 0 };
    let speed = if value == FabricValue::Friction {
        0.01
    } else {
        0.5
    };
    let response = ui
        .add(
            egui::DragValue::new(&mut shown)
                .range(value.range())
                .suffix(value.suffix())
                .speed(speed)
                .max_decimals(decimals),
        )
        .labelled_by(label.id)
        .on_hover_text(value.help());
    if response.changed() {
        pending = Some((id, value, shown));
    }
    let mut edited = None;
    let busy = response.dragged() || response.has_focus();
    if !busy {
        if let Some((pid, v, x)) = pending {
            if pid == id && v == value {
                pending = None;
                if (x - current).abs() > 1e-4 {
                    edited = Some(value.set(fabric, x));
                }
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(pending_id, pending));
    edited
}
