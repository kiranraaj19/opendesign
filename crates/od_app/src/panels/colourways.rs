//! Colourways in Properties: named colour variants of the whole garment
//! (see [`od_core::colourway`]), switched with one click, each switch one
//! undo step.

use od_core::colourway::{
    self, AddColourway, ApplyColourway, RemoveColourway, RenameColourway, UpdateColourway,
};

use crate::app::Session;

/// A colourway name being typed: which one, and the text.
type PendingName = Option<(usize, String)>;

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    if s.doc.patterns.is_empty() {
        return;
    }
    ui.add_space(8.0);
    egui::CollapsingHeader::new("Colourways")
        .id_salt("colourways")
        .default_open(true)
        .show(ui, |ui| section(ui, s));
}

fn section(ui: &mut egui::Ui, s: &mut Session) {
    let active = s
        .doc
        .active_colourway
        .filter(|&i| i < s.doc.colourways.len());
    if s.doc.colourways.is_empty() {
        ui.weak("Save the garment's colours as a colourway, then recolour it for another.");
    }
    let mut switch = None;
    ui.horizontal_wrapped(|ui| {
        for (i, c) in s.doc.colourways.iter().enumerate() {
            let shown = active == Some(i);
            let response = ui
                .selectable_label(shown, &c.name)
                .on_hover_text("Show the garment in these colours");
            if response.clicked() && !(shown && c.is_shown(&s.doc)) {
                switch = Some(i);
            }
        }
    });
    if let Some(i) = switch {
        s.execute(ApplyColourway::new(&s.doc, i));
    }

    ui.horizontal_wrapped(|ui| {
        if ui
            .button("New colourway")
            .on_hover_text("Save the colours the patterns have now as a new colourway")
            .clicked()
        {
            let name = colourway::next_name(&s.doc);
            s.execute(AddColourway::new(&s.doc, name));
        }
        let Some(i) = active else {
            return;
        };
        let changed = !s.doc.colourways[i].is_shown(&s.doc);
        if ui
            .add_enabled(changed, egui::Button::new("Save colours"))
            .on_hover_text("Store the colours the patterns have now in this colourway")
            .on_disabled_hover_text("The patterns already show this colourway")
            .clicked()
        {
            s.execute(UpdateColourway::new(i));
        }
        if ui.button("Delete colourway").clicked() {
            s.execute(RemoveColourway::new(i));
        }
    });

    if let Some(i) = active.filter(|&i| i < s.doc.colourways.len()) {
        ui.horizontal(|ui| {
            let id = egui::Id::new("colourway_name");
            let current = s.doc.colourways[i].name.clone();
            let mut pending: PendingName = ui.data(|d| d.get_temp(id)).flatten();
            let name = match &mut pending {
                Some((index, name)) if *index == i => name,
                other => &mut other.insert((i, current.clone())).1,
            };
            let label = ui.label("Name");
            let response = ui
                .add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
                .labelled_by(label.id);
            let typed = name.trim().to_owned();
            if response.lost_focus() && !typed.is_empty() && typed != current {
                s.execute(RenameColourway::new(i, typed));
            }
            // Only kept while typing, so an undo shows straight away.
            if !response.has_focus() {
                pending = None;
            }
            ui.data_mut(|d| d.insert_temp(id, pending));
        });
    }
}
