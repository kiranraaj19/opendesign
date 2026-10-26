//! Library: starter pattern blocks now; avatars and fabrics later.

use od_core::{geometry, OutlinePoint, PointKind, Vec2};

use crate::app::Session;

use PointKind::{Corner as C, Curve as Q};

/// A named outline: `(x, y, kind)` points in millimetres.
type Block = (&'static str, &'static [(f32, f32, PointKind)]);

/// Starter blocks, origin at the top-left. `Q` points are curve control
/// points: the outline bends towards them.
const BLOCKS: [Block; 4] = [
    (
        // T-shirt front, both halves: scooped neckline, curved armholes.
        "Front bodice",
        &[
            (40.0, 50.0, C),   // left shoulder
            (180.0, 0.0, C),   // left neck point
            (185.0, 95.0, Q),  // front neckline...
            (335.0, 95.0, Q),  // ...through centre front at (260, 95)
            (340.0, 0.0, C),   // right neck point
            (480.0, 50.0, C),  // right shoulder
            (468.0, 215.0, Q), // right armhole
            (520.0, 230.0, C), // right underarm
            (520.0, 700.0, C), // right hem
            (0.0, 700.0, C),   // left hem
            (0.0, 230.0, C),   // left underarm
            (52.0, 215.0, Q),  // left armhole
        ],
    ),
    (
        // T-shirt back: the same with a shallow back neck.
        "Back bodice",
        &[
            (40.0, 50.0, C),
            (180.0, 0.0, C),
            (185.0, 25.0, Q),
            (335.0, 25.0, Q),
            (340.0, 0.0, C),
            (480.0, 50.0, C),
            (468.0, 215.0, Q),
            (520.0, 230.0, C),
            (520.0, 700.0, C),
            (0.0, 700.0, C),
            (0.0, 230.0, C),
            (52.0, 215.0, Q),
        ],
    ),
    (
        // Short sleeve: an S-shaped cap, hollow at the underarms and
        // rounded over the shoulder, tapering to the hem.
        "Sleeve",
        &[
            (0.0, 130.0, C),
            (80.0, 125.0, Q),
            (120.0, 0.0, Q),
            (320.0, 0.0, Q),
            (360.0, 125.0, Q),
            (440.0, 130.0, C),
            (410.0, 330.0, C),
            (30.0, 330.0, C),
        ],
    ),
    (
        "Skirt panel",
        &[
            (100.0, 0.0, C),
            (400.0, 0.0, C),
            (500.0, 650.0, C),
            (0.0, 650.0, C),
        ],
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
                    let positions: Vec<Vec2> =
                        points.iter().map(|&(x, y, _)| Vec2::new(x, y)).collect();
                    let (min, max) = geometry::bounds(&positions).unwrap_or_default();
                    let shift = center - (min + max) * 0.5;
                    let outline = points
                        .iter()
                        .map(|&(x, y, kind)| OutlinePoint {
                            pos: Vec2::new(x, y) + shift,
                            kind,
                        })
                        .collect();
                    s.add_outline(name, outline);
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
