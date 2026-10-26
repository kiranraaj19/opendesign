//! 2D pattern window: draft, edit and arrange flat garment pieces.
//!
//! Document units are millimetres; `zoom` is screen pixels per millimetre.

use egui::{
    Align2, Color32, FontId, Key, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2 as Px,
};
use od_core::document::TOLERANCE;
use od_core::{
    geometry, AddInternalLine, AddNotch, AddSewingLine, Document, InsertPoint, MovePattern,
    MovePoint, OutlinePoint, Pattern, PatternId, PatternValue, PointKind, RemoveNotch,
    RemoveSewingLine, SeamEdge, SetPointKind, Vec2,
};

use crate::app::{Session, Tool};
use crate::theme;

/// Rectangles smaller than this (in mm) are treated as accidental clicks.
const MIN_RECT_MM: f32 = 5.0;
/// How close (in screen pixels) the pointer must be to grab a point or edge.
const PICK_PX: f32 = 8.0;
/// How close to snap to an existing point, in screen pixels.
const SNAP_PX: f32 = 10.0;

/// What the 2D cursor snaps to while drafting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snap {
    pub grid: bool,
    pub points: bool,
}

impl Default for Snap {
    fn default() -> Self {
        Self {
            grid: true,
            points: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Snapped {
    Point,
    Grid,
}

pub struct Canvas2D {
    pan: Px,
    zoom: f32,
    drag: Option<Drag>,
    /// Where the canvas was drawn last frame.
    rect: Rect,
    pub snap: Snap,
    /// Points placed so far by the pen or internal-line tool.
    draft: Vec<OutlinePoint>,
    /// The tool that placed `draft`; switching tools drops the draft.
    draft_tool: Tool,
    /// Pattern an internal line being drawn belongs to.
    draft_owner: Option<PatternId>,
    /// Snapped pointer position, for previews.
    cursor: Option<(Vec2, Option<Snapped>)>,
    /// Outline point selected with the edit-points tool.
    pub selected_point: Option<(PatternId, usize)>,
    /// First edge picked with the sew tool, waiting for a second to match.
    sew_pending: Option<SeamEdge>,
}

enum Drag {
    Move {
        id: PatternId,
        delta: Vec2,
    },
    Rect {
        start: Vec2,
        end: Vec2,
    },
    Point {
        id: PatternId,
        index: usize,
        /// From the pointer to the point, so the point doesn't jump.
        grab: Vec2,
        to: Vec2,
    },
}

impl Default for Canvas2D {
    fn default() -> Self {
        Self {
            pan: Px::ZERO,
            zoom: 0.6,
            drag: None,
            rect: Rect::NOTHING,
            snap: Snap::default(),
            draft: Vec::new(),
            draft_tool: Tool::Select,
            draft_owner: None,
            cursor: None,
            selected_point: None,
            sew_pending: None,
        }
    }
}

impl Canvas2D {
    #[cfg(test)]
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Points placed so far by the pen or line tool.
    #[cfg(test)]
    pub fn draft_points(&self) -> Vec<Vec2> {
        self.draft.iter().map(|p| p.pos).collect()
    }

    /// Document point at the centre of the visible canvas.
    pub fn view_center(&self) -> Vec2 {
        if self.rect.is_positive() {
            self.to_doc(self.rect, self.rect.center())
        } else {
            Vec2::ZERO
        }
    }

    /// Screen position of a document point, as of the last frame.
    #[cfg(test)]
    pub fn screen_pos(&self, p: Vec2) -> Pos2 {
        self.to_screen(self.rect, p)
    }

    fn to_screen(&self, rect: Rect, p: Vec2) -> Pos2 {
        rect.center() + self.pan + Px::new(p.x, p.y) * self.zoom
    }

    fn to_doc(&self, rect: Rect, p: Pos2) -> Vec2 {
        let v = (p - rect.center() - self.pan) / self.zoom;
        Vec2::new(v.x, v.y)
    }

    /// Screen pixels as millimetres at the current zoom.
    fn px(&self, px: f32) -> f32 {
        px / self.zoom
    }

    /// Spacing of the finest grid lines drawn, which is also the snap step.
    pub fn grid_step(&self) -> f32 {
        let mut step_mm = 10.0;
        while step_mm * self.zoom < 12.0 {
            step_mm *= 5.0;
        }
        step_mm
    }

    /// Snap `p` to a nearby outline or draft point, else to the grid.
    /// `skip` is a point being dragged, which must not snap to itself.
    fn snap(
        &self,
        doc: &Document,
        p: Vec2,
        skip: Option<(PatternId, usize)>,
    ) -> (Vec2, Option<Snapped>) {
        if self.snap.points {
            let outline = doc.patterns.iter().flat_map(|pat| {
                pat.outline
                    .iter()
                    .enumerate()
                    .filter(move |(i, _)| skip != Some((pat.id, *i)))
                    .map(|(_, op)| op.pos)
            });
            let draft = self.draft.iter().map(|op| op.pos);
            let nearest = outline
                .chain(draft)
                .map(|q| (q.distance(p), q))
                .filter(|(d, _)| *d <= self.px(SNAP_PX))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, q)) = nearest {
                return (q, Some(Snapped::Point));
            }
        }
        if self.snap.grid {
            let step = self.grid_step();
            let snapped = Vec2::new((p.x / step).round() * step, (p.y / step).round() * step);
            return (snapped, Some(Snapped::Grid));
        }
        (p, None)
    }

    /// The outline point under `p`, preferring the selected pattern.
    fn point_at(
        &self,
        doc: &Document,
        selection: Option<PatternId>,
        p: Vec2,
    ) -> Option<(PatternId, usize)> {
        let reach = self.px(PICK_PX);
        let nearest_in = |pat: &Pattern| {
            pat.outline
                .iter()
                .enumerate()
                .map(|(i, op)| (op.pos.distance(p), i))
                .filter(|(d, _)| *d <= reach)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(d, i)| (d, (pat.id, i)))
        };
        if let Some(hit) = selection
            .and_then(|id| doc.pattern(id))
            .and_then(nearest_in)
        {
            return Some(hit.1);
        }
        doc.patterns
            .iter()
            .rev()
            .filter_map(nearest_in)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, hit)| hit)
    }

    /// The pattern whose outline passes nearest `p` (within reach),
    /// preferring the selected one, with its flattened outline.
    fn edge_at(&self, doc: &Document, selection: Option<PatternId>, p: Vec2) -> Option<EdgeHit> {
        let reach = self.px(PICK_PX);
        let order = selection
            .into_iter()
            .chain(doc.patterns.iter().rev().map(|pat| pat.id));
        for id in order {
            let Some(pat) = doc.pattern(id) else { continue };
            let flat = pat.flatten(TOLERANCE);
            let n = flat.len();
            let mut best: Option<EdgeHit> = None;
            for i in 0..n {
                let (a, b) = (flat[i], flat[(i + 1) % n]);
                let (q, t) = geometry::closest_on_segment(a.pos, b.pos, p);
                let gap = q.distance(p);
                if gap <= reach && best.as_ref().is_none_or(|h| gap < h.gap) {
                    let count = pat.outline.len() as f32;
                    let mut end = b.param;
                    if end <= a.param {
                        end += count; // wrapped past the start
                    }
                    best = Some(EdgeHit {
                        id,
                        gap,
                        pos: q,
                        param: (a.param + (end - a.param) * t).rem_euclid(count),
                    });
                }
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
}

struct EdgeHit {
    id: PatternId,
    gap: f32,
    pos: Vec2,
    /// Outline parameter (see [`od_core::document::FlatPoint`]).
    param: f32,
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "2D pattern canvas")
    });
    let rect = response.rect;
    s.canvas.rect = rect;
    let pal = theme::palette(ui.visuals().dark_mode);

    // Zoom around the cursor so the point under it stays put.
    if let Some(hover) = response.hover_pos() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let anchor = s.canvas.to_doc(rect, hover);
            s.canvas.zoom = (s.canvas.zoom * (scroll * 0.002).exp()).clamp(0.05, 20.0);
            s.canvas.pan += hover - s.canvas.to_screen(rect, anchor);
        }
    }
    if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
        s.canvas.pan += response.drag_delta();
    }

    if s.canvas.draft_tool != s.tool {
        s.canvas.draft.clear();
        s.canvas.draft_tool = s.tool;
    }
    if s.tool != Tool::EditPoints {
        s.canvas.selected_point = None;
    }
    s.canvas.cursor = response
        .hover_pos()
        .map(|p| s.canvas.snap(&s.doc, s.canvas.to_doc(rect, p), None));

    // Single keys must not fire while typing in a text field.
    let typing = ui.memory(|m| m.focused().is_some());
    match s.tool {
        Tool::Select | Tool::Rectangle => handle_drag_tools(&response, rect, s),
        Tool::Pen | Tool::InternalLine => handle_draft(ui, &response, rect, s, typing),
        Tool::EditPoints => handle_edit_points(&response, rect, s),
        Tool::Notch => handle_notch(&response, rect, s),
        Tool::Sew => handle_sew(&response, rect, s),
    }

    // Background and grid.
    painter.rect_filled(rect, 0.0, pal.canvas_bg);
    draw_grid(&painter, rect, &s.canvas, pal);

    for pattern in &s.doc.patterns {
        draw_pattern(&painter, rect, s, pattern, pal);
    }
    draw_sewing_lines(&painter, rect, s, pal);
    draw_overlays(&painter, rect, s, pal);

    let hint = match s.tool {
        Tool::Select => "Click to select · drag to move",
        Tool::Rectangle => "Drag to draw a rectangle",
        Tool::Pen => {
            "Click to place points (Shift+click: curve point) · click the first point or Enter to close · Esc cancels"
        }
        Tool::EditPoints => {
            "Drag points · double-click an edge to add a point, a point to curve it · C toggles curve · Del deletes"
        }
        Tool::InternalLine => {
            "Click inside a pattern to draw a line · Enter or double-click finishes · Esc cancels"
        }
        Tool::Notch => "Click an outline to add a notch · click a notch to remove it",
        Tool::Sew => match s.canvas.sew_pending {
            Some(_) => "Click the matching edge on another pattern (or the same one) to sew · Esc cancels",
            None => "Click an edge to start a seam · click a sewn edge to unsew it",
        },
    };
    painter.text(
        rect.left_top() + Px::new(10.0, 8.0),
        Align2::LEFT_TOP,
        format!(
            "{hint} · scroll to zoom · right-drag to pan · {:.0}%",
            s.canvas.zoom * 100.0
        ),
        FontId::proportional(12.0),
        pal.hint,
    );
}

/// The press position of the current primary drag, in document space.
fn press_origin(response: &egui::Response, rect: Rect, canvas: &Canvas2D) -> Option<Vec2> {
    response
        .ctx
        .input(|i| i.pointer.press_origin())
        .map(|o| canvas.to_doc(rect, o))
}

fn handle_drag_tools(response: &egui::Response, rect: Rect, s: &mut Session) {
    let pointer = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p));

    if response.clicked() && s.tool == Tool::Select {
        s.selection = pointer.and_then(|p| s.doc.pattern_at(p));
    }

    // egui reports a drag only after the pointer has moved a few pixels, so
    // anchor it at the press point and count the movement already made.
    if response.drag_started_by(PointerButton::Primary) {
        if let (Some(origin), Some(p)) = (press_origin(response, rect, &s.canvas), pointer) {
            s.canvas.drag = match s.tool {
                Tool::Select => {
                    s.selection = s.doc.pattern_at(origin);
                    s.selection.map(|id| Drag::Move {
                        id,
                        delta: p - origin,
                    })
                }
                _ => Some(Drag::Rect {
                    start: s.canvas.snap(&s.doc, origin, None).0,
                    end: s.canvas.snap(&s.doc, p, None).0,
                }),
            };
        }
    } else if response.dragged_by(PointerButton::Primary) {
        let step = response.drag_delta() / s.canvas.zoom;
        let snapped = pointer.map(|p| s.canvas.snap(&s.doc, p, None).0);
        match &mut s.canvas.drag {
            Some(Drag::Move { delta, .. }) => *delta += Vec2::new(step.x, step.y),
            Some(Drag::Rect { end, .. }) => {
                if let Some(p) = snapped {
                    *end = p;
                }
            }
            _ => {}
        }
    }

    // Only a finished drag becomes a command, so one drag is one undo step.
    if response.drag_stopped() {
        match s.canvas.drag.take() {
            Some(Drag::Move { id, delta }) if delta != Vec2::ZERO => {
                s.execute(MovePattern::new(id, delta));
            }
            Some(Drag::Rect { start, end }) => {
                let size = end - start;
                if size.x.abs() >= MIN_RECT_MM && size.y.abs() >= MIN_RECT_MM {
                    let name = next_pattern_name(&s.doc);
                    s.add_pattern(name, geometry::rectangle(start, end));
                }
            }
            _ => {}
        }
    }
}

fn next_pattern_name(doc: &Document) -> String {
    format!("Pattern {}", doc.patterns.len() + 1)
}

/// Pen (closed outline) and internal line (open line) drafting.
fn handle_draft(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    s: &mut Session,
    typing: bool,
) {
    let pen = s.tool == Tool::Pen;
    if response.clicked() {
        if let Some(screen) = response.interact_pointer_pos() {
            let raw = s.canvas.to_doc(rect, screen);
            let (p, _) = s.canvas.snap(&s.doc, raw, None);
            let near_first = s
                .canvas
                .draft
                .first()
                .is_some_and(|f| s.canvas.to_screen(rect, f.pos).distance(screen) <= SNAP_PX);
            if pen && s.canvas.draft.len() >= 3 && near_first {
                finish_draft(s);
                return;
            }
            if !pen && s.canvas.draft.is_empty() {
                // A line belongs to the pattern it starts in (or on).
                s.canvas.draft_owner = s
                    .doc
                    .pattern_at(raw)
                    .or_else(|| s.canvas.edge_at(&s.doc, s.selection, raw).map(|h| h.id));
                if s.canvas.draft_owner.is_none() {
                    return;
                }
                s.selection = s.canvas.draft_owner;
            }
            let curve = pen && ui.input(|i| i.modifiers.shift);
            let point = if curve {
                OutlinePoint::curve(p)
            } else {
                OutlinePoint::corner(p)
            };
            // A double-click lands twice on the same spot.
            if s.canvas.draft.last().is_none_or(|last| last.pos != p) {
                s.canvas.draft.push(point);
            }
        }
    }
    if !pen && response.double_clicked() {
        finish_draft(s);
    }
    if !typing {
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            finish_draft(s);
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            s.canvas.draft.clear();
        }
    }
}

/// Turn the draft into a pattern or internal line, if it is one.
fn finish_draft(s: &mut Session) {
    let draft = std::mem::take(&mut s.canvas.draft);
    match s.tool {
        Tool::Pen if draft.len() >= 3 => {
            let probe = Pattern::new(PatternId(0), "", draft.clone());
            if probe.area() > 1.0 {
                let name = next_pattern_name(&s.doc);
                s.add_outline(name, draft);
            }
        }
        Tool::InternalLine if draft.len() >= 2 => {
            if let Some(id) = s.canvas.draft_owner.take() {
                s.execute(AddInternalLine::new(
                    id,
                    draft.iter().map(|p| p.pos).collect(),
                ));
            }
        }
        _ => {}
    }
}

fn handle_edit_points(response: &egui::Response, rect: Rect, s: &mut Session) {
    let pointer = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p));

    if response.clicked() {
        if let Some(p) = pointer {
            match s.canvas.point_at(&s.doc, s.selection, p) {
                Some((id, index)) => {
                    s.selection = Some(id);
                    s.canvas.selected_point = Some((id, index));
                }
                None => {
                    s.selection = s.doc.pattern_at(p);
                    s.canvas.selected_point = None;
                }
            }
        }
    }
    if response.double_clicked() {
        if let Some(p) = pointer {
            if let Some((id, index)) = s.canvas.point_at(&s.doc, s.selection, p) {
                let kind = s
                    .doc
                    .pattern(id)
                    .and_then(|pat| pat.outline.get(index))
                    .map(|p| p.kind);
                if let Some(kind) = kind {
                    s.execute(SetPointKind::new(id, index, kind.toggled()));
                }
            } else if let Some(hit) = s.canvas.edge_at(&s.doc, s.selection, p) {
                let count = s.doc.pattern(hit.id).map_or(0, |pat| pat.outline.len());
                let index = (hit.param.floor() as usize % count.max(1)) + 1;
                s.execute(InsertPoint::new(
                    hit.id,
                    index,
                    OutlinePoint::corner(hit.pos),
                ));
                s.selection = Some(hit.id);
                s.canvas.selected_point = Some((hit.id, index));
            }
        }
    }

    if response.drag_started_by(PointerButton::Primary) {
        if let Some(origin) = press_origin(response, rect, &s.canvas) {
            if let Some((id, index)) = s.canvas.point_at(&s.doc, s.selection, origin) {
                let pos = s
                    .doc
                    .pattern(id)
                    .map_or(origin, |pat| pat.outline[index].pos);
                s.selection = Some(id);
                s.canvas.selected_point = Some((id, index));
                s.canvas.drag = Some(Drag::Point {
                    id,
                    index,
                    grab: pos - origin,
                    to: pos,
                });
            }
        }
    }
    if response.dragged_by(PointerButton::Primary) {
        if let (
            Some(p),
            Some(Drag::Point {
                id, index, grab, ..
            }),
        ) = (pointer, &s.canvas.drag)
        {
            let target = s.canvas.snap(&s.doc, p + *grab, Some((*id, *index))).0;
            if let Some(Drag::Point { to, .. }) = &mut s.canvas.drag {
                *to = target;
            }
        }
    }
    if response.drag_stopped() {
        if let Some(Drag::Point { id, index, to, .. }) = s.canvas.drag.take() {
            let from = s
                .doc
                .pattern(id)
                .and_then(|pat| pat.outline.get(index))
                .map(|p| p.pos);
            if from.is_some_and(|f| f != to) {
                s.execute(MovePoint::new(id, index, to));
            }
        }
    }
}

fn handle_notch(response: &egui::Response, rect: Rect, s: &mut Session) {
    if !response.clicked() {
        return;
    }
    let Some(p) = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p))
    else {
        return;
    };
    let Some(hit) = s.canvas.edge_at(&s.doc, s.selection, p) else {
        return;
    };
    let Some(pattern) = s.doc.pattern(hit.id) else {
        return;
    };
    s.selection = Some(hit.id);
    let poly = pattern.polyline(TOLERANCE);
    let reach = s.canvas.px(PICK_PX);
    let existing = pattern.notches.iter().position(|n| {
        geometry::point_at_distance(&poly, n.at).is_some_and(|(q, _)| q.distance(p) <= reach)
    });
    match existing {
        Some(index) => s.execute(RemoveNotch::new(hit.id, index)),
        None => {
            if let Some((at, _)) = geometry::distance_along(&poly, hit.pos) {
                s.execute(AddNotch::new(hit.id, at));
            }
        }
    }
}

/// Click an outline edge to pick it for sewing; click a second edge to sew
/// it to the first; click an already-sewn edge to unsew it.
fn handle_sew(response: &egui::Response, rect: Rect, s: &mut Session) {
    if response.ctx.input(|i| i.key_pressed(Key::Escape)) {
        s.canvas.sew_pending = None;
    }
    if !response.clicked() {
        return;
    }
    let Some(p) = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p))
    else {
        return;
    };
    let Some(hit) = s.canvas.edge_at(&s.doc, s.selection, p) else {
        return;
    };
    let Some(pattern) = s.doc.pattern(hit.id) else {
        return;
    };
    s.selection = Some(hit.id);
    let (start, end) = pattern.segment_at(hit.param);
    let edge = SeamEdge {
        pattern: hit.id,
        start,
        end,
    };

    // Clicking an edge that is already sewn unsews it.
    let sewn = s
        .doc
        .sewing_lines
        .iter()
        .find(|sl| sl.a == edge || sl.b == edge);
    if let Some(sewn) = sewn {
        s.canvas.sew_pending = None;
        s.execute(RemoveSewingLine::new(sewn.id));
        return;
    }

    match s.canvas.sew_pending.take() {
        Some(first) if first == edge => {} // clicked the same edge again: cancel
        Some(first) => s.execute(AddSewingLine::new(first, edge)),
        None => s.canvas.sew_pending = Some(edge),
    }
}

/// The pattern as it should look this frame, with any drag in progress.
fn previewed<'a>(s: &Session, pattern: &'a Pattern) -> std::borrow::Cow<'a, Pattern> {
    match &s.canvas.drag {
        Some(Drag::Move { id, delta }) if *id == pattern.id => {
            let mut p = pattern.clone();
            p.translate(*delta);
            std::borrow::Cow::Owned(p)
        }
        Some(Drag::Point { id, index, to, .. }) if *id == pattern.id => {
            let mut p = pattern.clone();
            if let Some(op) = p.outline.get_mut(*index) {
                op.pos = *to;
            }
            std::borrow::Cow::Owned(p)
        }
        _ => std::borrow::Cow::Borrowed(pattern),
    }
}

fn draw_pattern(
    painter: &egui::Painter,
    rect: Rect,
    s: &Session,
    pattern: &Pattern,
    pal: &theme::Palette,
) {
    let pattern = previewed(s, pattern);
    let canvas = &s.canvas;
    let to_screen = |p: Vec2| canvas.to_screen(rect, p);
    let poly = pattern.polyline(TOLERANCE);
    if poly.len() < 2 {
        return;
    }
    let points: Vec<Pos2> = poly.iter().map(|p| to_screen(*p)).collect();
    let selected = s.selection == Some(pattern.id);

    // Seam allowance, dashed outside the outline.
    let allowance = s
        .props
        .preview(pattern.id, PatternValue::SeamAllowance)
        .unwrap_or(pattern.seam_allowance);
    if allowance > 0.0 {
        let mut seam: Vec<Pos2> = geometry::offset_outline(&poly, allowance)
            .into_iter()
            .map(to_screen)
            .collect();
        if let Some(&first) = seam.first() {
            seam.push(first);
            painter.extend(Shape::dashed_line(
                &seam,
                Stroke::new(1.0, pal.pattern_stroke),
                6.0,
                4.0,
            ));
        }
    }

    // Fill: triangulated, so concave necklines and armholes come out right.
    let mut mesh = egui::Mesh::default();
    for p in &points {
        mesh.colored_vertex(*p, pal.pattern_fill);
    }
    for [a, b, c] in geometry::triangulate(&poly) {
        mesh.add_triangle(a, b, c);
    }
    painter.add(Shape::mesh(mesh));

    let stroke = if selected {
        Stroke::new(2.5, pal.selected)
    } else {
        Stroke::new(1.5, pal.pattern_stroke)
    };
    painter.add(Shape::closed_line(points, stroke));

    // Notches: short ticks across the outline into the allowance.
    let ccw = geometry::signed_area(&poly) > 0.0;
    let tick = allowance.max(canvas.px(8.0));
    for notch in &pattern.notches {
        if let Some((at, dir)) = geometry::point_at_distance(&poly, notch.at) {
            let out = if ccw { -dir.perp() } else { dir.perp() };
            painter.line_segment(
                [to_screen(at), to_screen(at + out * tick)],
                Stroke::new(2.0, stroke.color),
            );
        }
    }

    for line in &pattern.internal_lines {
        let pts = line.points.iter().map(|p| to_screen(*p)).collect();
        painter.add(Shape::line(pts, Stroke::new(1.5, pal.pattern_stroke)));
    }

    if let Some((min, max)) = geometry::bounds(&poly) {
        let angle = s
            .props
            .preview(pattern.id, PatternValue::GrainAngle)
            .unwrap_or(pattern.grain_angle);
        draw_grainline(
            painter,
            rect,
            canvas,
            (min + max) * 0.5,
            max - min,
            angle,
            pal.hint,
        );
        painter.text(
            to_screen((min + max) * 0.5) + Px::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            &pattern.name,
            FontId::proportional(13.0),
            pal.text,
        );
    }

    // Point handles: on the selected pattern, or on all of them while
    // editing points.
    if selected || s.tool == Tool::EditPoints {
        draw_handles(painter, rect, s, &pattern, pal);
    }
}

/// Highlight sewn edges along their outlines, and the edge waiting for its
/// match while using the sew tool. A seam whose two edges differ in length
/// by more than [`od_core::document::LENGTH_MISMATCH_TOLERANCE`] is drawn in
/// a warning colour, with the difference labelled.
fn draw_sewing_lines(painter: &egui::Painter, rect: Rect, s: &Session, pal: &theme::Palette) {
    let to_screen = |p: Vec2| s.canvas.to_screen(rect, p);
    let edge_points = |edge: &SeamEdge| -> Vec<Pos2> {
        s.doc
            .pattern(edge.pattern)
            .map(|pattern| pattern.segment_points(edge.start, edge.end, TOLERANCE))
            .unwrap_or_default()
            .into_iter()
            .map(to_screen)
            .collect()
    };
    let draw_edge = |edge: &SeamEdge, stroke: Stroke| {
        let points = edge_points(edge);
        if points.len() >= 2 {
            painter.add(Shape::line(points, stroke));
        }
    };
    for line in &s.doc.sewing_lines {
        let mismatch = line.length_mismatch(&s.doc);
        if line.is_length_mismatched(&s.doc) {
            let stroke = Stroke::new(3.0, pal.warning);
            draw_edge(&line.a, stroke);
            draw_edge(&line.b, stroke);
            let a_points = edge_points(&line.a);
            if let (Some(mismatch), Some(&mid)) = (mismatch, a_points.get(a_points.len() / 2)) {
                painter.text(
                    mid + Px::new(8.0, -8.0),
                    Align2::LEFT_BOTTOM,
                    format!("seam mismatch: {:+.0} mm", mismatch),
                    FontId::proportional(12.0),
                    pal.warning,
                );
            }
        } else {
            let stroke = Stroke::new(3.0, pal.accent);
            draw_edge(&line.a, stroke);
            draw_edge(&line.b, stroke);
        }
    }
    if let Some(pending) = &s.canvas.sew_pending {
        draw_edge(pending, Stroke::new(3.0, pal.selected));
    }
}

/// Double-headed arrow through `center`, `angle` degrees clockwise from
/// straight down, scaled to the pattern's `size`.
fn draw_grainline(
    painter: &egui::Painter,
    rect: Rect,
    canvas: &Canvas2D,
    center: Vec2,
    size: Vec2,
    angle: f32,
    color: Color32,
) {
    let (sin, cos) = angle.to_radians().sin_cos();
    let dir = Vec2::new(-sin, cos);
    let extent = dir.x.abs() * size.x + dir.y.abs() * size.y;
    let half = dir * (extent * 0.3);
    let (a, b) = (
        canvas.to_screen(rect, center - half),
        canvas.to_screen(rect, center + half),
    );
    let stroke = Stroke::new(1.5, color);
    painter.line_segment([a, b], stroke);
    let d = (b - a).normalized();
    let side = d.rot90();
    for (tip, back) in [(a, d), (b, -d)] {
        for wing in [side, -side] {
            painter.line_segment([tip, tip + back * 9.0 + wing * 5.0], stroke);
        }
    }
}

fn draw_handles(
    painter: &egui::Painter,
    rect: Rect,
    s: &Session,
    pattern: &Pattern,
    pal: &theme::Palette,
) {
    let to_screen = |p: Vec2| s.canvas.to_screen(rect, p);
    let n = pattern.outline.len();
    // Control polygon around curve points, as in a vector editor.
    for (i, op) in pattern.outline.iter().enumerate() {
        if op.kind == PointKind::Curve {
            for j in [(i + n - 1) % n, (i + 1) % n] {
                let other = pattern.outline[j].pos;
                painter.extend(Shape::dashed_line(
                    &[to_screen(op.pos), to_screen(other)],
                    Stroke::new(1.0, pal.hint),
                    4.0,
                    3.0,
                ));
            }
        }
    }
    for (i, op) in pattern.outline.iter().enumerate() {
        let at = to_screen(op.pos);
        let is_selected = s.canvas.selected_point == Some((pattern.id, i));
        let r = if is_selected { 5.5 } else { 3.5 };
        match op.kind {
            PointKind::Corner => {
                painter.circle_filled(at, r, pal.selected);
            }
            PointKind::Curve => {
                painter.circle(at, r, pal.canvas_bg, Stroke::new(1.5, pal.selected));
            }
        }
        if is_selected {
            painter.circle_stroke(at, r + 3.0, Stroke::new(1.5, pal.selected));
        }
    }
}

/// Drafts in progress, drag previews, snap marker and length readout.
fn draw_overlays(painter: &egui::Painter, rect: Rect, s: &Session, pal: &theme::Palette) {
    let canvas = &s.canvas;
    let to_screen = |p: Vec2| canvas.to_screen(rect, p);
    let readout = |from: Vec2, to: Vec2| {
        painter.text(
            to_screen(to) + Px::new(12.0, 12.0),
            Align2::LEFT_TOP,
            measure(from, to),
            FontId::monospace(12.0),
            pal.preview,
        );
    };

    if let Some(Drag::Rect { start, end }) = &canvas.drag {
        let points = geometry::rectangle(*start, *end)
            .into_iter()
            .map(to_screen)
            .collect();
        painter.add(Shape::closed_line(points, Stroke::new(1.5, pal.preview)));
        let size = *end - *start;
        painter.text(
            to_screen(*end) + Px::new(10.0, 10.0),
            Align2::LEFT_TOP,
            format!("{:.0} × {:.0} mm", size.x.abs(), size.y.abs()),
            FontId::monospace(12.0),
            pal.preview,
        );
    }

    if let Some(Drag::Point { to, .. }) = &canvas.drag {
        painter.text(
            to_screen(*to) + Px::new(12.0, 12.0),
            Align2::LEFT_TOP,
            format!("{:.1}, {:.1} mm", to.x, to.y),
            FontId::monospace(12.0),
            pal.preview,
        );
    }

    let drafting = matches!(s.tool, Tool::Pen | Tool::InternalLine);
    if drafting && !canvas.draft.is_empty() {
        let stroke = Stroke::new(1.5, pal.preview);
        let mut pts: Vec<Pos2> = canvas.draft.iter().map(|p| to_screen(p.pos)).collect();
        if let Some((cursor, _)) = canvas.cursor {
            pts.push(to_screen(cursor));
            if let Some(last) = canvas.draft.last() {
                readout(last.pos, cursor);
            }
        }
        painter.add(Shape::line(pts, stroke));
        for (i, p) in canvas.draft.iter().enumerate() {
            let at = to_screen(p.pos);
            // The first pen point is where a click closes the outline.
            let r = if i == 0 && s.tool == Tool::Pen {
                6.0
            } else {
                3.5
            };
            match p.kind {
                PointKind::Corner => painter.circle_filled(at, r, pal.preview),
                PointKind::Curve => painter.circle_stroke(at, r, stroke),
            };
        }
    }

    if drafting || s.tool == Tool::Rectangle || matches!(canvas.drag, Some(Drag::Point { .. })) {
        let marker = match &canvas.drag {
            Some(Drag::Point { to, .. }) => Some((*to, Some(Snapped::Point))),
            _ => canvas.cursor,
        };
        match marker {
            Some((p, Some(Snapped::Point))) => {
                let r = Rect::from_center_size(to_screen(p), Px::splat(12.0));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.5, pal.preview),
                    egui::StrokeKind::Middle,
                );
            }
            Some((p, Some(Snapped::Grid))) => {
                let c = to_screen(p);
                let stroke = Stroke::new(1.0, pal.preview);
                painter.line_segment([c - Px::new(6.0, 0.0), c + Px::new(6.0, 0.0)], stroke);
                painter.line_segment([c - Px::new(0.0, 6.0), c + Px::new(0.0, 6.0)], stroke);
            }
            _ => {}
        }
    }
}

/// Length and angle of the segment being drawn. Angles read
/// counter-clockwise from the right, as on paper (the screen's y is down).
fn measure(from: Vec2, to: Vec2) -> String {
    let d = to - from;
    // 0 - y rather than -y, so a level line pointing left reads 180°, not -180°.
    let angle = (0.0 - d.y).atan2(d.x).to_degrees();
    format!("{:.1} mm  {angle:.1}°", d.length())
}

fn draw_grid(painter: &egui::Painter, rect: Rect, canvas: &Canvas2D, pal: &theme::Palette) {
    // Pick a grid spacing that stays readable at any zoom.
    let step_mm = canvas.grid_step();
    let min = canvas.to_doc(rect, rect.left_top());
    let max = canvas.to_doc(rect, rect.right_bottom());

    let mut x = (min.x / step_mm).floor() * step_mm;
    while x <= max.x {
        let major = (x / step_mm).round() as i64 % 5 == 0;
        let color = if x == 0.0 {
            pal.axis
        } else if major {
            pal.grid_major
        } else {
            pal.grid_minor
        };
        let sx = canvas.to_screen(rect, Vec2::new(x, 0.0)).x;
        painter.line_segment(
            [Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())],
            Stroke::new(1.0, color),
        );
        x += step_mm;
    }
    let mut y = (min.y / step_mm).floor() * step_mm;
    while y <= max.y {
        let major = (y / step_mm).round() as i64 % 5 == 0;
        let color = if y == 0.0 {
            pal.axis
        } else if major {
            pal.grid_major
        } else {
            pal.grid_minor
        };
        let sy = canvas.to_screen(rect, Vec2::new(0.0, y)).y;
        painter.line_segment(
            [Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)],
            Stroke::new(1.0, color),
        );
        y += step_mm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_shows_length_and_paper_angle() {
        let o = Vec2::ZERO;
        assert_eq!(measure(o, Vec2::new(30.0, -40.0)), "50.0 mm  53.1°");
        assert_eq!(measure(o, Vec2::new(-10.0, 0.0)), "10.0 mm  180.0°");
        assert_eq!(measure(o, Vec2::new(0.0, 25.0)), "25.0 mm  -90.0°");
    }

    #[test]
    fn grid_step_stays_readable() {
        let mut c = Canvas2D::default();
        for zoom in [0.05, 0.6, 1.2, 5.0] {
            c.zoom = zoom;
            assert!(c.grid_step() * zoom >= 12.0, "zoom {zoom}");
        }
        c.zoom = 1.2;
        assert_eq!(c.grid_step(), 10.0);
    }
}
