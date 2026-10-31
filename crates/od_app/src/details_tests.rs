//! UI tests for garment details: the Zipper tool, zippers in Properties
//! and over the 3D view.

use egui::{Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use od_core::{PatternId, SetZipper, Vec2, Zipper};
use od_sim::details::EdgeKind;

use super::{OpenDesignApp, Tool};
use crate::panels::details3d;

fn harness() -> Harness<'static, OpenDesignApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 880.0))
        .build_eframe(|cc| OpenDesignApp::new(cc));
    h.run();
    h
}

fn press(h: &mut Harness<'_, OpenDesignApp>, mods: Modifiers, key: Key) {
    h.key_press_modifiers(mods, key);
    h.run();
}

fn click(h: &mut Harness<'_, OpenDesignApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

/// Press and release the primary button at doc point (`x`, `y`) mm.
fn click_mm(h: &mut Harness<'_, OpenDesignApp>, x: f32, y: f32) {
    let pos = h.state().session.canvas.screen_pos(Vec2::new(x, y));
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
}

fn last_step(h: &Harness<'_, OpenDesignApp>) -> String {
    h.state().session.history.undo_label().unwrap_or_default()
}

/// Two 100 × 300 pieces side by side, 50 mm apart.
fn two_pieces(h: &mut Harness<'_, OpenDesignApp>) -> [PatternId; 2] {
    let s = &mut h.state_mut().session;
    s.add_pattern(
        "Left",
        od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 300.0)),
    );
    s.add_pattern(
        "Right",
        od_core::geometry::rectangle(Vec2::new(150.0, 0.0), Vec2::new(250.0, 300.0)),
    );
    s.selection.clear();
    let ids = [s.doc.patterns[0].id, s.doc.patterns[1].id];
    h.run();
    ids
}

/// Zip Left's right edge to Right's left edge with the Zipper tool.
fn zip_side_seam(h: &mut Harness<'_, OpenDesignApp>) {
    press(h, Modifiers::NONE, Key::Z);
    assert_eq!(h.state().session.tool, Tool::Zipper);
    click_mm(h, 100.0, 40.0);
    assert!(h.state().session.canvas.details.zipper_first.is_some());
    click_mm(h, 150.0, 40.0);
}

#[test]
fn the_zipper_tool_zips_two_edges_and_opens_and_closes_the_zipper() {
    let mut h = harness();
    let [left, right] = two_pieces(&mut h);
    zip_side_seam(&mut h);
    let doc = h.state().session.doc.clone();
    assert_eq!(doc.sewing_lines.len(), 1);
    let line = &doc.sewing_lines[0];
    assert_eq!(line.details.zipper, Some(Zipper::default()));
    assert_eq!((line.a[0].pattern, line.b[0].pattern), (left, right));
    // Picked near the top of both, so they zip untwisted.
    assert_eq!(line.a[0].forward, !line.b[0].forward);
    assert_eq!(last_step(&h), "Add zipper");
    assert_eq!(h.state().session.selected_seam, Some(line.id));

    // Clicking the zipper opens it; again closes it.
    click_mm(&mut h, 150.0, 200.0);
    assert!(!h.state().session.doc.sewing_lines[0].holds_closed());
    assert_eq!(last_step(&h), "Open zipper");
    click_mm(&mut h, 100.0, 200.0);
    assert!(h.state().session.doc.sewing_lines[0].holds_closed());
    assert_eq!(last_step(&h), "Close zipper");

    // Esc drops a half-placed zipper.
    click_mm(&mut h, 50.0, 0.0);
    assert!(h.state().session.canvas.details.zipper_first.is_some());
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().session.canvas.details.zipper_first.is_none());

    press(&mut h, Modifiers::COMMAND, Key::Z);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().session.doc.sewing_lines.is_empty());
}

#[test]
fn a_seams_zipper_is_set_in_properties() {
    let mut h = harness();
    two_pieces(&mut h);
    {
        let s = &mut h.state_mut().session;
        let a = s.doc.patterns[0].shorter_edge(1.0, 2.0);
        let b = s.doc.patterns[1].shorter_edge(3.0, 4.0).reversed();
        s.execute(od_core::AddSewingLine::new(vec![a], vec![b]));
        s.selected_seam = Some(s.doc.sewing_lines[0].id);
    }
    h.run();
    click(&mut h, "Make zipper");
    assert!(h.state().session.doc.sewing_lines[0].is_zipper());
    click(&mut h, "Closed");
    assert!(!h.state().session.doc.sewing_lines[0].holds_closed());
    assert_eq!(last_step(&h), "Open zipper");
    click(&mut h, "Plain seam");
    assert!(!h.state().session.doc.sewing_lines[0].is_zipper());
    assert_eq!(
        h.state().session.doc.sewing_lines.len(),
        1,
        "the seam stays"
    );
}

#[test]
fn zippers_are_drawn_over_the_3d_view_and_open_while_draping() {
    let mut h = harness();
    two_pieces(&mut h);
    zip_side_seam(&mut h);
    h.state_mut().session.tool = Tool::Select;
    h.run();
    let pal = crate::theme::palette(true);
    let flat = details3d::overlays(&h.state().session, pal);
    assert_eq!(flat.len(), 2, "one line per side");
    assert!(flat.iter().all(|o| !o.dashed && o.points.len() >= 2));

    // A drape built as the Drape button builds one, not stepped.
    {
        let s = &mut h.state_mut().session;
        let placements = crate::panels::viewport3d::placements(s);
        let drape = od_sim::drape::Drape::new(
            &s.doc,
            &placements,
            Some(s.viewport.avatar.mesh()),
            &Default::default(),
        );
        s.viewport.drape = Some(drape);
    }
    h.run();
    let closed = |h: &Harness<'_, OpenDesignApp>| {
        let s = &h.state().session;
        let drape = s.viewport.drape.as_ref().unwrap();
        drape
            .edge_detail_positions()
            .iter()
            .map(|(kind, _)| matches!(kind, EdgeKind::Zipper { closed: true, .. }))
            .collect::<Vec<_>>()
    };
    assert_eq!(closed(&h), [true, true]);

    // Opening it in the document opens it in the running drape.
    {
        let s = &mut h.state_mut().session;
        let id = s.doc.sewing_lines[0].id;
        let open = Zipper {
            closed: false,
            ..Zipper::default()
        };
        s.execute(SetZipper::new(id, Some(open)));
    }
    h.run();
    assert_eq!(closed(&h), [false, false]);
    let draped = details3d::overlays(&h.state().session, pal);
    assert!(draped.iter().all(|o| o.dashed));
}

/// Screenshots of the garment details for a look by eye, written to
/// `$OD_SHOTS` (default: a temporary folder). Needs a wgpu adapter; a
/// software one (lavapipe) is enough.
#[test]
#[ignore = "writes screenshots; run with --ignored"]
fn screenshots_of_garment_details() {
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
    // The 3D view draws with a depth buffer the test renderer's pass
    // doesn't have; these shots are of the 2D window and Properties.
    let dock = &mut h.state_mut().dock;
    if let Some(found) = dock.find_tab(&super::Tab::Viewport3D) {
        dock.remove_tab(found);
    }
    h.run();
    two_pieces(&mut h);
    h.state_mut().session.canvas.snap.grid = false;
    press(&mut h, Modifiers::COMMAND, Key::A);
    press(&mut h, Modifiers::NONE, Key::F);
    h.state_mut().session.selection.clear();
    h.run();

    // A zipper between the two pieces, selected, with its Properties.
    zip_side_seam(&mut h);
    shot(&mut h, "zipper.png");
    click_mm(&mut h, 100.0, 200.0);
    shot(&mut h, "zipper-open.png");
    eprintln!("screenshots in {}", dir.display());
}
