//! Library: starter pattern blocks now; avatars and fabrics later.

use od_core::{geometry, Vec2};

use crate::app::Session;

/// Convex starter blocks in millimetres, origin at the top-left.
const BLOCKS: [(&str, &[(f32, f32)]); 4] = [
    (
        "Front bodice",
        &[
            (0.0, 40.0),
            (150.0, 0.0),
            (350.0, 0.0),
            (500.0, 40.0),
            (480.0, 620.0),
            (20.0, 620.0),
        ],
    ),
    (
        "Back bodice",
        &[(0.0, 20.0), (500.0, 20.0), (480.0, 620.0), (20.0, 620.0)],
    ),
    (
        "Sleeve",
        &[
            (60.0, 550.0),
            (0.0, 180.0),
            (250.0, 0.0),
            (500.0, 180.0),
            (440.0, 550.0),
        ],
    ),
    (
        "Skirt panel",
        &[(100.0, 0.0), (400.0, 0.0), (500.0, 650.0), (0.0, 650.0)],
    ),
];

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    egui::CollapsingHeader::new("Pattern blocks")
        .default_open(true)
        .show(ui, |ui| {
            for (name, points) in BLOCKS {
                if ui
                    .button(format!("+ {name}"))
                    .on_hover_text("Add to the 2D window")
                    .clicked()
                {
                    // Centre new blocks in the 2D view, cascading so repeats
                    // don't stack exactly on top of each other.
                    let step = (s.doc.patterns.len() % 8) as f32 * 40.0;
                    let center = s.canvas.view_center() + Vec2::new(step, step);
                    let outline: Vec<Vec2> = points.iter().map(|&(x, y)| Vec2::new(x, y)).collect();
                    let (min, max) = geometry::bounds(&outline).unwrap_or_default();
                    let shift = center - (min + max) * 0.5;
                    let outline = outline.into_iter().map(|p| p + shift).collect();
                    s.add_pattern(name, outline);
                }
            }
        });

    placeholder_section(ui, "Avatars", &["Female", "Male", "Kid"], "Phase 4");
    placeholder_section(
        ui,
        "Fabrics",
        &["Cotton poplin", "Denim", "Silk charmeuse"],
        "Phase 7",
    );
    placeholder_section(ui, "Trims", &["Buttons", "Zippers"], "Phase 10");
}

fn placeholder_section(ui: &mut egui::Ui, title: &str, items: &[&str], phase: &str) {
    egui::CollapsingHeader::new(title).show(ui, |ui| {
        for item in items {
            ui.add_enabled(false, egui::Button::new(*item))
                .on_disabled_hover_text(format!("Coming in {phase}"));
        }
    });
}
