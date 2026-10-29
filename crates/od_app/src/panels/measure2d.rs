//! Measuring the flat patterns: the Measure tool in the 2D window, the
//! named measurements drawn over the pieces, every graded size drawn
//! faintly around the one previewed, the Measurements section of
//! Properties and the measurement chart (every measurement at every size).

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, Shape, Stroke, Vec2 as Px};
use od_core::document::TOLERANCE;
use od_core::measure::{chart, chart_csv};
use od_core::{
    AddMeasurement, Document, MeasureKind, Measurement, MeasurementId, OutlineSpot,
    RemoveMeasurement, RenameMeasurement, Vec2,
};

use crate::app::{Session, Tool};
use crate::theme;

/// Measuring state of the 2D window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measure2D {
    /// Where a measurement being taken starts.
    pub start: Option<OutlineSpot>,
    /// Measure along the outline rather than straight across (Shift
    /// switches for one click).
    pub along: bool,
    /// Draw every size's outline faintly around the one previewed.
    pub all_sizes: bool,
    /// The measurement chart window is open.
    pub chart_open: bool,
    /// The measurement last added or picked in Properties, drawn
    /// highlighted.
    pub selected: Option<MeasurementId>,
    /// A measurement's name being typed in Properties.
    editing: Option<(MeasurementId, String)>,
}

/// The outline point a Measure click at `p` lands on, snapped to a corner
/// within reach, and where that is.
fn spot_at(s: &Session, p: Vec2) -> Option<(OutlineSpot, Vec2)> {
    let hit = s.canvas.edge_at(&s.doc, s.selection.primary(), p)?;
    let pattern = s.doc.pattern(hit.id)?;
    let param = s.canvas.snap_param(pattern, hit.param);
    let spot = OutlineSpot {
        pattern: hit.id,
        param,
    };
    Some((spot, pattern.pos_at_param(param, TOLERANCE)))
}

/// What a measurement from `a` to `b` measures: along the outline (the
/// shorter way) when asked for and both are on one pattern, else straight
/// across.
fn kind_between(doc: &Document, a: OutlineSpot, b: OutlineSpot, along: bool) -> MeasureKind {
    match doc.pattern(a.pattern) {
        Some(p) if along && a.pattern == b.pattern => {
            MeasureKind::Along(p.shorter_edge(a.param, b.param))
        }
        _ => MeasureKind::Distance { a, b },
    }
}

/// How long a measurement from `a` to `b` would be, at the base size.
fn length_between(doc: &Document, a: OutlineSpot, b: OutlineSpot, along: bool) -> Option<f32> {
    let m = Measurement {
        id: MeasurementId(0),
        name: String::new(),
        kind: kind_between(doc, a, b, along),
    };
    m.length_at(doc, 0)
}

/// Measurements shorter than this (in mm) are taken as a double click.
const MIN_LENGTH: f32 = 0.5;

/// The Measure tool: click an outline point, then another; the
/// measurement between them is kept in the document.
pub fn handle(ui: &egui::Ui, response: &egui::Response, rect: Rect, s: &mut Session, typing: bool) {
    if !typing && ui.input(|i| i.key_pressed(Key::Escape)) {
        s.canvas.measure.start = None;
    }
    if !response.clicked() {
        return;
    }
    let Some(pos) = response.interact_pointer_pos() else {
        return;
    };
    let p = s.canvas.to_doc(rect, pos);
    let Some((spot, _)) = spot_at(s, p) else {
        return;
    };
    let Some(start) = s.canvas.measure.start.take() else {
        s.canvas.measure.start = Some(spot);
        return;
    };
    let along = s.canvas.measure.along != ui.input(|i| i.modifiers.shift);
    if length_between(&s.doc, start, spot, along).is_none_or(|l| l < MIN_LENGTH) {
        // Clicked the start again: keep waiting for the other end.
        s.canvas.measure.start = Some(start);
        return;
    }
    let add = AddMeasurement::new(None, kind_between(&s.doc, start, spot, along));
    let count = s.doc.measurements.len();
    s.execute(add);
    s.canvas.measure.selected = s.doc.measurements.get(count).map(|m| m.id);
}

/// A label with a backing so it reads over outlines and the grid.
fn label(painter: &egui::Painter, at: Pos2, text: String, color: Color32, pal: &theme::Palette) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), color);
    let rect = Align2::LEFT_BOTTOM
        .anchor_size(at, galley.size())
        .expand(2.0);
    // Kept inside the window, so a measurement by its edge still reads.
    let clip = painter.clip_rect();
    let shift = Px::new(
        (clip.right() - rect.right()).min(0.0) + (clip.left() - rect.left()).max(0.0),
        (clip.bottom() - rect.bottom()).min(0.0) + (clip.top() - rect.top()).max(0.0),
    );
    let rect = rect.translate(shift);
    painter.rect_filled(rect, 2.0, pal.canvas_bg.gamma_multiply(0.85));
    painter.galley(rect.min + Px::splat(2.0), galley, color);
}

/// "123.4 mm", or "–" for a measurement whose pattern is gone.
pub fn format_length(length: Option<f32>) -> String {
    length.map_or_else(|| "–".to_owned(), |mm| format!("{mm:.1} mm"))
}

/// Draw a measurement's line: dashed between two points with ticks at
/// the ends, or along the outline stretch it measures.
fn draw_line(painter: &egui::Painter, points: &[Pos2], along: bool, stroke: Stroke) {
    if points.len() < 2 {
        return;
    }
    if along {
        painter.add(Shape::line(points.to_vec(), stroke));
    } else {
        painter.extend(Shape::dashed_line(points, stroke, 6.0, 3.0));
    }
    for (end, next) in [
        (points[0], points[1]),
        (points[points.len() - 1], points[points.len() - 2]),
    ] {
        let d = (next - end).normalized();
        let tick = Px::new(-d.y, d.x) * 5.0;
        painter.line_segment([end - tick, end + tick], stroke);
    }
}

/// Where a label for a line of screen points goes: half way along it.
fn middle(points: &[Pos2]) -> Option<Pos2> {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total * 0.5;
    for w in points.windows(2) {
        let d = w[0].distance(w[1]);
        if d >= left && d > 0.0 {
            return Some(w[0].lerp(w[1], left / d));
        }
        left -= d;
    }
    points.first().copied()
}

/// The document's measurements over the patterns, at the size being
/// previewed, and the one being taken with the Measure tool.
pub fn draw(painter: &egui::Painter, rect: Rect, s: &Session, pal: &theme::Palette) {
    let to_screen = |p: Vec2| s.canvas.to_screen(rect, p);
    let steps = s.preview_steps();
    let hidden = |m: &Measurement| {
        m.patterns()
            .iter()
            .any(|&id| s.doc.pattern(id).is_some_and(|p| p.hidden))
    };
    for m in s.doc.measurements.iter().filter(|m| !hidden(m)) {
        let Some(points) = m.points_at(&s.doc, steps) else {
            continue;
        };
        let color = if s.canvas.measure.selected == Some(m.id) {
            pal.selected
        } else {
            pal.accent
        };
        let pts: Vec<Pos2> = points.into_iter().map(to_screen).collect();
        let along = matches!(m.kind, MeasureKind::Along(_));
        draw_line(
            painter,
            &pts,
            along,
            Stroke::new(if along { 3.0 } else { 1.5 }, color),
        );
        if let Some(mid) = middle(&pts) {
            let text = format!("{}  {}", m.name, format_length(m.length_at(&s.doc, steps)));
            label(painter, mid + Px::new(6.0, -6.0), text, color, pal);
        }
    }

    if s.tool != Tool::Measure {
        return;
    }
    let target = s.canvas.hover.and_then(|p| spot_at(s, p));
    if let Some((_, at)) = target {
        painter.circle_stroke(to_screen(at), 5.0, Stroke::new(1.5, pal.preview));
    }
    let Some(start) = s.canvas.measure.start else {
        return;
    };
    let Some(from) = s
        .doc
        .pattern(start.pattern)
        .map(|p| p.pos_at_param(start.param, TOLERANCE))
    else {
        return;
    };
    painter.circle_filled(to_screen(from), 4.0, pal.preview);
    let Some((end, _)) = target else {
        return;
    };
    let along = s.canvas.measure.along != painter.ctx().input(|i| i.modifiers.shift);
    let m = Measurement {
        id: MeasurementId(0),
        name: String::new(),
        kind: kind_between(&s.doc, start, end, along),
    };
    if let Some(points) = m.points_at(&s.doc, 0) {
        let pts: Vec<Pos2> = points.into_iter().map(to_screen).collect();
        draw_line(painter, &pts, along, Stroke::new(1.5, pal.preview));
        if let Some(&last) = pts.last() {
            let text = format_length(m.length_at(&s.doc, 0));
            label(painter, last + Px::new(12.0, 24.0), text, pal.preview, pal);
        }
    }
}

/// With "Show all sizes" on, every size of each graded pattern drawn
/// faintly around the one being previewed, with its size name.
pub fn draw_sizes(painter: &egui::Painter, rect: Rect, s: &Session, pal: &theme::Palette) {
    if !s.canvas.measure.all_sizes {
        return;
    }
    let to_screen = |p: Vec2| s.canvas.to_screen(rect, p);
    let chart = &s.doc.size_chart;
    let shown = s
        .doc
        .patterns
        .iter()
        .filter(|p| !p.hidden && !p.grading.point_deltas.is_empty());
    for pattern in shown {
        for (i, name) in chart.names.iter().enumerate() {
            if i == s.preview_size {
                continue;
            }
            let graded = pattern.graded(chart.steps(i));
            let poly = graded.polyline(TOLERANCE);
            // Smaller sizes in the hint colour, larger in the accent.
            let color = if i < s.preview_size {
                pal.hint
            } else {
                pal.pattern_stroke
            }
            .gamma_multiply(0.6);
            let pts: Vec<Pos2> = poly.iter().map(|&p| to_screen(p)).collect();
            painter.add(Shape::closed_line(pts, Stroke::new(1.0, color)));
            // The name by the size's top-right-most point.
            let corner = poly
                .iter()
                .copied()
                .max_by(|a, b| (a.x - a.y).total_cmp(&(b.x - b.y)));
            if let Some(corner) = corner {
                painter.text(
                    to_screen(corner) + Px::new(4.0, -2.0),
                    Align2::LEFT_BOTTOM,
                    name,
                    FontId::proportional(11.0),
                    color,
                );
            }
        }
    }
}

/// The Measurements section of Properties: how the Measure tool measures,
/// the size overlay, and the document's measurements at the size being
/// previewed, each renamable and removable.
pub fn section(ui: &mut egui::Ui, s: &mut Session) {
    if s.tool != Tool::Measure && s.doc.measurements.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.heading("Measurements");
    if s.tool == Tool::Measure {
        ui.horizontal(|ui| {
            let along = &mut s.canvas.measure.along;
            ui.selectable_value(along, false, "Straight")
                .on_hover_text("Measure the straight distance between two points");
            ui.selectable_value(along, true, "Along outline")
                .on_hover_text("Measure along the outline between two points of one piece");
        });
    }
    ui.checkbox(&mut s.canvas.measure.all_sizes, "Show all sizes")
        .on_hover_text("Draw every graded size faintly around the size being previewed");

    let steps = s.preview_steps();
    let rows: Vec<(MeasurementId, String, Option<f32>)> = s
        .doc
        .measurements
        .iter()
        .map(|m| (m.id, m.name.clone(), m.length_at(&s.doc, steps)))
        .collect();
    let mut rename = None;
    let mut remove = None;
    for (id, name, length) in rows {
        ui.horizontal(|ui| {
            let m = &mut s.canvas.measure;
            let text = match &mut m.editing {
                Some((editing, text)) if *editing == id => text,
                editing => &mut editing.insert((id, name.clone())).1,
            };
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
            if edit.gained_focus() || edit.clicked() {
                m.selected = Some(id);
            }
            if edit.lost_focus() {
                let new = text.trim().to_owned();
                if !new.is_empty() && new != name {
                    rename = Some((id, new));
                }
                m.editing = None;
            } else if !edit.has_focus() {
                m.editing = None;
            }
            ui.label(format_length(length));
            if ui.small_button("Remove").clicked() {
                remove = Some(id);
            }
        });
    }
    if let Some((id, name)) = rename {
        s.execute(RenameMeasurement::new(id, name));
    }
    if let Some(id) = remove {
        s.execute(RemoveMeasurement::new(id));
    }
    if ui
        .button("Chart of all sizes")
        .on_hover_text("Every measurement at every size")
        .clicked()
    {
        s.canvas.measure.chart_open = true;
    }
}

/// The measurement chart: every measurement at every size of the size
/// chart, from the graded outlines.
pub fn chart_window(ctx: &egui::Context, s: &mut Session) {
    let mut open = s.canvas.measure.chart_open;
    egui::Window::new("Measurement chart")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            let rows = chart(&s.doc);
            if rows.is_empty() {
                ui.weak(
                    "No measurements yet: take some with the Measure tool (M) in the 2D window.",
                );
                return;
            }
            let sizes = &s.doc.size_chart;
            egui::Grid::new("measurement_chart")
                .num_columns(sizes.names.len() + 1)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Measurement");
                    for (i, name) in sizes.names.iter().enumerate() {
                        let text = if i == sizes.base {
                            format!("{name} (base)")
                        } else {
                            name.clone()
                        };
                        ui.strong(text);
                    }
                    ui.end_row();
                    for row in rows {
                        ui.label(&row.name);
                        for (i, length) in row.lengths.into_iter().enumerate() {
                            let text = egui::RichText::new(format_length(length));
                            ui.label(if i == s.preview_size {
                                text.strong()
                            } else {
                                text
                            });
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            if ui
                .button("Copy as CSV")
                .on_hover_text("Copy the chart for pasting into a spreadsheet")
                .clicked()
            {
                ui.ctx().copy_text(chart_csv(&s.doc));
            }
        });
    s.canvas.measure.chart_open = open;
}
