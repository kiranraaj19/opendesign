//! Every pattern, seam and tack in the document, to find, select, hide and
//! lock them.

use od_core::{PatternFlag, PatternId};

use crate::app::Session;

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        patterns(ui, s);
        seams_and_tacks(ui, s);
    });
}

fn patterns(ui: &mut egui::Ui, s: &mut Session) {
    ui.strong(format!("Patterns ({})", s.doc.patterns.len()));
    if s.doc.patterns.is_empty() {
        ui.weak("None yet.");
        return;
    }
    let mut picked = None;
    let mut flags = Vec::new();
    egui::Grid::new("objects_patterns")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for p in &s.doc.patterns {
                let label = if p.hidden {
                    egui::RichText::new(&p.name).weak()
                } else {
                    egui::RichText::new(&p.name)
                };
                let selected = s.selection.contains(p.id);
                if ui
                    .add_enabled(!p.hidden, egui::Button::selectable(selected, label))
                    .on_hover_text("Click to select; Shift or Ctrl+click to add")
                    .clicked()
                {
                    picked = Some(p.id);
                }
                let mut shown = !p.hidden;
                if ui.checkbox(&mut shown, "Show").changed() {
                    flags.push((p.id, PatternFlag::Hidden, !shown));
                }
                let mut locked = p.locked;
                if ui.checkbox(&mut locked, "Lock").changed() {
                    flags.push((p.id, PatternFlag::Locked, locked));
                }
                ui.end_row();
            }
        });
    if let Some(id) = picked {
        let adding = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        if adding {
            s.selection.toggle(id);
        } else {
            s.selection.set(Some(id));
        }
        s.selected_seam = None;
        s.selected_tack = None;
    }
    for (id, flag, value) in flags {
        s.set_flag(vec![id], flag, value);
    }

    let selected = s.selection.ids().to_vec();
    let all: Vec<PatternId> = s.doc.patterns.iter().map(|p| p.id).collect();
    ui.horizontal_wrapped(|ui| {
        let any = !selected.is_empty();
        if ui
            .add_enabled(any, egui::Button::new("Hide selected"))
            .clicked()
        {
            s.set_flag(selected.clone(), PatternFlag::Hidden, true);
        }
        if ui.button("Show all").clicked() {
            s.set_flag(all.clone(), PatternFlag::Hidden, false);
        }
        if ui
            .add_enabled(any, egui::Button::new("Lock selected"))
            .clicked()
        {
            s.set_flag(selected.clone(), PatternFlag::Locked, true);
        }
        if ui.button("Unlock all").clicked() {
            s.set_flag(all, PatternFlag::Locked, false);
        }
    });
}

fn seams_and_tacks(ui: &mut egui::Ui, s: &mut Session) {
    let name = |s: &Session, id| {
        s.doc
            .pattern(id)
            .map_or("?", |p| p.name.as_str())
            .to_owned()
    };

    ui.add_space(8.0);
    ui.strong(format!("Seams ({})", s.doc.sewing_lines.len()));
    let mut picked_seam = None;
    for (i, line) in s.doc.sewing_lines.iter().enumerate() {
        let between = |side: &[od_core::SeamEdge]| {
            let mut names: Vec<String> = side.iter().map(|e| name(s, e.pattern)).collect();
            names.dedup();
            names.join(", ")
        };
        let text = format!(
            "Seam {}: {} to {}",
            i + 1,
            between(&line.a),
            between(&line.b)
        );
        if ui
            .selectable_label(s.selected_seam == Some(line.id), text)
            .clicked()
        {
            picked_seam = Some(line.id);
        }
    }
    if let Some(id) = picked_seam {
        s.selected_seam = Some(id);
        s.selected_tack = None;
        s.selection.set(None);
    }

    ui.add_space(8.0);
    ui.strong(format!("Tacks ({})", s.doc.tacks.len()));
    let mut picked_tack = None;
    for (i, tack) in s.doc.tacks.iter().enumerate() {
        let to = match tack.b {
            od_core::TackTarget::Pattern(b) => name(s, b.pattern),
            _ => "avatar".to_owned(),
        };
        let text = format!("Tack {}: {} to {to}", i + 1, name(s, tack.a.pattern));
        if ui
            .selectable_label(s.selected_tack == Some(tack.id), text)
            .clicked()
        {
            picked_tack = Some(tack.id);
        }
    }
    if let Some(id) = picked_tack {
        s.selected_tack = Some(id);
        s.selected_seam = None;
        s.selection.set(None);
    }
}
