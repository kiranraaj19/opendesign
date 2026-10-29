//! Groups in the Objects tab: name the selected pieces as a group, then
//! click the group to select them all again.

use od_core::workspace::{unused_name, Group, SetGroups};
use od_core::PatternId;

use crate::app::Session;

/// Make a group of the selected patterns, named "Group N", as one undo
/// step. Returns its index.
pub fn group_selected(s: &mut Session) -> Option<usize> {
    let members = s.selection.ids().to_vec();
    if members.is_empty() {
        return None;
    }
    let name = unused_name("Group", s.doc.groups.iter().map(|g| g.name.as_str()));
    let mut groups = s.doc.groups.clone();
    groups.push(Group::new(name.clone(), members));
    s.execute(SetGroups::new(format!("Add {name}"), groups));
    Some(s.doc.groups.len() - 1)
}

/// Select the group's pieces that are there and shown; with `adding`, add
/// them to the selection instead.
pub fn select(s: &mut Session, index: usize, adding: bool) {
    let Some(group) = s.doc.groups.get(index) else {
        return;
    };
    let ids: Vec<PatternId> = group
        .members_in(&s.doc)
        .filter(|&id| s.doc.pattern(id).is_some_and(|p| !p.hidden))
        .collect();
    if !adding {
        s.selection.clear();
    }
    for id in ids {
        s.selection.add(id);
    }
    s.selected_seam = None;
    s.selected_tack = None;
}

/// Change one group with `edit`, as one undo step labelled `label`.
fn edit(s: &mut Session, index: usize, label: String, edit: impl FnOnce(&mut Vec<Group>)) {
    if index >= s.doc.groups.len() {
        return;
    }
    let mut groups = s.doc.groups.clone();
    edit(&mut groups);
    if groups != s.doc.groups {
        s.execute(SetGroups::new(label, groups));
    }
}

pub fn rename(s: &mut Session, index: usize, name: &str) {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return;
    }
    let label = format!("Rename group to {name}");
    edit(s, index, label, |g| g[index].name = name);
}

/// Make the group hold exactly the selected patterns.
pub fn set_to_selection(s: &mut Session, index: usize) {
    let members = s.selection.ids().to_vec();
    let Some(name) = s.doc.groups.get(index).map(|g| g.name.clone()) else {
        return;
    };
    if members.is_empty() {
        return;
    }
    edit(s, index, format!("Update {name}"), |g| {
        g[index].members = members
    });
}

/// Remove the group; its pieces stay.
pub fn ungroup(s: &mut Session, index: usize) {
    let Some(name) = s.doc.groups.get(index).map(|g| g.name.clone()) else {
        return;
    };
    edit(s, index, format!("Ungroup {name}"), |g| {
        g.remove(index);
    });
}

/// What a group's right-click menu asked for.
enum Action {
    Rename(String),
    SetToSelection,
    Ungroup,
}

/// The Groups part of the Objects tab.
pub fn section(ui: &mut egui::Ui, s: &mut Session) {
    ui.add_space(8.0);
    ui.strong(format!("Groups ({})", s.doc.groups.len()));
    if s.doc.groups.is_empty() {
        ui.weak("Select pieces and press Group selected to name them as a set.");
    }
    let mut picked = None;
    let mut action = None;
    for (i, group) in s.doc.groups.iter().enumerate() {
        let live: Vec<PatternId> = group.members_in(&s.doc).collect();
        let selected = !live.is_empty() && live.iter().all(|&id| s.selection.contains(id));
        let text = format!("{} ({})", group.name, live.len());
        let response = ui
            .add(egui::Button::selectable(selected, text))
            .on_hover_text("Click to select its pieces; Shift or Ctrl+click to add; right-click to rename or change");
        if response.clicked() {
            picked = Some(i);
        }
        let name_id = ui.id().with(("group_name", i));
        response.context_menu(|ui| {
            let mut name: String = ui
                .data_mut(|d| d.get_temp(name_id))
                .unwrap_or_else(|| group.name.clone());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
                if ui.button("Rename").clicked() {
                    action = Some((i, Action::Rename(name.clone())));
                    ui.close();
                }
            });
            ui.data_mut(|d| d.insert_temp(name_id, name));
            if ui
                .button("Set to selection")
                .on_hover_text("Make the group hold exactly the selected pieces")
                .clicked()
            {
                action = Some((i, Action::SetToSelection));
            }
            if ui
                .button("Ungroup")
                .on_hover_text("Forget the group; its pieces stay")
                .clicked()
            {
                action = Some((i, Action::Ungroup));
            }
        });
        if !response.context_menu_opened() {
            ui.data_mut(|d| d.remove::<String>(name_id));
        }
    }
    if let Some(i) = picked {
        let adding = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        select(s, i, adding);
    }
    match action {
        Some((i, Action::Rename(name))) => rename(s, i, &name),
        Some((i, Action::SetToSelection)) => set_to_selection(s, i),
        Some((i, Action::Ungroup)) => ungroup(s, i),
        None => {}
    }
    if ui
        .add_enabled(
            s.selection.primary().is_some(),
            egui::Button::new("Group selected"),
        )
        .on_hover_text("Name the selected pieces as a group, saved with the project")
        .clicked()
    {
        group_selected(s);
    }
}
