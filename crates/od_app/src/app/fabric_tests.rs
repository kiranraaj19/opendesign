//! UI tests for Phase 7's fabric work: the fabric editor, fabric files in
//! the Library, and colourways. A child of `app` so it can reach the
//! app's state the way the main test module does.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use od_core::{EditFabric, Fabric, FabricTexture, FabricValue, PatternId, Vec2};

use super::{OpenDesignApp, Tab};

/// Tall enough that Properties shows everything without scrolling.
fn harness() -> Harness<'static, OpenDesignApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 1600.0))
        .build_eframe(|cc| OpenDesignApp::new(cc));
    h.run();
    h
}

fn click(h: &mut Harness<'_, OpenDesignApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn show_tab(h: &mut Harness<'_, OpenDesignApp>, tab: Tab) {
    let dock = &mut h.state_mut().dock;
    let found = dock.find_tab(&tab).unwrap();
    dock.set_active_tab(found).unwrap();
    h.run();
}

fn drag(h: &mut Harness<'_, OpenDesignApp>, from: egui::Pos2, to: egui::Pos2) {
    h.drag_at(from);
    h.run();
    for t in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + (to - from) * t);
        h.run();
    }
    h.drop_at(to);
    h.run();
}

/// One selected rectangle, Properties showing.
fn one_piece(h: &mut Harness<'_, OpenDesignApp>) -> PatternId {
    let s = &mut h.state_mut().session;
    s.add_pattern(
        "Panel",
        od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 80.0)),
    );
    let id = s.doc.patterns[0].id;
    show_tab(h, Tab::Properties);
    id
}

fn fabric(h: &Harness<'_, OpenDesignApp>, id: PatternId) -> Fabric {
    h.state().session.doc.pattern(id).unwrap().fabric.clone()
}

fn last_step(h: &Harness<'_, OpenDesignApp>) -> String {
    h.state().session.history.undo_label().unwrap_or_default()
}

/// Where the editor's field for `value` is on screen.
fn field(h: &Harness<'_, OpenDesignApp>, value: FabricValue) -> egui::Pos2 {
    h.get_all_by_label(value.label())
        .find(|n| n.accesskit_node().role() == Role::SpinButton)
        .unwrap_or_else(|| panic!("no field for {value:?}"))
        .rect()
        .center()
}

#[test]
fn the_fabric_editor_changes_one_value_per_undo_step() {
    let mut h = harness();
    let id = one_piece(&mut h);
    // Collapsed at first, so Properties stays short.
    assert!(h
        .query_all_by_label(FabricValue::StretchWeft.label())
        .all(|n| n.accesskit_node().role() != Role::SpinButton));
    click(&mut h, "Edit fabric");

    let before = fabric(&h, id);
    let at = field(&h, FabricValue::StretchWeft);
    drag(&mut h, at, at - egui::vec2(40.0, 0.0));
    let after = fabric(&h, id);
    assert!(
        after.weft_stretch_compliance > before.weft_stretch_compliance,
        "softer across the grain: {} -> {}",
        before.weft_stretch_compliance,
        after.weft_stretch_compliance
    );
    assert_eq!(after.stretch_compliance, before.stretch_compliance);
    assert_eq!(last_step(&h), "Edit fabric: stretch firmness across grain");
    assert_eq!(h.state().session.history.done_labels().len(), 2, "one step");

    // Heavier.
    let at = field(&h, FabricValue::Weight);
    drag(&mut h, at, at + egui::vec2(40.0, 0.0));
    assert!(fabric(&h, id).mass_per_area > before.mass_per_area);

    // The weave, one click.
    click(&mut h, "Twill");
    assert_eq!(fabric(&h, id).texture, FabricTexture::Twill);
    assert_eq!(last_step(&h), "Edit fabric: weave");

    // An edited cotton isn't the preset any more: picking Cotton resets it.
    click(&mut h, "Cotton");
    assert_eq!(fabric(&h, id), Fabric::cotton());

    for _ in 0..4 {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
    }
    assert_eq!(fabric(&h, id), before);
}
#[test]
fn a_saved_fabric_is_listed_in_the_library_and_can_be_used() {
    let mut h = harness();
    let id = one_piece(&mut h);
    let dir = std::env::temp_dir().join(format!("od_app_{}_fabric_files", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wool = Fabric {
        name: "Boiled wool".into(),
        ..FabricValue::BendWarp.set(&Fabric::denim(), 90.0)
    };
    let path = dir.join("wool.odfab");
    assert!(h.state_mut().session.fabrics.save(&wool, &path));
    show_tab(&mut h, Tab::Library);
    click(&mut h, "Boiled wool");
    assert_eq!(fabric(&h, id), wool);
    assert_eq!(last_step(&h), "Set fabric: Boiled wool");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(fabric(&h, id), Fabric::default());

    // A file joins the list once, however often it's loaded, and the
    // list is remembered by path for next time.
    let shelf = &mut h.state_mut().session.fabrics;
    assert!(shelf.load(&path).is_some());
    assert_eq!(shelf.fabrics.len(), 1);
    let mut storage = MemStorage::default();
    eframe::App::save(h.state_mut(), &mut storage);
    let paths: Vec<std::path::PathBuf> =
        eframe::get_value(&storage, crate::fabric_shelf::FABRICS_KEY).unwrap();
    assert_eq!(paths, vec![path]);

    // Taking it off the list.
    h.run();
    click(&mut h, "Forget");
    assert!(h.state().session.fabrics.fabrics.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn colourways_switch_the_garments_colours_and_undo() {
    let mut h = harness();
    let id = one_piece(&mut h);
    h.state_mut().session.selection.clear();
    h.run();
    let natural = fabric(&h, id).color;
    click(&mut h, "New colourway");
    assert_eq!(last_step(&h), "Add colourway: Colourway 1");

    // Recolour, and keep that as a second colourway.
    {
        let s = &mut h.state_mut().session;
        let red = Fabric {
            color: [190, 30, 40],
            ..s.doc.patterns[0].fabric.clone()
        };
        s.execute(EditFabric::new(id, red, "colour"));
    }
    h.run();
    click(&mut h, "New colourway");
    assert_eq!(h.state().session.doc.colourways.len(), 2);

    click(&mut h, "Colourway 1");
    assert_eq!(fabric(&h, id).color, natural);
    assert_eq!(last_step(&h), "Switch colourway: Colourway 1");
    click(&mut h, "Colourway 2");
    assert_eq!(fabric(&h, id).color, [190, 30, 40]);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(fabric(&h, id).color, natural);
    assert_eq!(h.state().session.doc.active_colourway, Some(0));

    // Recolouring the shown colourway can be saved into it.
    {
        let s = &mut h.state_mut().session;
        let blue = Fabric {
            color: [20, 40, 160],
            ..s.doc.patterns[0].fabric.clone()
        };
        s.execute(EditFabric::new(id, blue, "colour"));
    }
    h.run();
    click(&mut h, "Save colours");
    assert_eq!(
        h.state().session.doc.colourways[0].colors,
        vec![(id, [20, 40, 160])]
    );
    click(&mut h, "Delete colourway");
    assert_eq!(h.state().session.doc.colourways.len(), 1);
    assert_eq!(h.state().session.doc.active_colourway, None);
}

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

/// Screenshots of the fabric editor, the Library's fabric list and the
/// colourways, for a look by eye, written to `$OD_SHOTS`. Needs a wgpu
/// adapter; a software one (lavapipe) is enough.
#[test]
#[ignore = "writes screenshots; run with --ignored"]
fn screenshots_of_fabric_editor_and_colourways() {
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
    // doesn't have.
    let dock = &mut h.state_mut().dock;
    if let Some(found) = dock.find_tab(&Tab::Viewport3D) {
        dock.remove_tab(found);
    }
    h.run();
    let id = one_piece(&mut h);
    {
        let s = &mut h.state_mut().session;
        let edited = FabricValue::Shear.set(&s.doc.patterns[0].fabric, 20.0);
        s.execute(EditFabric::new(id, edited, "bias"));
    }
    h.run();
    click(&mut h, "Edit fabric");
    shot(&mut h, "fabric-editor.png");

    let saved = std::env::temp_dir().join(format!("od_shots_{}.odfab", std::process::id()));
    let linen = Fabric {
        name: "Linen".into(),
        color: [200, 190, 160],
        ..Fabric::cotton()
    };
    h.state_mut().session.fabrics.save(&linen, &saved);
    show_tab(&mut h, Tab::Library);
    shot(&mut h, "fabric-library.png");
    let _ = std::fs::remove_file(&saved);

    show_tab(&mut h, Tab::Properties);
    h.state_mut().session.selection.clear();
    h.run();
    click(&mut h, "New colourway");
    {
        let s = &mut h.state_mut().session;
        let red = Fabric {
            color: [190, 30, 40],
            ..s.doc.patterns[0].fabric.clone()
        };
        s.execute(EditFabric::new(id, red, "colour"));
    }
    h.run();
    click(&mut h, "New colourway");
    click(&mut h, "Colourway 1");
    shot(&mut h, "colourways.png");
    eprintln!("screenshots in {}", dir.display());
}
