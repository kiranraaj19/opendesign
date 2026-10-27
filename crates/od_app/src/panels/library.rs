//! Library: starter pattern blocks, avatars and fabrics (Phase 8's asset
//! browser). Thumbnails are small shapes painted directly by egui rather
//! than real renders: WSL has no GPU/display to render a live preview
//! from, and a scoped Phase 8 doesn't need one — a silhouette for each
//! avatar body type, a flat colour swatch for each fabric.

use od_core::{geometry, OutlinePoint, PointKind, SetFabric, Vec2, BUILTIN_FABRICS};
use od_render::avatar::AvatarKind;

use crate::app::Session;
use crate::theme;

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

    avatars(ui, s);
    fabrics(ui, s);
    placeholder_section(ui, "Trims", &["Buttons", "Zippers"], "Phase 11");
}

/// Side of a thumbnail swatch/icon, in points.
const THUMB_SIZE: f32 = 28.0;

/// Preset mannequins, or a model from a file, for the 3D window. Each
/// preset gets a small painted silhouette rather than a rendered
/// thumbnail (see the module doc).
fn avatars(ui: &mut egui::Ui, s: &mut Session) {
    egui::CollapsingHeader::new("Avatars")
        .default_open(true)
        .show(ui, |ui| {
            let avatar = &mut s.viewport.avatar;
            for kind in AvatarKind::ALL {
                let selected = avatar.kind() == Some(kind);
                let response = ui
                    .horizontal(|ui| {
                        avatar_icon(ui, kind, selected);
                        ui.selectable_label(selected, kind.label())
                    })
                    .inner
                    .on_hover_text("Use this avatar in the 3D window");
                if response.clicked() {
                    avatar.set_preset(kind);
                }
            }
            if ui
                .button("Load avatar file…")
                .on_hover_text("OBJ or glTF, Y up; sizes in m, cm or mm")
                .clicked()
            {
                s.viewport.load_avatar_dialog();
            }
        });
}

/// A simple humanoid silhouette (head circle + body wedge) standing in for
/// a thumbnail render of the preset avatar.
fn avatar_icon(ui: &mut egui::Ui, kind: AvatarKind, selected: bool) {
    let pal = theme::palette(ui.visuals().dark_mode);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(THUMB_SIZE, THUMB_SIZE), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, pal.widget_bg);
    let color = if selected { pal.accent } else { pal.hint };

    // Head-to-shoulder ratio and silhouette width vary a little by body
    // type, just enough that the three icons read as different.
    let (head_r, shoulder_w) = match kind {
        AvatarKind::Female => (0.11, 0.42),
        AvatarKind::Male => (0.12, 0.52),
        AvatarKind::Kid => (0.15, 0.36),
    };
    let w = rect.width();
    let h = rect.height();
    let cx = rect.center().x;
    let top = rect.top() + h * 0.18;
    let head_radius = w * head_r;
    painter.circle_filled(egui::pos2(cx, top), head_radius, color);

    let body_top = top + head_radius * 1.3;
    let body_bottom = rect.bottom() - h * 0.1;
    let half_shoulder = w * shoulder_w * 0.5;
    let half_hem = half_shoulder * 0.6;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(cx - half_shoulder, body_top),
            egui::pos2(cx + half_shoulder, body_top),
            egui::pos2(cx + half_hem, body_bottom),
            egui::pos2(cx - half_hem, body_bottom),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// Built-in fabrics; picking one assigns it to the selected pattern. A
/// flat colour square stands in for a render of the weave (see the module
/// doc).
fn fabrics(ui: &mut egui::Ui, s: &mut Session) {
    egui::CollapsingHeader::new("Fabrics")
        .default_open(true)
        .show(ui, |ui| {
            let selected_pattern = s.selection;
            let current_fabric = selected_pattern
                .and_then(|id| s.doc.pattern(id))
                .and_then(|p| p.fabric.clone());

            if selected_pattern.is_none() {
                ui.weak("Select a pattern to assign a fabric.");
            }

            for fabric in BUILTIN_FABRICS {
                let selected = current_fabric.as_deref() == Some(fabric.name);
                let response = ui
                    .horizontal(|ui| {
                        fabric_swatch(ui, fabric.color, selected);
                        ui.add_enabled(
                            selected_pattern.is_some(),
                            egui::Button::selectable(selected, fabric.name),
                        )
                    })
                    .inner
                    .on_hover_text("Assign this fabric to the selected pattern");
                if response.clicked() {
                    if let Some(id) = selected_pattern {
                        let next = if selected {
                            None
                        } else {
                            Some(fabric.name.to_owned())
                        };
                        s.execute(SetFabric::new(id, next));
                    }
                }
            }
        });
}

fn fabric_swatch(ui: &mut egui::Ui, color: [u8; 3], selected: bool) {
    let pal = theme::palette(ui.visuals().dark_mode);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(THUMB_SIZE, THUMB_SIZE), egui::Sense::hover());
    let painter = ui.painter();
    let [r, g, b] = color;
    painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(r, g, b));
    let border = if selected { pal.accent } else { pal.border };
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(if selected { 2.0 } else { 1.0 }, border),
        egui::StrokeKind::Outside,
    );
}

fn placeholder_section(ui: &mut egui::Ui, title: &str, items: &[&str], phase: &str) {
    egui::CollapsingHeader::new(title).show(ui, |ui| {
        for item in items {
            ui.add_enabled(false, egui::Button::new(*item))
                .on_disabled_hover_text(format!("Coming in {phase}"));
        }
    });
}
