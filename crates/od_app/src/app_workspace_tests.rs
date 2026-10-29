//! UI tests for the workspace tools: align and distribute, groups, saved
//! camera views and preferences. A child of `app`, so it can reach the
//! app's private state the way `app::tests` does.

use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
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
