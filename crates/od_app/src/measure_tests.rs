//! UI tests for measuring: the Measure tool in the 2D window, the
//! Measurements section of Properties, the measurement chart, the graded
//! size overlay and the 3D circumference and tape.

use egui::{Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use od_core::{Grading, MeasureKind, PatternId, SetGrading, Vec2};

use super::{OpenDesignApp, Tab, Tool};
use crate::panels::measure3d::Mode;

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

fn screen(h: &Harness<'_, OpenDesignApp>, x: f32, y: f32) -> egui::Pos2 {
    h.state().session.canvas.screen_pos(Vec2::new(x, y))
}

/// Press and release the primary button at `pos`, holding `mods`.
fn click_at(h: &mut Harness<'_, OpenDesignApp>, pos: egui::Pos2, mods: Modifiers) {
    h.hover_at(pos);
    h.run();
    h.event(egui::Event::ModifiersChanged(mods));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: mods,
        });
        h.run();
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
}

fn click_mm(h: &mut Harness<'_, OpenDesignApp>, x: f32, y: f32, mods: Modifiers) {
    let at = screen(h, x, y);
    click_at(h, at, mods);
}

fn show_tab(h: &mut Harness<'_, OpenDesignApp>, tab: Tab) {
    let dock = &mut h.state_mut().dock;
    let found = dock.find_tab(&tab).unwrap();
    dock.set_active_tab(found).unwrap();
    h.run();
}

/// A 100 × 80 rectangle at the origin, graded 10% per size, framed in
/// the 2D window at the default zoom.
fn panel(h: &mut Harness<'_, OpenDesignApp>) -> PatternId {
    let s = &mut h.state_mut().session;
    s.add_pattern(
        "Panel",
        od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 80.0)),
    );
    let id = s.doc.patterns[0].id;
    let outline = s.doc.patterns[0].outline.clone();
    s.execute(SetGrading::new(id, Grading::uniform_scale(&outline, 10.0)));
    s.selection.clear();
    h.run();
    id
}

fn lengths(h: &Harness<'_, OpenDesignApp>) -> Vec<f32> {
    let doc = &h.state().session.doc;
    doc.measurements
        .iter()
        .map(|m| m.length_at(doc, 0).unwrap())
        .collect()
}

#[test]
fn the_measure_tool_measures_across_and_along_outlines() {
    let mut h = harness();
    panel(&mut h);
    click(&mut h, "Measure");
    assert_eq!(h.state().session.tool, Tool::Measure);

    // Top left corner to top right: straight across.
    click_mm(&mut h, 0.0, 0.0, Modifiers::NONE);
    assert!(h.state().session.canvas.measure.start.is_some());
    click_mm(&mut h, 100.0, 0.0, Modifiers::NONE);
    assert_eq!(lengths(&h), [100.0]);
    assert_eq!(
        h.state().session.history.undo_label().as_deref(),
        Some("Add measurement")
    );
    let m = &h.state().session.doc.measurements[0];
    assert_eq!(m.name, "Measurement 1");
    assert_eq!(h.state().session.canvas.measure.selected, Some(m.id));

    // Shift: along the outline, the shorter way, from the top left corner
    // round the top right one to half way down the right side.
    click_mm(&mut h, 0.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 100.0, 40.0, Modifiers::SHIFT);
    let doc = &h.state().session.doc;
    assert!(matches!(doc.measurements[1].kind, MeasureKind::Along(_)));
    assert!((lengths(&h)[1] - 140.0).abs() < 0.5, "{:?}", lengths(&h));

    // Esc drops a measurement half taken; a click off any outline does
    // nothing.
    click_mm(&mut h, 0.0, 0.0, Modifiers::NONE);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(h.state().session.canvas.measure.start, None);
    click_mm(&mut h, 50.0, 40.0, Modifiers::NONE);
    assert_eq!(h.state().session.canvas.measure.start, None);

    // Undo takes measurements away one at a time.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().session.doc.measurements.len(), 1);
}

#[test]
fn measurements_are_listed_renamed_and_removed_in_properties() {
    let mut h = harness();
    let id = panel(&mut h);
    h.state_mut().session.execute(od_core::AddMeasurement::new(
        Some("Width".into()),
        MeasureKind::Distance {
            a: od_core::OutlineSpot {
                pattern: id,
                param: 0.0,
            },
            b: od_core::OutlineSpot {
                pattern: id,
                param: 1.0,
            },
        },
    ));
    show_tab(&mut h, Tab::Properties);
    h.get_by_label("Measurements");
    h.get_by_label("100.0 mm");

    // The length follows the size previewed.
    click(&mut h, "L");
    h.get_by_label("110.0 mm");

    // Rename by typing a new name.
    let field = h.get_all_by_value("Width").next().unwrap();
    field.click();
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("Bust".into()));
    h.run();
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().session.doc.measurements[0].name, "Bust");
    assert_eq!(
        h.state().session.history.undo_label().as_deref(),
        Some("Rename to Bust")
    );

    click(&mut h, "Remove");
    assert!(h.state().session.doc.measurements.is_empty());
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().session.doc.measurements.len(), 1);
}

#[test]
fn the_chart_shows_every_size_and_the_overlay_draws_them() {
    let mut h = harness();
    panel(&mut h);
    click(&mut h, "Measure");
    click_mm(&mut h, 0.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 100.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 100.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 100.0, 80.0, Modifiers::NONE);

    // View → Measurement chart: S to XL from the graded outlines.
    click(&mut h, "View");
    click(&mut h, "Measurement chart");
    h.get_by_label("Measurement chart");
    // The base size is marked, in the chart as in Properties.
    assert_eq!(h.get_all_by_label("M (base)").count(), 2);
    for length in [
        "90.0 mm", "100.0 mm", "110.0 mm", "120.0 mm", "72.0 mm", "96.0 mm",
    ] {
        assert!(
            h.query_all_by_label(length).next().is_some(),
            "no {length} in the chart"
        );
    }
    click(&mut h, "Copy as CSV");
    assert!(h.state().session.canvas.measure.chart_open);

    // View → Show all sizes, and the same switch in Properties.
    assert!(!h.state().session.canvas.measure.all_sizes);
    click(&mut h, "View");
    click(&mut h, "Show all graded sizes");
    assert!(h.state().session.canvas.measure.all_sizes);
    show_tab(&mut h, Tab::Properties);
    press(&mut h, Modifiers::NONE, Key::Escape);
    click(&mut h, "Show all sizes");
    assert!(!h.state().session.canvas.measure.all_sizes);
}

#[test]
fn circumference_and_tape_are_taken_on_the_avatar() {
    use od_render::avatar::landmarks;
    use od_render::glam::Vec3;
    let mut h = harness();
    let view = h.get_by_label("3D garment view").rect();
    let (kind, m) = {
        let avatar = &h.state().session.viewport.avatar;
        (avatar.kind().unwrap(), avatar.measurements)
    };
    let waist = landmarks(kind, m).waist;
    // A point on the waist's front, as the camera sees it.
    let front = |h: &Harness<'_, OpenDesignApp>| {
        let mesh = h.state().session.viewport.avatar.mesh();
        let cut = od_render::measure::slice(mesh, Vec3::new(0.0, waist, 0.0), Vec3::Y);
        let p = cut
            .iter()
            .flat_map(|s| s.points.iter().copied())
            .filter(|p| p.x.abs() < 0.02)
            .max_by(|a, b| a.z.total_cmp(&b.z))
            .unwrap();
        h.state().session.viewport.screen_pos(view, p).unwrap()
    };

    click(&mut h, "Measure body");
    click(&mut h, "Circumference");
    assert_eq!(h.state().session.viewport.measure.mode, Mode::Girth);
    let at = front(&h);
    click_at(&mut h, at, Modifiers::NONE);
    let girths = h.state().session.viewport.measure.girth_lengths();
    assert_eq!(girths.len(), 1);
    let want = m.waist / 100.0;
    let got = girths[0].expect("a loop round the waist");
    assert!((got - want).abs() < want * 0.03, "{got} vs {want}");
    // Measuring doesn't select or change anything else.
    assert!(h.state().session.selection.primary().is_none());

    // The tape: two clicks on the front of the body.
    h.state_mut().session.viewport.measure.mode = Mode::Tape;
    h.run();
    click_at(&mut h, at, Modifiers::NONE);
    click_at(&mut h, at + egui::vec2(0.0, -40.0), Modifiers::NONE);
    let s = &mut h.state_mut().session;
    assert_eq!(s.viewport.measure.tape.len(), 2);
    let v = &mut s.viewport;
    v.measure.refresh(&v.avatar);
    let tape = v.measure.tape_length();
    assert!(tape > 0.02 && tape < 0.5, "{tape}");

    // Resizing the avatar measures again on the new body.
    let mut bigger = m;
    bigger.waist += 10.0;
    v.avatar.set_measurements(bigger);
    v.measure.refresh(&v.avatar);
    let got = v.measure.girth_lengths()[0].unwrap();
    assert!((got - bigger.waist / 100.0).abs() < 0.03, "{got}");
    v.measure.clear();
    v.measure.refresh(&v.avatar);
    assert!(v.measure.girth_lengths().is_empty());
}

/// Pictures of the measuring UI for checking it by eye, written to
/// `$OD_SHOTS` (default: a temporary folder). Needs a wgpu adapter; run
/// with `--ignored`.
#[test]
#[ignore]
fn screenshots_of_measuring() {
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
    if let Some(found) = dock.find_tab(&Tab::Viewport3D) {
        dock.remove_tab(found);
    }
    h.run();
    panel(&mut h);
    h.state_mut().session.canvas.snap.grid = false;
    press(&mut h, Modifiers::COMMAND, Key::A);
    press(&mut h, Modifiers::NONE, Key::F);
    h.state_mut().session.selection.clear();
    h.run();

    // Two measurements kept, a third being taken.
    click(&mut h, "Measure");
    click_mm(&mut h, 0.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 100.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 100.0, 0.0, Modifiers::NONE);
    click_mm(&mut h, 50.0, 80.0, Modifiers::SHIFT);
    click_mm(&mut h, 0.0, 80.0, Modifiers::NONE);
    let at = screen(&h, 100.0, 40.0);
    h.hover_at(at);
    show_tab(&mut h, Tab::Properties);
    shot(&mut h, "measure-tool.png");

    // Every graded size around the base.
    press(&mut h, Modifiers::NONE, Key::Escape);
    h.state_mut().session.canvas.measure.all_sizes = true;
    shot(&mut h, "all-sizes.png");

    // The chart.
    h.state_mut().session.canvas.measure.chart_open = true;
    shot(&mut h, "measurement-chart.png");
    eprintln!("screenshots in {}", dir.display());
}
