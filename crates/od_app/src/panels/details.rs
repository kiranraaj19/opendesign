//! Garment details in the 2D window and Properties: the Zipper tool, the
//! zippers drawn along their seams (tape, teeth and slider), and the
//! zipper settings of a selected seam.

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, Shape, Stroke};
use od_core::{
    AddSewingLine, Pattern, SeamEdge, SetZipper, SewingLine, SewingLineId, Vec2, Zipper,
};

use crate::app::{Session, Tool};
use crate::theme;

/// The detail tools' state in the 2D window.
#[derive(Debug, Clone, PartialEq)]
pub struct Details2D {
    /// The first stretch of a zipper being placed.
    pub zipper_first: Option<SeamEdge>,
    /// Where a drag along an outline began, in doc space.
    drag_start: Option<Vec2>,
    /// Tape width of new zippers, in mm.
    pub tape_width: f32,
}

impl Default for Details2D {
    fn default() -> Self {
        Self {
            zipper_first: None,
            drag_start: None,
            tape_width: Zipper::DEFAULT_TAPE_WIDTH,
        }
    }
}

/// Pick distance in screen pixels, as for the other tools.
const PICK_PX: f32 = 8.0;

/// Same stretch of the same outline, whichever way it runs.
fn same_stretch(a: &SeamEdge, b: &SeamEdge) -> bool {
    a.pattern == b.pattern && a.start == b.start && a.end == b.end
}

/// The outline stretch a click or drag picked: a click takes the whole
/// corner-to-corner segment, a drag along one outline the stretch from
/// where it began to where it ended (the shorter way round).
fn picked_edge(response: &egui::Response, rect: Rect, s: &mut Session) -> Option<SeamEdge> {
    // egui forgets the press position once the drag is released.
    if response.dragged() || response.drag_started() {
        if let Some(origin) = response.ctx.input(|i| i.pointer.press_origin()) {
            s.canvas.details.drag_start = Some(s.canvas.to_doc(rect, origin));
        }
    }
    if !response.clicked() && !response.drag_stopped() {
        return None;
    }
    let release = s.canvas.to_doc(rect, response.interact_pointer_pos()?);
    let press = s.canvas.details.drag_start.take();
    let reach = s.canvas.px(PICK_PX);
    let selected = s.selection.primary();
    match press.filter(|p| p.distance(release) > reach) {
        Some(press) => {
            let a = s.canvas.edge_at(&s.doc, selected, press)?;
            let b = s.canvas.edge_at(&s.doc, selected, release)?;
            (a.id == b.id)
                .then(|| s.doc.pattern(a.id))
                .flatten()
                .map(|p| p.shorter_edge(a.param, b.param))
        }
        None => s.canvas.segment_edge(&s.doc, selected, release),
    }
}

/// The Zipper tool: pick one outline stretch, then the one it zips to
/// (on the same piece or another), and the zipper is sewn between them.
/// Picking a stretch that already has a zipper opens or closes it.
pub fn handle_zipper(ui: &egui::Ui, response: &egui::Response, rect: Rect, s: &mut Session) {
    let typing = ui.memory(|m| m.focused().is_some());
    if !typing && ui.input(|i| i.key_pressed(Key::Escape)) {
        s.canvas.details.zipper_first = None;
    }
    let Some(edge) = picked_edge(response, rect, s) else {
        return;
    };
    s.selection.set(Some(edge.pattern));
    match s.canvas.details.zipper_first.take() {
        None => {
            let zipped = s
                .doc
                .sewing_lines
                .iter()
                .find(|l| l.is_zipper() && l.edges().any(|e| same_stretch(e, &edge)));
            match zipped {
                Some(line) => {
                    let id = line.id;
                    let zipper = line.details.zipper.map(|z| Zipper {
                        closed: !z.closed,
                        ..z
                    });
                    s.execute(SetZipper::new(id, zipper));
                    s.selected_seam = Some(id);
                }
                None => s.canvas.details.zipper_first = Some(edge),
            }
        }
        Some(first) if same_stretch(&first, &edge) => {}
        Some(first) => {
            let zipper = Zipper {
                closed: true,
                tape_width: s.canvas.details.tape_width,
            };
            s.execute(AddSewingLine::new(vec![first], vec![edge]).zipped(zipper));
            s.selected_seam = s.doc.sewing_lines.last().map(|l| l.id);
        }
    }
}

/// What the detail tools say they do, at the top of the 2D window.
pub fn hint(s: &Session) -> &'static str {
    match s.tool {
        Tool::Zipper if s.canvas.details.zipper_first.is_some() => {
            "Click or drag along the stretch it zips to · Esc cancels"
        }
        Tool::Zipper => {
            "Click an edge (or drag along one) for one side of a zipper · click a zipper to open or close it"
        }
        _ => "",
    }
}

/// Teeth are drawn this far apart along a zipper, in mm.
const TOOTH_MM: f32 = 4.0;

/// The unit normal at `a` → `b` pointing into `pattern`.
fn inward(pattern: &Pattern, a: Vec2, b: Vec2) -> Vec2 {
    let d = b - a;
    let len = d.length();
    if len < 1e-6 {
        return Vec2::ZERO;
    }
    let n = Vec2::new(-d.y, d.x) * (1.0 / len);
    if pattern.contains(a.lerp(b, 0.5) + n * 0.5) {
        n
    } else {
        -n
    }
}

/// A band `width` mm wide along the inside of the open polyline `points`
/// on `pattern`, as quads in doc space.
fn band(pattern: &Pattern, points: &[Vec2], width: f32) -> Vec<[Vec2; 4]> {
    points
        .windows(2)
        .map(|w| {
            let n = inward(pattern, w[0], w[1]) * width;
            [w[0], w[1], w[1] + n, w[0] + n]
        })
        .collect()
}

/// Points every `step` mm along the open polyline `points`, each with the
/// segment it falls on.
fn every(points: &[Vec2], step: f32) -> Vec<(Vec2, usize)> {
    let mut out = Vec::new();
    let mut next = step * 0.5;
    let mut walked = 0.0;
    for (i, w) in points.windows(2).enumerate() {
        let len = w[0].distance(w[1]);
        while next <= walked + len && len > 0.0 {
            out.push((w[0].lerp(w[1], (next - walked) / len), i));
            next += step;
        }
        walked += len;
    }
    out
}

/// The colour zippers are drawn in.
pub fn zipper_color(pal: &theme::Palette) -> Color32 {
    pal.text
}

/// Zippers: a band for each side's tape, teeth across the seam line, and
/// the slider (at the top when closed, at the bottom stop when open), plus
/// the first stretch of one being placed.
pub fn draw(painter: &egui::Painter, rect: Rect, s: &Session, pal: &theme::Palette) {
    let to_screen = |p: Vec2| s.canvas.to_screen(rect, p);
    for line in s.doc.sewing_lines.iter().filter(|l| l.is_zipper()) {
        draw_zipper(painter, s, line, pal, &to_screen);
    }
    if let Some(first) = &s.canvas.details.zipper_first {
        if let Some(pattern) = s.doc.pattern(first.pattern) {
            let pts: Vec<Pos2> = pattern
                .edge_points(first)
                .into_iter()
                .map(to_screen)
                .collect();
            painter.add(Shape::line(pts, Stroke::new(4.0, pal.preview)));
        }
    }
}

fn draw_zipper(
    painter: &egui::Painter,
    s: &Session,
    line: &SewingLine,
    pal: &theme::Palette,
    to_screen: &impl Fn(Vec2) -> Pos2,
) {
    let Some(zipper) = line.details.zipper else {
        return;
    };
    let color = zipper_color(pal);
    let selected = s.selected_seam == Some(line.id);
    let tape = if selected { pal.selected } else { pal.accent }.gamma_multiply(0.35);
    let half_tooth = s.canvas.px(4.0).max(1.0);
    let mut slider = None;
    for (k, edge) in line.edges().enumerate() {
        let Some(pattern) = s.doc.pattern(edge.pattern).filter(|p| !p.hidden) else {
            continue;
        };
        let points = pattern.edge_points(edge);
        for quad in band(pattern, &points, zipper.tape_width * 0.5) {
            painter.add(Shape::convex_polygon(
                quad.map(to_screen).to_vec(),
                tape,
                Stroke::NONE,
            ));
        }
        for (at, i) in every(&points, TOOTH_MM) {
            let n = inward(pattern, points[i], points[i + 1]);
            // Across the seam line, standing out either side of it.
            painter.line_segment(
                [
                    to_screen(at - n * half_tooth),
                    to_screen(at + n * half_tooth),
                ],
                Stroke::new(1.5, color),
            );
        }
        if k == 0 {
            slider = if zipper.closed {
                points.last().copied()
            } else {
                points.first().copied()
            };
        }
    }
    if let Some(at) = slider {
        let at = to_screen(at);
        painter.rect_filled(
            Rect::from_center_size(at, egui::vec2(8.0, 12.0)),
            2.0,
            color,
        );
        if !zipper.closed {
            painter.text(
                at + egui::vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                "open",
                FontId::proportional(11.0),
                color,
            );
        }
    }
}

/// Settings for a detail tool in use, in Properties.
pub fn tool_section(ui: &mut egui::Ui, s: &mut Session) {
    if s.tool == Tool::Zipper {
        ui.add_space(8.0);
        ui.heading("Zipper tool");
        ui.horizontal(|ui| {
            ui.label("Tape width");
            ui.add(
                egui::DragValue::new(&mut s.canvas.details.tape_width)
                    .range(Zipper::TAPE_WIDTH_RANGE)
                    .suffix(" mm")
                    .speed(0.5),
            );
        });
    }
}

/// A number edited by dragging or typing that takes effect, as one undo
/// step, only once let go of. The value in progress is kept in egui's
/// memory under `key`.
fn deferred_value(
    ui: &mut egui::Ui,
    key: impl std::hash::Hash + std::fmt::Debug,
    current: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> Option<f32> {
    let id = egui::Id::new(("deferred_value", key));
    let mut value = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(current);
    let response = ui.add(
        egui::DragValue::new(&mut value)
            .range(range)
            .suffix(suffix)
            .speed(0.5)
            .max_decimals(1),
    );
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(id, value));
    }
    if response.dragged() || response.has_focus() {
        return None;
    }
    let done = ui.data_mut(|d| d.remove_temp::<f32>(id))?;
    (done != current).then_some(done)
}

/// A selected seam's zipper: open or closed and its tape, or a way to
/// make the seam one.
pub fn seam_section(ui: &mut egui::Ui, s: &mut Session, id: SewingLineId) {
    let Some(line) = s.doc.sewing_line(id) else {
        return;
    };
    ui.add_space(8.0);
    let Some(zipper) = line.details.zipper else {
        if ui
            .button("Make zipper")
            .on_hover_text("Sew a zipper into this seam, closed")
            .clicked()
        {
            let zipper = Zipper {
                tape_width: s.canvas.details.tape_width,
                ..Zipper::default()
            };
            s.execute(SetZipper::new(id, Some(zipper)));
        }
        return;
    };
    ui.label("Zipper");
    let mut closed = zipper.closed;
    let mut set = None;
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut closed, "Closed")
            .on_hover_text("A closed zipper holds its sides together when draping")
            .changed()
        {
            set = Some(Zipper { closed, ..zipper });
        }
        ui.label("Tape");
        if let Some(width) = deferred_value(
            ui,
            ("zipper_tape", id),
            zipper.tape_width,
            Zipper::TAPE_WIDTH_RANGE,
            " mm",
        ) {
            set = Some(Zipper {
                tape_width: width,
                ..zipper
            });
        }
        if ui
            .button("Plain seam")
            .on_hover_text("Take the zipper out and keep the seam")
            .clicked()
        {
            s.execute(SetZipper::new(id, None));
        }
    });
    if let Some(zipper) = set {
        s.execute(SetZipper::new(id, Some(zipper)));
    }
}

/// The tape band drawn along `edge`, for tests.
#[cfg(test)]
pub(crate) fn tape_quads(pattern: &Pattern, edge: &SeamEdge, width: f32) -> Vec<[Vec2; 4]> {
    band(pattern, &pattern.edge_points(edge), width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use od_core::{OutlinePoint, PatternId};

    #[test]
    fn a_tape_lies_inside_the_piece() {
        let pattern = Pattern::new(
            PatternId(1),
            "Panel",
            od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 200.0))
                .into_iter()
                .map(OutlinePoint::corner)
                .collect(),
        );
        for (start, forward) in [(1.0, true), (3.0, false)] {
            let edge = SeamEdge {
                pattern: PatternId(1),
                start,
                end: start + 1.0,
                forward,
            };
            for quad in tape_quads(&pattern, &edge, 6.0) {
                let mid = quad[2].lerp(quad[3], 0.5);
                assert!(pattern.contains(mid), "{quad:?}");
            }
        }
        let teeth = every(&[Vec2::ZERO, Vec2::new(0.0, 20.0)], TOOTH_MM);
        assert_eq!(teeth.len(), 5);
        assert_eq!(teeth[0].0, Vec2::new(0.0, 2.0));
    }
}
