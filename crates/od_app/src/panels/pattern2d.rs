//! 2D pattern window: draft, edit and arrange flat garment pieces.
//!
//! Document units are millimetres; `zoom` is screen pixels per millimetre.

use egui::{
    Align2, Color32, FontId, Key, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2 as Px,
};
use od_core::document::{normalize_degrees, rotate_point, TOLERANCE};
use od_core::edit::PleatKind;
use od_core::{
    edit, geometry, AddDart, AddInternalLine, AddNotch, AddSewingLine, AddTack, AddTopstitch,
    AddTrim, Dart, Document, InsertPoint, MovePattern, MovePoint, OutlinePoint, Pattern, PatternId,
    PatternValue, PointKind, RemoveNotch, RemoveSewingLine, RemoveTack, RemoveTrim, RotatePattern,
    SeamEdge, SetPointKind, SetSeamEdge, SewingLine, SewingLineId, Side, TackId, TackPoint,
    TackTarget, TrimKind, Vec2,
};

use crate::app::{Session, Tool};
use crate::theme;

/// Rectangles smaller than this (in mm) are treated as accidental clicks.
const MIN_RECT_MM: f32 = 5.0;
/// How close (in screen pixels) the pointer must be to grab a point or edge.
const PICK_PX: f32 = 8.0;
/// How close to snap to an existing point, in screen pixels.
const SNAP_PX: f32 = 10.0;

/// Finest grid spacing, in mm, unless Preferences say otherwise.
pub const DEFAULT_GRID_MM: f32 = 10.0;

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
    /// Finest grid spacing, in mm (see [`Canvas2D::grid_step`]).
    pub grid_mm: f32,
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
    /// What the sew tool is building: a seam (two sides of edges, each
    /// possibly more than one for a one-to-many seam) or a tack (a single
    /// point waiting for its match).
    sew_pending: Option<SewBuild>,
    /// Where a sew-tool drag began, in doc space. Captured as the drag
    /// starts, since the pointer's press position is no longer available
    /// by the time it is released.
    sew_drag_start: Option<Vec2>,
    /// Unsnapped pointer position, for hover highlights and readouts.
    pub(crate) hover: Option<Vec2>,
    /// Fit the selected patterns in view on the next frame.
    pub frame_selection: bool,
    /// Pan to this pattern on the next frame if it is out of view, e.g.
    /// after it was picked in the 3D view.
    pub reveal: Option<PatternId>,
    /// How wide new darts are, in mm: across the V at the outline, or
    /// across the middle of a diamond.
    pub dart_width: f32,
    /// How far an outline dart reaches into the piece, in mm.
    pub dart_depth: f32,
    pub pleat: PleatSettings,
    /// The Measure tool, size overlay and measurement chart.
    pub measure: crate::panels::measure2d::Measure2D,
    /// The garment detail tools (zippers).
    pub details: crate::panels::details::Details2D,
}

/// What the Pleat tool lays out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PleatSettings {
    pub kind: PleatKind,
    pub count: usize,
    /// Between neighbouring fold lines, in mm.
    pub depth: f32,
    /// How far each fold turns, in degrees.
    pub angle: f32,
    /// Tack each pleat's folds together at the end the drag started from.
    pub tack: bool,
}

impl Default for PleatSettings {
    fn default() -> Self {
        Self {
            kind: PleatKind::Knife,
            count: 1,
            depth: 30.0,
            angle: od_core::Fold::DEFAULT_ANGLE,
            tack: true,
        }
    }
}

/// What the sew tool has picked so far.
#[derive(Debug, Clone, PartialEq)]
enum SewBuild {
    /// Two sides being built up by edge picks; `active` says which one the
    /// next pick joins (or removes from, if already present there).
    Edges {
        sides: [Vec<SeamEdge>; 2],
        active: usize,
    },
    /// A single point, waiting for a second to tack to.
    Point(TackPoint),
}

enum Drag {
    /// Moving every selected pattern.
    Move {
        ids: Vec<PatternId>,
        delta: Vec2,
    },
    /// Turning every selected pattern about its own centre by the angle
    /// the pointer has swept around `pivot` (the main pattern's centre)
    /// since `from`.
    Rotate {
        ids: Vec<PatternId>,
        pivot: Vec2,
        from: Vec2,
        degrees: f32,
    },
    /// Selecting the patterns a box touches, adding to the selection if
    /// `add`.
    Select {
        start: Vec2,
        end: Vec2,
        add: bool,
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
    /// A straight line drawn inside one pattern, for a diamond dart.
    Line {
        owner: PatternId,
        start: Vec2,
        end: Vec2,
    },
    /// Moving one end of one edge of the selected seam along its outline.
    SeamEnd {
        end: SeamEnd,
        /// The edge as it is being dragged.
        edge: SeamEdge,
    },
}

/// One end of one edge of a seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeamEnd {
    line: SewingLineId,
    side: Side,
    index: usize,
    /// The end sewing starts from, or the one it finishes at.
    first: bool,
}

/// Same stretch of the same outline, whichever way it is sewn.
fn same_stretch(a: &SeamEdge, b: &SeamEdge) -> bool {
    a.pattern == b.pattern && a.start == b.start && a.end == b.end
}

impl Default for Canvas2D {
    fn default() -> Self {
        Self {
            pan: Px::ZERO,
            zoom: 0.6,
            drag: None,
            rect: Rect::NOTHING,
            snap: Snap::default(),
            grid_mm: DEFAULT_GRID_MM,
            draft: Vec::new(),
            draft_tool: Tool::Select,
            draft_owner: None,
            cursor: None,
            selected_point: None,
            sew_pending: None,
            sew_drag_start: None,
            hover: None,
            frame_selection: false,
            reveal: None,
            dart_width: 30.0,
            dart_depth: 80.0,
            pleat: PleatSettings::default(),
            measure: Default::default(),
            details: Default::default(),
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

    /// The two sides being built by the sew tool, if a seam (rather than a
    /// tack) is in progress.
    #[cfg(test)]
    pub fn sew_pending_edges(&self) -> Option<[Vec<SeamEdge>; 2]> {
        match &self.sew_pending {
            Some(SewBuild::Edges { sides, .. }) => Some(sides.clone()),
            _ => None,
        }
    }

    /// The point picked by the sew tool, if a tack is half-built.
    #[cfg(test)]
    pub fn sew_pending_point(&self) -> Option<TackPoint> {
        match self.sew_pending {
            Some(SewBuild::Point(p)) => Some(p),
            _ => None,
        }
    }

    /// The corner-to-corner segment under `p` as an edge, sewn from
    /// whichever end is nearer, so picking two edges near matching ends
    /// sews them untwisted.
    pub(crate) fn segment_edge(
        &self,
        doc: &Document,
        selection: Option<PatternId>,
        p: Vec2,
    ) -> Option<SeamEdge> {
        let hit = self.edge_at(doc, selection, p)?;
        let pattern = doc.pattern(hit.id)?;
        let n = pattern.outline.len() as f32;
        let (start, end) = pattern.segment_at(hit.param);
        let from_start = (hit.param - start).rem_euclid(n);
        let to_end = (end - hit.param).rem_euclid(n);
        Some(SeamEdge {
            pattern: hit.id,
            start,
            end,
            forward: from_start <= to_end,
        })
    }

    /// Snap an outline parameter onto a corner point within reach.
    pub(crate) fn snap_param(&self, pattern: &Pattern, t: f32) -> f32 {
        if !self.snap.points {
            return t;
        }
        let at = pattern.pos_at_param(t, TOLERANCE);
        let reach = self.px(SNAP_PX);
        pattern
            .outline
            .iter()
            .enumerate()
            .filter(|(_, op)| op.kind == PointKind::Corner)
            .map(|(i, op)| (op.pos.distance(at), i))
            .filter(|(d, _)| *d <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map_or(t, |(_, i)| i as f32)
    }

    /// The seam whose sewn line passes nearest `p`, within reach.
    pub fn seam_at(&self, doc: &Document, p: Vec2) -> Option<SewingLineId> {
        let reach = self.px(PICK_PX);
        doc.sewing_lines
            .iter()
            .rev()
            .filter_map(|line| {
                let gap = line
                    .edges()
                    .filter_map(|e| doc.pattern(e.pattern).map(|pat| pat.edge_points(e)))
                    .flat_map(|pts| {
                        pts.windows(2)
                            .map(|w| geometry::closest_on_segment(w[0], w[1], p).0.distance(p))
                            .collect::<Vec<_>>()
                    })
                    .fold(f32::MAX, f32::min);
                (gap <= reach).then_some((gap, line.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// The end of an edge of seam `id` within reach of `p`.
    fn seam_end_at(&self, doc: &Document, id: Option<SewingLineId>, p: Vec2) -> Option<SeamEnd> {
        let line = doc.sewing_line(id?)?;
        let reach = self.px(PICK_PX);
        let mut best: Option<(f32, SeamEnd)> = None;
        for side in [Side::A, Side::B] {
            for (index, edge) in line.side(side).iter().enumerate() {
                let Some(pattern) = doc.pattern(edge.pattern) else {
                    continue;
                };
                for (first, t) in [(true, edge.first()), (false, edge.last())] {
                    let d = pattern.pos_at_param(t, TOLERANCE).distance(p);
                    if d <= reach && best.is_none_or(|(b, _)| d < b) {
                        let end = SeamEnd {
                            line: line.id,
                            side,
                            index,
                            first,
                        };
                        best = Some((d, end));
                    }
                }
            }
        }
        best.map(|(_, end)| end)
    }

    /// The tack whose ends or thread pass nearest `p`, within reach.
    pub fn tack_at(&self, doc: &Document, p: Vec2) -> Option<TackId> {
        let reach = self.px(PICK_PX);
        doc.tacks
            .iter()
            .rev()
            .map(|tack| {
                let gap = match tack.b {
                    TackTarget::Pattern(b) => geometry::closest_on_segment(tack.a.at, b.at, p)
                        .0
                        .distance(p),
                    TackTarget::Avatar(_) | TackTarget::AvatarSurface { .. } => {
                        tack.a.at.distance(p)
                    }
                };
                (gap, tack.id)
            })
            .filter(|(gap, _)| *gap <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// The pattern point a tack click at `p` lands on: inside a pattern, or
    /// on its outline.
    fn tack_point_at(&self, doc: &Document, p: Vec2) -> Option<TackPoint> {
        if let Some(pattern) = doc.pattern_at(p) {
            return Some(TackPoint { pattern, at: p });
        }
        self.edge_at(doc, None, p).map(|hit| TackPoint {
            pattern: hit.id,
            at: hit.pos,
        })
    }

    pub(crate) fn to_screen(&self, rect: Rect, p: Vec2) -> Pos2 {
        rect.center() + self.pan + Px::new(p.x, p.y) * self.zoom
    }

    pub(crate) fn to_doc(&self, rect: Rect, p: Pos2) -> Vec2 {
        let v = (p - rect.center() - self.pan) / self.zoom;
        Vec2::new(v.x, v.y)
    }

    /// Screen pixels as millimetres at the current zoom.
    pub(crate) fn px(&self, px: f32) -> f32 {
        px / self.zoom
    }

    /// Spacing of the finest grid lines drawn, which is also the snap step.
    pub fn grid_step(&self) -> f32 {
        let mut step_mm = self.grid_mm.max(0.1);
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
            let outline = doc.patterns.iter().filter(|p| !p.hidden).flat_map(|pat| {
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
            .filter(|p| !p.hidden)
            .filter_map(nearest_in)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, hit)| hit)
    }

    /// The pattern whose outline passes nearest `p` (within reach),
    /// preferring the selected one, with its flattened outline.
    pub(crate) fn edge_at(
        &self,
        doc: &Document,
        selection: Option<PatternId>,
        p: Vec2,
    ) -> Option<EdgeHit> {
        let reach = self.px(PICK_PX);
        let order = selection.into_iter().chain(
            doc.patterns
                .iter()
                .rev()
                .filter(|p| !p.hidden)
                .map(|pat| pat.id),
        );
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

pub(crate) struct EdgeHit {
    pub id: PatternId,
    gap: f32,
    pub pos: Vec2,
    /// Outline parameter (see [`od_core::document::FlatPoint`]).
    pub param: f32,
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
        s.tack_start = None;
        s.canvas.draft_tool = s.tool;
    }
    if s.tool != Tool::EditPoints {
        s.canvas.selected_point = None;
    }
    s.canvas.hover = response.hover_pos().map(|p| s.canvas.to_doc(rect, p));
    if let Some(p) = s.canvas.hover {
        s.hover_next = s.doc.pattern_at(p);
    }
    frame_and_reveal(s, rect);
    if response.secondary_clicked() {
        let hit = response
            .interact_pointer_pos()
            .and_then(|p| s.doc.pattern_at(s.canvas.to_doc(rect, p)));
        crate::panels::context::select_for_menu(s, hit);
    }
    response.context_menu(|ui| crate::panels::context::menu(ui, s));
    s.canvas.cursor = s.canvas.hover.map(|p| s.canvas.snap(&s.doc, p, None));

    // Single keys must not fire while typing in a text field.
    let typing = ui.memory(|m| m.focused().is_some());
    match s.tool {
        Tool::Select | Tool::Rectangle => handle_drag_tools(&response, rect, s),
        Tool::Pen | Tool::InternalLine | Tool::Topstitch => {
            handle_draft(ui, &response, rect, s, typing);
        }
        Tool::EditPoints => handle_edit_points(&response, rect, s),
        Tool::Notch => handle_notch(&response, rect, s),
        Tool::Sew => handle_sew(&response, rect, s),
        Tool::Trim => handle_trim(ui, &response, rect, s),
        Tool::Tack => handle_tack(ui, &response, rect, s, typing),
        Tool::Dart => handle_dart(&response, rect, s),
        Tool::Pleat => handle_pleat(&response, rect, s),
        Tool::Measure => crate::panels::measure2d::handle(ui, &response, rect, s, typing),
        Tool::Zipper => crate::panels::details::handle_zipper(ui, &response, rect, s),
        Tool::Buttonhole => crate::panels::details::handle_buttonhole(ui, &response, rect, s),
        Tool::Binding => crate::panels::details::handle_binding(&response, rect, s),
    }

    // Background and grid.
    painter.rect_filled(rect, 0.0, pal.canvas_bg);
    draw_grid(&painter, rect, &s.canvas, pal);
    crate::panels::measure2d::draw_sizes(&painter, rect, s, pal);

    // Previewing a graded size only changes what's drawn: editing always
    // acts on the base pattern, so every tool keeps working off real
    // outline indices regardless of which size is being looked at.
    let steps = s.preview_steps();
    let graded: Vec<Pattern> = if steps == 0 {
        Vec::new()
    } else {
        s.doc.patterns.iter().map(|p| p.graded(steps)).collect()
    };
    let display: Vec<&Pattern> = if steps == 0 {
        s.doc.patterns.iter().collect()
    } else {
        graded.iter().collect()
    };
    for pattern in display.into_iter().filter(|p| !p.hidden) {
        draw_pattern(&painter, rect, s, pattern, pal);
    }
    draw_sewing_lines(&painter, rect, s, pal);
    crate::panels::details::draw(&painter, rect, s, pal);
    draw_overlays(&painter, rect, s, pal);
    crate::panels::measure2d::draw(&painter, rect, s, pal);

    let hint = match s.tool {
        Tool::Select => {
            "Click to select a pattern, seam or tack · drag to move · drag a selected seam's ends to edit it"
        }
        Tool::Tack => {
            "Click a point on a pattern, then a point on a pattern to hold it to · in the 3D view the second click can be on the avatar · Esc cancels"
        }
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
        Tool::Topstitch => {
            "Click inside a pattern to draw a topstitch line · Enter or double-click finishes · Esc cancels"
        }
        Tool::Notch => "Click an outline to add a notch · click a notch to remove it",
        Tool::Dart => {
            "Click an outline to cut a dart into it · drag inside a pattern, tip to tip, for a diamond dart · sizes in Properties"
        }
        Tool::Pleat => {
            "Drag along where the first pleat folds · the rest step to the right of the drag · kind and depth in Properties"
        }
        Tool::Trim => {
            "Click to place a button (Shift+click: snap) · click a trim to remove it"
        }
        Tool::Measure => {
            "Click two outline points to measure between them (Shift: along the outline) · Esc cancels"
        }
        Tool::Zipper | Tool::Buttonhole | Tool::Binding => crate::panels::details::hint(s),
        Tool::Sew => match &s.canvas.sew_pending {
            Some(SewBuild::Edges { active, .. }) => {
                if *active == 0 {
                    "Click/drag edges for this side · Tab: other side · Enter sews them · Esc cancels"
                } else {
                    "Click/drag edges for the other side · Tab: first side · Enter sews them · Esc cancels"
                }
            }
            Some(SewBuild::Point(_)) => "Click the matching point to finish the tack · Esc cancels",
            None => {
                "Click/drag an edge to sew (Tab/Enter for one-to-many), or an exact point to tack · click a sewn edge or tack to remove it"
            }
        },
    };
    let size_note = if steps != 0 {
        format!(
            " · previewing size {} (editing still acts on the base pattern)",
            s.doc.size_chart.names[s.preview_size]
        )
    } else {
        String::new()
    };
    painter.text(
        rect.left_top() + Px::new(10.0, 8.0),
        Align2::LEFT_TOP,
        format!(
            "{hint} · scroll to zoom · right-drag to pan · {:.0}%{size_note}",
            s.canvas.zoom * 100.0
        ),
        FontId::proportional(12.0),
        pal.hint,
    );
}

/// Radius of the rotation handle, in screen pixels.
const HANDLE_PX: f32 = 6.0;
/// How far the rotation handle sits above the main selected pattern.
const HANDLE_GAP_PX: f32 = 28.0;

/// With the Select tool and something unlocked selected: the main
/// pattern's centre and, above its top edge, the handle that turns the
/// selection. In document space.
fn rotate_handle(s: &Session) -> Option<(Vec2, Vec2)> {
    if s.tool != Tool::Select || s.unlocked_selection().is_empty() {
        return None;
    }
    let id = s.selection.primary()?;
    let (min, max) = s.doc.pattern(id)?.bounds()?;
    let centre = (min + max) * 0.5;
    Some((
        centre,
        Vec2::new(centre.x, min.y - s.canvas.px(HANDLE_GAP_PX)),
    ))
}

#[cfg(test)]
pub fn rotate_handle_for_tests(s: &Session) -> Option<(Vec2, Vec2)> {
    rotate_handle(s)
}

/// Bounds of `ids`' outlines together, in mm.
fn bounds_of(doc: &Document, ids: &[PatternId]) -> Option<(Vec2, Vec2)> {
    let points: Vec<Vec2> = ids
        .iter()
        .filter_map(|&id| doc.pattern(id)?.bounds())
        .flat_map(|(min, max)| [min, max])
        .collect();
    geometry::bounds(&points)
}

/// Act on [`Canvas2D::frame_selection`] and [`Canvas2D::reveal`].
fn frame_and_reveal(s: &mut Session, rect: Rect) {
    if std::mem::take(&mut s.canvas.frame_selection) {
        if let Some((min, max)) = bounds_of(&s.doc, s.selection.ids()) {
            // A margin of a tenth on each side.
            let size = (max - min) * 1.2;
            let zoom = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
            s.canvas.zoom = zoom.clamp(0.05, 20.0);
            let centre = (min + max) * 0.5;
            s.canvas.pan = -Px::new(centre.x, centre.y) * s.canvas.zoom;
        }
    }
    if let Some(id) = s.canvas.reveal.take() {
        if let Some((min, max)) = bounds_of(&s.doc, &[id]) {
            let seen =
                Rect::from_two_pos(s.canvas.to_screen(rect, min), s.canvas.to_screen(rect, max));
            if !rect.contains_rect(seen) {
                let centre = (min + max) * 0.5;
                s.canvas.pan = -Px::new(centre.x, centre.y) * s.canvas.zoom;
            }
        }
    }
}

/// Whether Shift or Ctrl (Cmd on macOS) is held, to add to the selection.
fn adding(response: &egui::Response) -> bool {
    response
        .ctx
        .input(|i| i.modifiers.shift || i.modifiers.command)
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
        // Tacks and seams lie on patterns, so they win over the pattern
        // underneath.
        s.selected_tack = pointer.and_then(|p| s.canvas.tack_at(&s.doc, p));
        s.selected_seam = match s.selected_tack {
            Some(_) => None,
            None => pointer.and_then(|p| s.canvas.seam_at(&s.doc, p)),
        };
        let hit = match (s.selected_tack, s.selected_seam) {
            (None, None) => pointer.and_then(|p| s.doc.pattern_at(p)),
            _ => None,
        };
        // Shift or Ctrl adds or removes a pattern; a click on nothing with
        // either held keeps the selection.
        match (hit, adding(response)) {
            (Some(id), true) => s.selection.toggle(id),
            (None, true) => {}
            (hit, false) => s.selection.set(hit),
        }
    }

    // egui reports a drag only after the pointer has moved a few pixels, so
    // anchor it at the press point and count the movement already made.
    if response.drag_started_by(PointerButton::Primary) {
        if let (Some(origin), Some(p)) = (press_origin(response, rect, &s.canvas), pointer) {
            s.canvas.drag = match s.tool {
                Tool::Select
                    if rotate_handle(s).is_some_and(|(_, h)| {
                        h.distance(origin) <= s.canvas.px(HANDLE_PX + 4.0)
                    }) =>
                {
                    let (pivot, _) = rotate_handle(s).unwrap_or_default();
                    Some(Drag::Rotate {
                        ids: s.unlocked_selection(),
                        pivot,
                        from: origin,
                        degrees: 0.0,
                    })
                }
                Tool::Select => {
                    // The selected seam's ends come first: they sit on
                    // outlines, over the pattern.
                    let end = s.canvas.seam_end_at(&s.doc, s.selected_seam, origin);
                    let edge = end.and_then(|end| {
                        let line = s.doc.sewing_line(end.line)?;
                        line.side(end.side).get(end.index).copied()
                    });
                    match (end, edge) {
                        (Some(end), Some(edge)) => Some(Drag::SeamEnd { end, edge }),
                        _ => match s.doc.pattern_at(origin) {
                            // Dragging a selected pattern moves the whole
                            // selection; dragging another selects it first.
                            Some(id) => {
                                if adding(response) {
                                    s.selection.add(id);
                                } else if !s.selection.contains(id) {
                                    s.selection.set(Some(id));
                                }
                                s.selected_seam = None;
                                s.selected_tack = None;
                                let ids: Vec<_> = s
                                    .selection
                                    .ids()
                                    .iter()
                                    .copied()
                                    .filter(|&id| !locked(&s.doc, id))
                                    .collect();
                                (!ids.is_empty()).then(|| Drag::Move {
                                    ids,
                                    delta: p - origin,
                                })
                            }
                            None => Some(Drag::Select {
                                start: origin,
                                end: p,
                                add: adding(response),
                            }),
                        },
                    }
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
        if let (Some(p), Some(Drag::SeamEnd { end, edge })) = (pointer, &s.canvas.drag) {
            if let Some(pattern) = s.doc.pattern(edge.pattern) {
                let t = s.canvas.snap_param(pattern, pattern.nearest_param(p).0);
                let moved = if end.first {
                    pattern.edge_between(t, edge.last(), edge.forward)
                } else {
                    pattern.edge_between(edge.first(), t, edge.forward)
                };
                let end = *end;
                s.canvas.drag = Some(Drag::SeamEnd { end, edge: moved });
            }
        }
        let coarse = response.ctx.input(|i| i.modifiers.shift);
        match &mut s.canvas.drag {
            Some(Drag::Move { delta, .. }) => *delta += Vec2::new(step.x, step.y),
            Some(Drag::Rotate {
                pivot,
                from,
                degrees,
                ..
            }) => {
                if let Some(p) = pointer {
                    let (a, b) = (*from - *pivot, p - *pivot);
                    let swept = b.y.atan2(b.x) - a.y.atan2(a.x);
                    // Whole degrees, or 15° steps with Shift held.
                    let step = if coarse { 15.0 } else { 1.0 };
                    *degrees = normalize_degrees((swept.to_degrees() / step).round() * step);
                }
            }
            Some(Drag::Select { end, .. }) => {
                if let Some(p) = pointer {
                    *end = p;
                }
            }
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
            Some(Drag::Move { ids, delta }) if delta != Vec2::ZERO => {
                s.for_each(ids, "Move", "patterns", |id| {
                    Box::new(MovePattern::new(id, delta))
                });
            }
            Some(Drag::Rotate { ids, degrees, .. }) if degrees != 0.0 => {
                s.for_each(ids, "Rotate", "patterns", |id| {
                    Box::new(RotatePattern::new(id, degrees))
                });
            }
            Some(Drag::Select { start, end, add }) => {
                let lo = Vec2::new(start.x.min(end.x), start.y.min(end.y));
                let hi = Vec2::new(start.x.max(end.x), start.y.max(end.y));
                let inside = s.doc.patterns.iter().filter(|p| !p.hidden).filter_map(|p| {
                    let (min, max) = p.bounds()?;
                    let touches = min.x <= hi.x && max.x >= lo.x && min.y <= hi.y && max.y >= lo.y;
                    touches.then_some(p.id)
                });
                let ids: Vec<_> = inside.collect();
                if !add {
                    s.selection.clear();
                }
                for id in ids {
                    s.selection.add(id);
                }
                s.selected_seam = None;
                s.selected_tack = None;
            }
            Some(Drag::SeamEnd { end, edge }) => {
                let current = s
                    .doc
                    .sewing_line(end.line)
                    .and_then(|line| line.side(end.side).get(end.index).copied());
                // An end dragged onto the other end would leave nothing sewn.
                let empty = edge.end - edge.start < 1e-4;
                if current.is_some_and(|c| c != edge) && !empty {
                    s.execute(SetSeamEdge::new(end.line, end.side, end.index, edge));
                }
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
    let open_line = s.tool == Tool::InternalLine || s.tool == Tool::Topstitch;
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
            if open_line && s.canvas.draft.is_empty() {
                // A line belongs to the pattern it starts in (or on).
                s.canvas.draft_owner = s.doc.pattern_at(raw).or_else(|| {
                    s.canvas
                        .edge_at(&s.doc, s.selection.primary(), raw)
                        .map(|h| h.id)
                });
                if s.canvas.draft_owner.is_none() {
                    return;
                }
                s.selection.set(s.canvas.draft_owner);
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
                let (id, link) = s.edit_target(id);
                s.execute(AddInternalLine::new(
                    id,
                    draft.iter().map(|p| to_source(link, p.pos)).collect(),
                ));
            }
        }
        Tool::Topstitch if draft.len() >= 2 => {
            if let Some(id) = s.canvas.draft_owner.take() {
                let (id, link) = s.edit_target(id);
                let points = draft.iter().map(|p| to_source(link, p.pos)).collect();
                s.execute(AddTopstitch::new(id, points));
            }
        }
        _ => {}
    }
}

/// The outline point under `p` on a pattern that isn't locked.
fn editable_point(s: &Session, p: Vec2) -> Option<(PatternId, usize)> {
    s.canvas
        .point_at(&s.doc, s.selection.primary(), p)
        .filter(|(id, _)| !locked(&s.doc, *id))
}

fn locked(doc: &Document, id: PatternId) -> bool {
    doc.pattern(id).is_some_and(|p| p.locked)
}

fn handle_edit_points(response: &egui::Response, rect: Rect, s: &mut Session) {
    let pointer = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p));

    if response.clicked() {
        if let Some(p) = pointer {
            match editable_point(s, p) {
                Some((id, index)) => {
                    s.selection.set(Some(id));
                    s.canvas.selected_point = Some((id, index));
                }
                None => {
                    s.selection.set(s.doc.pattern_at(p));
                    s.canvas.selected_point = None;
                }
            }
        }
    }
    if response.double_clicked() {
        if let Some(p) = pointer {
            if let Some((id, index)) = editable_point(s, p) {
                let kind = s
                    .doc
                    .pattern(id)
                    .and_then(|pat| pat.outline.get(index))
                    .map(|p| p.kind);
                if let Some(kind) = kind {
                    let (target, _) = s.edit_target(id);
                    s.execute(SetPointKind::new(target, index, kind.toggled()));
                }
            } else if let Some(hit) = s
                .canvas
                .edge_at(&s.doc, s.selection.primary(), p)
                .filter(|h| !locked(&s.doc, h.id))
            {
                let count = s.doc.pattern(hit.id).map_or(0, |pat| pat.outline.len());
                let index = (hit.param.floor() as usize % count.max(1)) + 1;
                let (target, link) = s.edit_target(hit.id);
                s.execute(InsertPoint::new(
                    target,
                    index,
                    OutlinePoint::corner(to_source(link, hit.pos)),
                ));
                s.selection.set(Some(hit.id));
                s.canvas.selected_point = Some((hit.id, index));
            }
        }
    }

    if response.drag_started_by(PointerButton::Primary) {
        if let Some(origin) = press_origin(response, rect, &s.canvas) {
            if let Some((id, index)) = editable_point(s, origin) {
                let pos = s
                    .doc
                    .pattern(id)
                    .map_or(origin, |pat| pat.outline[index].pos);
                s.selection.set(Some(id));
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
                let (target, link) = s.edit_target(id);
                s.execute(MovePoint::new(target, index, to_source(link, to)));
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
    let Some(hit) = s.canvas.edge_at(&s.doc, s.selection.primary(), p) else {
        return;
    };
    let Some(pattern) = s.doc.pattern(hit.id) else {
        return;
    };
    s.selection.set(Some(hit.id));
    let poly = pattern.polyline(TOLERANCE);
    let reach = s.canvas.px(PICK_PX);
    let existing = pattern.notches.iter().position(|n| {
        geometry::point_at_distance(&poly, n.at).is_some_and(|(q, _)| q.distance(p) <= reach)
    });
    // A linked copy's notches are its source's, at the same distances.
    let (target, _) = s.edit_target(hit.id);
    match existing {
        Some(index) => s.execute(RemoveNotch::new(target, index)),
        None => {
            if let Some((at, _)) = geometry::distance_along(&poly, hit.pos) {
                s.execute(AddNotch::new(target, at));
            }
        }
    }
}

/// The Dart tool: a click on an outline cuts a V-shaped dart into it (its
/// legs sewn together); a drag inside a pattern, from tip to tip, adds a
/// diamond dart there. Sizes come from the canvas's dart settings.
fn handle_dart(response: &egui::Response, rect: Rect, s: &mut Session) {
    let pointer = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p));
    let unlocked = |s: &Session, id: PatternId| !locked(&s.doc, id);

    if response.clicked() {
        let Some(p) = pointer else { return };
        let Some(hit) = s.canvas.edge_at(&s.doc, s.selection.primary(), p) else {
            return;
        };
        s.selection.set(Some(hit.id));
        let (width, depth) = (s.canvas.dart_width, s.canvas.dart_depth);
        let (target, link) = s.edit_target(hit.id);
        let at = to_source(link, hit.pos);
        let batch = s
            .doc
            .pattern(target)
            .filter(|_| unlocked(s, hit.id))
            .and_then(|pattern| edit::add_outline_dart(pattern, at, width, depth));
        if let Some(batch) = batch {
            s.execute(batch);
        }
        return;
    }
    line_drag(response, rect, s);
    if response.drag_stopped() {
        if let Some(Drag::Line { owner, start, end }) = s.canvas.drag.take() {
            let inside = s
                .doc
                .pattern(owner)
                .is_some_and(|p| p.contains(start) && p.contains(end));
            if inside && start.distance(end) >= MIN_RECT_MM {
                let width = s.canvas.dart_width;
                let (owner, link) = s.edit_target(owner);
                let (start, end) = (to_source(link, start), to_source(link, end));
                s.execute(AddDart::new(owner, Dart { start, end, width }));
            }
        }
    }
}

/// The Pleat tool: a drag inside a pattern marks the first fold; the
/// pleat's fold lines are laid across the whole piece from there.
fn handle_pleat(response: &egui::Response, rect: Rect, s: &mut Session) {
    line_drag(response, rect, s);
    if !response.drag_stopped() {
        return;
    }
    let Some(Drag::Line { owner, start, end }) = s.canvas.drag.take() else {
        return;
    };
    if start.distance(end) < MIN_RECT_MM {
        return;
    }
    let (owner, link) = s.edit_target(owner);
    let (start, end) = (to_source(link, start), to_source(link, end));
    let Some(pattern) = s.doc.pattern(owner) else {
        return;
    };
    let p = s.canvas.pleat;
    let lines = edit::pleat_lines(pattern, start, end, p.kind, p.count, p.depth, p.angle);
    let tacks = if p.tack {
        edit::pleat_tacks(pattern, start, end, p.kind, p.count, p.depth)
    } else {
        Vec::new()
    };
    let point = |at| TackPoint { pattern: owner, at };
    let mut commands: Vec<Box<dyn od_core::Command>> = lines
        .into_iter()
        .map(|line| -> Box<dyn od_core::Command> {
            match line.fold {
                Some(fold) => Box::new(AddInternalLine::folded(owner, line.points, fold)),
                None => Box::new(AddInternalLine::new(owner, line.points)),
            }
        })
        .collect();
    for [a, b] in tacks {
        commands.push(Box::new(AddTack::new(
            point(a),
            TackTarget::Pattern(point(b)),
        )));
    }
    if !commands.is_empty() {
        s.execute(od_core::Batch::new("Add pleats", commands));
    }
}

/// A position on a linked copy mapped back to its source (see
/// [`Session::edit_target`]), or `p` itself without a link.
fn to_source(link: Option<od_core::Link>, p: Vec2) -> Vec2 {
    link.map_or(p, |l| l.to_source(p))
}

/// Track a [`Drag::Line`] inside an unlocked pattern: started on the
/// pattern under the press, ended wherever the (snapped) pointer is.
fn line_drag(response: &egui::Response, rect: Rect, s: &mut Session) {
    let pointer = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p));
    if response.drag_started_by(PointerButton::Primary) {
        let origin = press_origin(response, rect, &s.canvas);
        let owner = origin
            .and_then(|o| s.doc.pattern_at(o))
            .filter(|&id| !locked(&s.doc, id));
        if let (Some(owner), Some(origin), Some(p)) = (owner, origin, pointer) {
            s.selection.set(Some(owner));
            s.canvas.drag = Some(Drag::Line {
                owner,
                start: s.canvas.snap(&s.doc, origin, None).0,
                end: s.canvas.snap(&s.doc, p, None).0,
            });
        }
    } else if response.dragged_by(PointerButton::Primary) {
        let snapped = pointer.map(|p| s.canvas.snap(&s.doc, p, None).0);
        if let (Some(p), Some(Drag::Line { end, .. })) = (snapped, &mut s.canvas.drag) {
            *end = p;
        }
    }
}

/// Place or remove a point trim (button or snap). A click on or inside a
/// pattern with no trim nearby adds one (a button, or Shift+click for a
/// snap); a click near an existing trim removes it instead.
fn handle_trim(ui: &egui::Ui, response: &egui::Response, rect: Rect, s: &mut Session) {
    if !response.clicked() {
        return;
    }
    let Some(screen) = response.interact_pointer_pos() else {
        return;
    };
    let p = s.canvas.to_doc(rect, screen);
    let Some(id) = s.doc.pattern_at(p).or_else(|| {
        s.canvas
            .edge_at(&s.doc, s.selection.primary(), p)
            .map(|h| h.id)
    }) else {
        return;
    };
    s.selection.set(Some(id));
    let Some(pattern) = s.doc.pattern(id) else {
        return;
    };
    let reach = s.canvas.px(PICK_PX);
    let existing = pattern
        .trims
        .iter()
        .position(|t| t.pos.distance(p) <= reach);
    let (id, link) = s.edit_target(id);
    let p = to_source(link, p);
    match existing {
        Some(index) => s.execute(RemoveTrim::new(id, index)),
        None => {
            let kind = if ui.input(|i| i.modifiers.shift) {
                TrimKind::Snap
            } else {
                TrimKind::Button
            };
            s.execute(AddTrim::new(id, kind, p));
        }
    }
}

/// Build up a seam or a tack. A plain click on an edge picks its whole
/// corner-to-corner span (see [`Pattern::segment_at`]); dragging along an
/// edge instead picks an arbitrary sub-range, for free sewing. Each pick
/// joins (or, if already there, leaves) the side of the seam that is
/// currently being built; Tab switches sides, Enter sews them together —
/// a side with more than one edge makes a one-to-many seam. Esc cancels.
///
/// Clicking an exact outline point, rather than along an edge, picks it
/// for a tack instead: a point-to-point stitch that commits as soon as a
/// second point is picked, since a tack is always one-to-one.
///
/// Clicking an edge or point that is already sewn or tacked, with no pick
/// in progress, removes it.
fn handle_sew(response: &egui::Response, rect: Rect, s: &mut Session) {
    let ctx = response.ctx.clone();
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        s.canvas.sew_pending = None;
    }
    if ctx.input(|i| i.key_pressed(Key::Tab)) {
        if let Some(SewBuild::Edges { active, .. }) = &mut s.canvas.sew_pending {
            *active = 1 - *active;
        }
    }
    if ctx.input(|i| i.key_pressed(Key::Enter)) {
        if let Some(SewBuild::Edges { sides, .. }) = &s.canvas.sew_pending {
            if !sides[0].is_empty() && !sides[1].is_empty() {
                let [a, b] = sides.clone();
                s.execute(AddSewingLine::new(a, b));
                s.canvas.sew_pending = None;
            }
        }
    }

    // The pointer's press position is only reported by egui while the
    // drag is ongoing (it reads back as `None` once released), so keep a
    // copy of the last-known one for when the drag stops.
    if response.dragged() || response.drag_started() {
        if let Some(origin) = press_origin(response, rect, &s.canvas) {
            s.canvas.sew_drag_start = Some(origin);
        }
    }
    if !response.clicked() && !response.drag_stopped() {
        return;
    }
    let Some(release) = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p))
    else {
        return;
    };

    // An exact point takes priority over an edge, but only while no
    // multi-edge seam is already being built (that takes over point
    // clicks near a corner, so edges there can still be picked).
    if !matches!(s.canvas.sew_pending, Some(SewBuild::Edges { .. })) {
        if let Some((id, index)) = s.canvas.point_at(&s.doc, s.selection.primary(), release) {
            if let Some(at) = s.doc.pattern(id).map(|p| p.outline[index].pos) {
                handle_sew_point(s, TackPoint { pattern: id, at });
            }
            return;
        }
    }

    let press = s.canvas.sew_drag_start.take();
    let dragged = press.is_some_and(|p| p.distance(release) > s.canvas.px(PICK_PX));
    let edge = if dragged {
        let (a, b) = (
            s.canvas
                .edge_at(&s.doc, s.selection.primary(), press.unwrap()),
            s.canvas.edge_at(&s.doc, s.selection.primary(), release),
        );
        match (a, b) {
            // From where the drag began to where it ended, the shorter way
            // round: it may cross the outline's starting point.
            (Some(a), Some(b)) if a.id == b.id => s
                .doc
                .pattern(a.id)
                .map(|pattern| pattern.shorter_edge(a.param, b.param)),
            _ => None,
        }
    } else {
        s.canvas
            .segment_edge(&s.doc, s.selection.primary(), release)
    };
    if let Some(edge) = edge {
        handle_sew_edge(s, edge);
    }
}

/// Handle one edge pick: unsew an existing seam that uses it, remove it
/// from (or add it to) the side currently being built, or start a new
/// build with it.
fn handle_sew_edge(s: &mut Session, edge: SeamEdge) {
    s.selection.set(Some(edge.pattern));
    if s.canvas.sew_pending.is_none() {
        let sewn = s
            .doc
            .sewing_lines
            .iter()
            .find(|sl| sl.edges().any(|e| same_stretch(e, &edge)));
        if let Some(sewn) = sewn {
            s.execute(RemoveSewingLine::new(sewn.id));
            return;
        }
    }
    let Some(SewBuild::Edges { sides, active }) = &mut s.canvas.sew_pending else {
        // Nothing pending, or a tack pick was: start a fresh seam build.
        s.canvas.sew_pending = Some(SewBuild::Edges {
            sides: [vec![edge], Vec::new()],
            active: 0,
        });
        return;
    };
    let side = &mut sides[*active];
    match side.iter().position(|e| same_stretch(e, &edge)) {
        Some(i) => {
            side.remove(i);
        }
        None => side.push(edge),
    }
    if sides[0].is_empty() && sides[1].is_empty() {
        s.canvas.sew_pending = None;
    }
}

/// Handle one point pick: untack an existing tack that uses it, complete a
/// pending point into a tack, or start a new pending point.
fn handle_sew_point(s: &mut Session, point: TackPoint) {
    s.selection.set(Some(point.pattern));
    if s.canvas.sew_pending.is_none() {
        let tacked = s.doc.tacks.iter().find(|t| t.points().any(|p| *p == point));
        if let Some(tacked) = tacked {
            s.execute(RemoveTack::new(tacked.id));
            return;
        }
    }
    match s.canvas.sew_pending.take() {
        Some(SewBuild::Point(first)) if first == point => {} // same point again: cancel
        Some(SewBuild::Point(first)) => {
            s.execute(AddTack::new(first, TackTarget::Pattern(point)));
        }
        // Nothing pending, or an edge build was: start a fresh tack pick.
        _ => s.canvas.sew_pending = Some(SewBuild::Point(point)),
    }
}

fn handle_tack(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    s: &mut Session,
    typing: bool,
) {
    if response.clicked() {
        let pointer = response
            .interact_pointer_pos()
            .map(|p| s.canvas.to_doc(rect, p));
        if let Some(point) = pointer.and_then(|p| s.canvas.tack_point_at(&s.doc, p)) {
            s.tack_to(TackTarget::Pattern(point));
        }
    }
    if !typing && ui.input(|i| i.key_pressed(Key::Escape)) {
        s.tack_start = None;
    }
}

/// The seam as it should look this frame, with a dragged end.
fn previewed_seam<'a>(s: &Session, line: &'a SewingLine) -> std::borrow::Cow<'a, SewingLine> {
    match &s.canvas.drag {
        Some(Drag::SeamEnd { end, edge }) if end.line == line.id => {
            let mut line = line.clone();
            if let Some(slot) = line.side_mut(end.side).get_mut(end.index) {
                *slot = *edge;
            }
            std::borrow::Cow::Owned(line)
        }
        _ => std::borrow::Cow::Borrowed(line),
    }
}

/// Colour seam `index` is drawn in: its own, or the warning colour if the
/// length check flags it.
pub fn seam_color(s: &Session, index: usize, pal: &theme::Palette) -> Color32 {
    if s.seam_check.flags(&s.doc, &s.doc.sewing_lines[index]) {
        pal.warning
    } else {
        pal.seams[index % pal.seams.len()]
    }
}

/// The pattern as it should look this frame, with any drag in progress.
fn previewed<'a>(s: &Session, pattern: &'a Pattern) -> std::borrow::Cow<'a, Pattern> {
    match &s.canvas.drag {
        Some(Drag::Rotate { ids, degrees, .. }) if ids.contains(&pattern.id) => {
            let mut p = pattern.clone();
            p.rotate(*degrees, p.center());
            std::borrow::Cow::Owned(p)
        }
        Some(Drag::Move { ids, delta }) if ids.contains(&pattern.id) => {
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
    let selected = s.selection.contains(pattern.id);

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

    // Under the pointer here or in the 3D view: drawn emphasised.
    let hovered = s.hovered == Some(pattern.id);
    let stroke = match (selected, hovered) {
        (true, _) => Stroke::new(2.5, pal.selected),
        (false, true) => Stroke::new(2.5, pal.accent),
        (false, false) => Stroke::new(1.5, pal.pattern_stroke),
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
        let pts: Vec<Pos2> = line.points.iter().map(|p| to_screen(*p)).collect();
        match line.fold {
            None => {
                painter.add(Shape::line(pts, Stroke::new(1.5, pal.pattern_stroke)));
            }
            // Fold lines: long dashes for a valley, short for a mountain.
            Some(fold) => {
                let (dash, gap) = if fold.angle >= 0.0 {
                    (10.0, 4.0)
                } else {
                    (3.0, 3.0)
                };
                painter.extend(Shape::dashed_line(
                    &pts,
                    Stroke::new(1.5, pal.accent),
                    dash,
                    gap,
                ));
            }
        }
    }
    for dart in &pattern.darts {
        draw_dart(
            painter,
            rect,
            canvas,
            dart,
            Stroke::new(1.5, pal.pattern_stroke),
        );
    }

    // Topstitch lines: dashed, so they read as stitching rather than a
    // dart or placement mark.
    for line in &pattern.topstitches {
        let pts: Vec<Pos2> = line.points.iter().map(|p| to_screen(*p)).collect();
        painter.extend(Shape::dashed_line(
            &pts,
            Stroke::new(1.5, pal.preview),
            5.0,
            3.0,
        ));
    }

    // Point trims: a filled dot for a button, a ring for a snap.
    for trim in &pattern.trims {
        let at = to_screen(trim.pos);
        let r = canvas.px(4.0).max(3.0);
        match trim.kind {
            TrimKind::Button => {
                painter.circle_filled(at, r, pal.selected);
            }
            TrimKind::Snap => {
                painter.circle_stroke(at, r, Stroke::new(1.5, pal.selected));
            }
            TrimKind::Buttonhole => {
                crate::panels::details::draw_buttonhole(painter, trim, &to_screen, pal.selected);
            }
        }
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

/// A diamond dart: its legs, and a dashed fold line from tip to tip.
fn draw_dart(painter: &egui::Painter, rect: Rect, canvas: &Canvas2D, dart: &Dart, stroke: Stroke) {
    let to_screen = |p: Vec2| canvas.to_screen(rect, p);
    for leg in dart.legs() {
        painter.add(Shape::line(leg.map(to_screen).to_vec(), stroke));
    }
    painter.extend(Shape::dashed_line(
        &[to_screen(dart.start), to_screen(dart.end)],
        Stroke::new(1.0, stroke.color),
        4.0,
        3.0,
    ));
}

/// Seams and tacks. Each seam has its own colour (the warning colour when
/// [`crate::app::SeamCheck`] flags its length), a dot where each side starts
/// sewing, and on the selected or hovered seam, dashed lines joining the
/// points sewn together, plus handles on the selected seam's ends. Tacks are
/// crosses with a dashed thread, or labelled "avatar" when held to it.
fn draw_sewing_lines(painter: &egui::Painter, rect: Rect, s: &Session, pal: &theme::Palette) {
    let to_screen = |p: Vec2| s.canvas.to_screen(rect, p);
    let edge_points = |edge: &SeamEdge| -> Vec<Pos2> {
        s.doc
            .pattern(edge.pattern)
            .map(|pattern| previewed(s, pattern).edge_points(edge))
            .unwrap_or_default()
            .into_iter()
            .map(to_screen)
            .collect()
    };
    let hidden = |id| s.doc.pattern(id).is_some_and(|p| p.hidden);
    let draw_group = |group: &[SeamEdge], stroke: Stroke| {
        for (k, edge) in group.iter().enumerate() {
            if hidden(edge.pattern) {
                continue;
            }
            let points = edge_points(edge);
            // A dot where each side starts shows which way it is sewn.
            if let (0, Some(&first)) = (k, points.first()) {
                painter.circle_filled(first, stroke.width + 1.0, stroke.color);
            }
            if points.len() >= 2 {
                painter.add(Shape::line(points, stroke));
            }
        }
    };
    let hovered = match s.tool {
        Tool::Select => s.canvas.hover.and_then(|p| s.canvas.seam_at(&s.doc, p)),
        _ => None,
    };

    for (i, line) in s.doc.sewing_lines.iter().enumerate() {
        let line = previewed_seam(s, line);
        let line = line.as_ref();
        let color = seam_color(s, i, pal);
        let selected = s.selected_seam == Some(line.id);
        let emphasis = selected || hovered == Some(line.id);
        let stroke = Stroke::new(if emphasis { 5.0 } else { 3.0 }, color);
        draw_group(&line.a, stroke);
        draw_group(&line.b, stroke);
        if s.seam_check.flags(&s.doc, line) {
            let mismatch = line.length_mismatch(&s.doc);
            let a_points: Vec<Pos2> = line.a.iter().flat_map(edge_points).collect();
            if let (Some(mismatch), Some(&mid)) = (mismatch, a_points.get(a_points.len() / 2)) {
                painter.text(
                    mid + Px::new(8.0, -8.0),
                    Align2::LEFT_BOTTOM,
                    format!("seam mismatch: {:+.0} mm", mismatch),
                    FontId::proportional(12.0),
                    pal.warning,
                );
            }
        }
        if emphasis {
            const PAIRS: usize = 8;
            for k in 0..=PAIRS {
                let t = k as f32 / PAIRS as f32;
                if let (Some((_, a)), Some((_, b))) = (
                    s.doc.side_point(line, Side::A, t),
                    s.doc.side_point(line, Side::B, t),
                ) {
                    painter.extend(Shape::dashed_line(
                        &[to_screen(a), to_screen(b)],
                        Stroke::new(1.0, color),
                        5.0,
                        4.0,
                    ));
                }
            }
        }
        if selected {
            for edge in line.edges() {
                if let Some(pattern) = s.doc.pattern(edge.pattern) {
                    for t in [edge.start, edge.end] {
                        let at = to_screen(previewed(s, pattern).pos_at_param(t, TOLERANCE));
                        painter.circle(at, 5.0, pal.canvas_bg, Stroke::new(2.0, color));
                    }
                }
            }
        }
    }

    // Tacked points move with a pattern being dragged.
    let at = |point: &TackPoint| match &s.canvas.drag {
        Some(Drag::Move { ids, delta }) if ids.contains(&point.pattern) => point.at + *delta,
        Some(Drag::Rotate { ids, degrees, .. }) if ids.contains(&point.pattern) => s
            .doc
            .pattern(point.pattern)
            .map_or(point.at, |p| rotate_point(point.at, *degrees, p.center())),
        _ => point.at,
    };
    let mark = |at: Pos2, color: Color32| {
        let stroke = Stroke::new(2.0, color);
        for d in [Px::new(5.0, 5.0), Px::new(5.0, -5.0)] {
            painter.line_segment([at - d, at + d], stroke);
        }
    };
    let hidden = |id| s.doc.pattern(id).is_some_and(|p| p.hidden);
    for tack in s
        .doc
        .tacks
        .iter()
        .filter(|t| !t.points().any(|p| hidden(p.pattern)))
    {
        let color = if s.selected_tack == Some(tack.id) {
            pal.selected
        } else {
            pal.accent
        };
        let from = to_screen(at(&tack.a));
        mark(from, color);
        match &tack.b {
            TackTarget::Pattern(b) => {
                let to = to_screen(at(b));
                mark(to, color);
                painter.extend(Shape::dashed_line(
                    &[from, to],
                    Stroke::new(1.5, color),
                    6.0,
                    4.0,
                ));
            }
            TackTarget::Avatar(_) | TackTarget::AvatarSurface { .. } => {
                painter.text(
                    from + Px::new(8.0, -8.0),
                    Align2::LEFT_BOTTOM,
                    "avatar",
                    FontId::proportional(11.0),
                    color,
                );
            }
        }
    }
    if let Some(start) = &s.tack_start {
        let from = to_screen(start.at);
        mark(from, pal.preview);
        if let (Tool::Tack, Some(hover)) = (s.tool, s.canvas.hover) {
            painter.extend(Shape::dashed_line(
                &[from, to_screen(hover)],
                Stroke::new(1.5, pal.preview),
                6.0,
                4.0,
            ));
        }
    }

    match &s.canvas.sew_pending {
        Some(SewBuild::Edges { sides, active }) => {
            draw_group(&sides[0], Stroke::new(3.0, pal.selected));
            draw_group(&sides[1], Stroke::new(3.0, pal.preview));
            // Running lengths of both sides, by the pointer.
            let length = |group: &[SeamEdge]| -> f32 {
                group
                    .iter()
                    .filter_map(|e| {
                        s.doc
                            .pattern(e.pattern)
                            .map(|p| p.segment_length(e.start, e.end, TOLERANCE))
                    })
                    .sum()
            };
            let (a, b) = (length(&sides[0]), length(&sides[1]));
            let text = if *active == 0 && sides[1].is_empty() {
                format!("A {a:.1} mm")
            } else {
                format!("A {a:.1} mm · B {b:.1} mm · Δ {:.1} mm", (a - b).abs())
            };
            if let Some(hover) = s.canvas.hover {
                painter.text(
                    to_screen(hover) + Px::new(12.0, 12.0),
                    Align2::LEFT_TOP,
                    text,
                    FontId::monospace(12.0),
                    pal.preview,
                );
            }
        }
        Some(SewBuild::Point(point)) => mark(to_screen(point.at), pal.selected),
        None => {}
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

    if let Some((centre, handle)) = rotate_handle(s) {
        let (min_y, color) = (
            s.selection
                .primary()
                .and_then(|id| s.doc.pattern(id)?.bounds())
                .map_or(centre.y, |(min, _)| min.y),
            pal.selected,
        );
        let top = to_screen(Vec2::new(centre.x, min_y));
        let at = to_screen(handle);
        painter.line_segment([top, at], Stroke::new(1.0, color));
        painter.circle_filled(at, HANDLE_PX, pal.canvas_bg);
        painter.circle_stroke(at, HANDLE_PX, Stroke::new(2.0, color));
    }
    if let Some(Drag::Rotate { degrees, pivot, .. }) = &canvas.drag {
        painter.text(
            to_screen(*pivot) + Px::new(12.0, -12.0),
            Align2::LEFT_BOTTOM,
            format!("{degrees:+.0}°"),
            FontId::monospace(12.0),
            pal.preview,
        );
    }

    if let Some(Drag::Select { start, end, .. }) = &canvas.drag {
        let box_rect = Rect::from_two_pos(to_screen(*start), to_screen(*end));
        painter.rect_filled(box_rect, 0.0, pal.preview.gamma_multiply(0.08));
        painter.rect_stroke(
            box_rect,
            0.0,
            Stroke::new(1.0, pal.preview),
            egui::StrokeKind::Inside,
        );
    }

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

    if let Some(Drag::Line { start, end, .. }) = &canvas.drag {
        let stroke = Stroke::new(1.5, pal.preview);
        if s.tool == Tool::Dart {
            let dart = Dart {
                start: *start,
                end: *end,
                width: canvas.dart_width,
            };
            draw_dart(painter, rect, canvas, &dart, stroke);
        } else if let Some(pattern) = s.selection.primary().and_then(|id| s.doc.pattern(id)) {
            let p = canvas.pleat;
            let lines = edit::pleat_lines(pattern, *start, *end, p.kind, p.count, p.depth, p.angle);
            for line in lines {
                let pts: Vec<Pos2> = line.points.iter().map(|q| to_screen(*q)).collect();
                painter.extend(Shape::dashed_line(&pts, stroke, 6.0, 4.0));
            }
            painter.line_segment([to_screen(*start), to_screen(*end)], stroke);
        }
        readout(*start, *end);
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

    let drafting = matches!(s.tool, Tool::Pen | Tool::InternalLine | Tool::Topstitch);
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
