//! UI tests for the workspace tools: align and distribute, groups, saved
//! camera views and preferences. A child of `app`, so it can reach the
//! app's private state the way `app::tests` does.

use super::*;
use egui_kittest::{
    kittest::{By, NodeT, Queryable},
    Harness,
};
use od_core::geometry::rectangle;

fn harness() -> Harness<'static, OpenDesignApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 880.0))
        .build_eframe(|cc| OpenDesignApp::new(cc));
    h.run();
    h
}

fn click(h: &mut Harness<'_, OpenDesignApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn press(h: &mut Harness<'_, OpenDesignApp>, mods: Modifiers, key: Key) {
    h.key_press_modifiers(mods, key);
    h.run();
}

/// Bring `tab` to the front of its dock group.
fn show_tab(h: &mut Harness<'_, OpenDesignApp>, tab: Tab) {
    let dock = &mut h.state_mut().dock;
    let found = dock.find_tab(&tab).unwrap();
    dock.set_active_tab(found).unwrap();
    h.run();
}

/// Three rectangles of different sizes, scattered, none selected.
fn three_pieces(h: &mut Harness<'_, OpenDesignApp>) -> [PatternId; 3] {
    let s = &mut h.state_mut().session;
    for (name, x, y, w, ht) in [
        ("Wide", 0.0, 0.0, 100.0, 50.0),
        ("Tall", 300.0, 80.0, 60.0, 120.0),
        ("Small", 150.0, -40.0, 40.0, 40.0),
    ] {
        s.add_pattern(name, rectangle(Vec2::new(x, y), Vec2::new(x + w, y + ht)));
    }
    s.selection.clear();
    let ids = [0, 1, 2].map(|i| s.doc.patterns[i].id);
    h.run();
    ids
}

/// The text field holding `value`.
fn text_field<'h>(h: &'h Harness<'_, OpenDesignApp>, value: &'h str) -> egui_kittest::Node<'h> {
    h.get(
        By::new()
            .role(egui::accesskit::Role::TextInput)
            .value(value),
    )
}

fn bounds(h: &Harness<'_, OpenDesignApp>, id: PatternId) -> (Vec2, Vec2) {
    h.state().session.doc.pattern(id).unwrap().bounds().unwrap()
}

fn last_step(h: &Harness<'_, OpenDesignApp>) -> String {
    h.state().session.history.undo_label().unwrap_or_default()
}

// ---- Align and distribute ----

#[test]
fn properties_align_and_distribute_the_selection_one_step_each() {
    let mut h = harness();
    let ids = three_pieces(&mut h);
    show_tab(&mut h, Tab::Properties);
    // One piece: no align buttons.
    h.state_mut().session.selection.set(Some(ids[0]));
    h.run();
    assert!(h.query_by_label("Align left").is_none());

    press(&mut h, Modifiers::COMMAND, Key::A);
    click(&mut h, "Align top");
    assert!(ids.iter().all(|&id| bounds(&h, id).0.y == -40.0));
    assert_eq!(last_step(&h), "Align top");
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(bounds(&h, ids[1]).0.y, 80.0);

    click(&mut h, "Distribute across");
    assert_eq!(last_step(&h), "Distribute across");
    assert!((bounds(&h, ids[2]).0.x - 180.0).abs() < 1e-3);

    // With two selected, distributing is off.
    h.state_mut().session.selection.set_all([ids[0], ids[1]]);
    h.run();
    assert!(h
        .get_by_label("Distribute down")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn the_right_click_menu_aligns_the_selection() {
    let mut h = harness();
    let ids = three_pieces(&mut h);
    // Properties has the same buttons; keep them out of the way.
    show_tab(&mut h, Tab::History);
    press(&mut h, Modifiers::COMMAND, Key::A);
    let at = h.state().session.canvas.screen_pos(Vec2::new(50.0, 25.0));
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run();
    // Right-clicking a selected piece keeps the whole selection.
    assert_eq!(h.state().session.selection.len(), 3);
    click(&mut h, "Align ⏵");
    click(&mut h, "Align right");
    assert!(ids.iter().all(|&id| bounds(&h, id).1.x == 360.0));
}

// ---- Groups ----

#[test]
fn a_group_is_made_from_the_selection_and_clicking_it_selects_it() {
    let mut h = harness();
    let ids = three_pieces(&mut h);
    show_tab(&mut h, Tab::Objects);
    assert!(h
        .get_by_label("Group selected")
        .accesskit_node()
        .is_disabled());
    h.state_mut().session.selection.set_all([ids[0], ids[2]]);
    h.run();
    click(&mut h, "Group selected");
    assert_eq!(last_step(&h), "Add Group 1");
    let group = &h.state().session.doc.groups[0];
    assert_eq!(group.members, [ids[0], ids[2]]);

    h.state_mut().session.selection.set(Some(ids[1]));
    h.run();
    click(&mut h, "Group 1 (2)");
    assert_eq!(h.state().session.selection.ids(), [ids[0], ids[2]]);
    // Shift-click adds the group to what is selected.
    h.state_mut().session.selection.set(Some(ids[1]));
    h.run();
    h.get_by_label("Group 1 (2)")
        .click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().session.selection.len(), 3);
}

#[test]
fn a_group_survives_deleting_and_undoing_its_pieces() {
    let mut h = harness();
    let ids = three_pieces(&mut h);
    show_tab(&mut h, Tab::Objects);
    press(&mut h, Modifiers::COMMAND, Key::A);
    click(&mut h, "Group selected");
    h.state_mut().session.selection.set(Some(ids[1]));
    h.run();
    press(&mut h, Modifiers::NONE, Key::Delete);
    // The deleted piece no longer counts...
    click(&mut h, "Group 1 (2)");
    assert_eq!(h.state().session.selection.ids(), [ids[0], ids[2]]);
    // ...and is back in the group when the delete is undone.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    click(&mut h, "Group 1 (3)");
    assert_eq!(h.state().session.selection.len(), 3);
    // Undoing the group itself removes it.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().session.doc.groups.is_empty());
    assert!(h.query_by_label("Group 1 (3)").is_none());
}

#[test]
fn the_group_menu_renames_refills_and_ungroups() {
    let mut h = harness();
    let ids = three_pieces(&mut h);
    show_tab(&mut h, Tab::Objects);
    h.state_mut().session.selection.set(Some(ids[0]));
    h.run();
    click(&mut h, "Group selected");

    h.get_by_label("Group 1 (1)").click_secondary();
    h.run();
    let field = text_field(&h, "Group 1");
    field.focus();
    field.type_text(" front");
    h.run();
    click(&mut h, "Rename");
    assert_eq!(h.state().session.doc.groups[0].name, "Group 1 front");
    assert_eq!(last_step(&h), "Rename group to Group 1 front");

    press(&mut h, Modifiers::COMMAND, Key::A);
    h.get_by_label("Group 1 front (1)").click_secondary();
    h.run();
    click(&mut h, "Set to selection");
    assert_eq!(h.state().session.doc.groups[0].members.len(), 3);

    h.get_by_label("Group 1 front (3)").click_secondary();
    h.run();
    click(&mut h, "Ungroup");
    assert!(h.state().session.doc.groups.is_empty());
    assert_eq!(h.state().session.doc.patterns.len(), 3, "the pieces stay");
}

// ---- Saved camera views ----

#[test]
fn a_saved_view_brings_the_camera_back_and_is_saved_with_the_project() {
    let mut h = harness();
    click(&mut h, "Top");
    let top = h.state().session.viewport.camera;
    click(&mut h, "Saved views");
    assert!(h.query_by_label("No saved views yet.").is_some());
    let field = text_field(&h, "View 1");
    field.focus();
    field.type_text(" from above");
    h.run();
    click(&mut h, "Save view");
    let s = &h.state().session;
    assert_eq!(s.doc.camera_views.len(), 1);
    assert_eq!(s.doc.camera_views[0].name, "View 1 from above");
    assert_eq!(last_step(&h), "Save view View 1 from above");
    assert!(s.history.is_dirty(), "a saved view is part of the project");

    // Look from the front, then jump back.
    h.key_press(Key::Escape);
    h.run();
    click(&mut h, "Front");
    assert_ne!(h.state().session.viewport.camera, top);
    click(&mut h, "Saved views");
    click(&mut h, "View 1 from above");
    assert_eq!(h.state().session.viewport.camera, top);

    // It round-trips through a project file.
    let json = od_core::project::to_json(&h.state().session.doc);
    let loaded = od_core::project::from_json(&json).unwrap();
    assert_eq!(loaded.camera_views, h.state().session.doc.camera_views);

    // Deleting it is one undo step.
    click(&mut h, "Saved views");
    click(&mut h, "Delete");
    assert!(h.state().session.doc.camera_views.is_empty());
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().session.doc.camera_views.len(), 1);
}

#[test]
fn saving_a_view_under_an_existing_name_replaces_it() {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    crate::panels::views::save(s, "Side");
    s.viewport.camera.orbit(0.7, 0.0);
    crate::panels::views::save(s, "Side");
    assert_eq!(s.doc.camera_views.len(), 1);
    assert_eq!(s.doc.camera_views[0].yaw, s.viewport.camera.yaw);
    assert_eq!(s.history.undo_label().as_deref(), Some("Update view Side"));
}

// ---- Preferences ----

use crate::prefs::{restore as restore_preferences, Preferences};

/// In-memory stand-in for eframe's on-disk storage.
#[derive(Default)]
struct MemStorage(std::collections::HashMap<String, String>);

impl eframe::Storage for MemStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

#[test]
fn preferences_open_from_the_edit_menu_and_restore_defaults() {
    let mut h = harness();
    click(&mut h, "Edit");
    click(&mut h, "Preferences…");
    for label in [
        "Grid size",
        "Seam allowance",
        "Particle distance",
        "Tolerance",
        "Theme",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let s = &mut h.state_mut().session;
    s.canvas.grid_mm = 25.0;
    s.defaults.seam_allowance = 15.0;
    s.seam_check.enabled = false;
    h.run();
    click(&mut h, "Restore defaults");
    let s = &h.state().session;
    assert_eq!(Preferences::capture(s), Preferences::default());
}

#[test]
fn preferences_are_remembered_between_sessions() {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.canvas.grid_mm = 5.0;
    s.canvas.snap.points = false;
    s.defaults.seam_allowance = 12.5;
    s.defaults.particle_distance = 15.0;
    s.seam_check.percent = 1.5;
    let saved = Preferences::capture(s);
    let mut storage = MemStorage::default();
    eframe::App::save(h.state_mut(), &mut storage);

    let mut next = harness();
    restore_preferences(Some(&storage)).apply(&mut next.state_mut().session);
    assert_eq!(Preferences::capture(&next.state().session), saved);
    // Nothing saved, or something unreadable: the defaults.
    assert_eq!(restore_preferences(None), Preferences::default());
    eframe::Storage::set_string(&mut storage, crate::prefs::PREFS_KEY, "nonsense".into());
    assert_eq!(restore_preferences(Some(&storage)), Preferences::default());
}

#[test]
fn new_pieces_get_the_preferred_seam_allowance_and_grid() {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.defaults.seam_allowance = 6.0;
    s.add_pattern("Patch", rectangle(Vec2::ZERO, Vec2::new(50.0, 50.0)));
    assert_eq!(s.doc.patterns[0].seam_allowance, 6.0);
    s.canvas.grid_mm = 25.0;
    assert_eq!(s.canvas.grid_step() % 25.0, 0.0);
}

// ---- Screenshots ----

/// Screenshots of align in Properties, groups in Objects and the
/// Preferences window, written to `$OD_SHOTS` (default: a temporary
/// folder). Needs a wgpu adapter; a software one (lavapipe) is enough.
/// The 3D tab (and its Saved views menu) is left out, as in
/// `app::tests::screenshots_of_darts_pleats_and_linked_copies`.
#[test]
#[ignore = "writes screenshots; run with --ignored"]
fn screenshots_of_the_workspace_tools() {
    let dir = std::env::var_os("OD_SHOTS")
        .map_or_else(|| std::env::temp_dir().join("od_shots"), Into::into);
    std::fs::create_dir_all(&dir).unwrap();
    let shot = |h: &mut Harness<'_, OpenDesignApp>, name: &str| {
        h.run();
        let image = h.render().expect("a wgpu adapter to render with");
        image.save(dir.join(name)).unwrap();
    };
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 880.0))
        .wgpu()
        .build_eframe(|cc| OpenDesignApp::new(cc));
    let dock = &mut h.state_mut().dock;
    if let Some(found) = dock.find_tab(&Tab::Viewport3D) {
        dock.remove_tab(found);
    }
    h.run();
    let ids = three_pieces(&mut h);
    press(&mut h, Modifiers::COMMAND, Key::A);
    press(&mut h, Modifiers::NONE, Key::F);
    show_tab(&mut h, Tab::Properties);
    shot(&mut h, "align-before.png");
    click(&mut h, "Align top");
    click(&mut h, "Distribute across");
    shot(&mut h, "align-after.png");

    show_tab(&mut h, Tab::Objects);
    h.state_mut().session.selection.set_all([ids[0], ids[2]]);
    h.run();
    click(&mut h, "Group selected");
    press(&mut h, Modifiers::COMMAND, Key::A);
    click(&mut h, "Group selected");
    click(&mut h, "Group 1 (2)");
    h.get_by_label("Group 2 (3)").click_secondary();
    shot(&mut h, "groups.png");
    h.key_press(Key::Escape);
    h.run();

    click(&mut h, "Edit");
    click(&mut h, "Preferences…");
    shot(&mut h, "preferences.png");
}
