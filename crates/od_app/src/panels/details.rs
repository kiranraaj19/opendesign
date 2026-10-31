//! Garment details in the 2D window and Properties: the Zipper tool, the
//! zippers drawn along their seams (tape, teeth and slider), and the
//! zipper settings of a selected seam; the Buttonhole tool, which places
//! buttonholes and fastens buttons into them; the Binding tool, binding
//! and piping drawn along their edges, and a piece's bindings in
//! Properties; gather marks along puckered seams and the Pucker setting
//! of a selected seam.

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, Shape, Stroke};
use od_core::details::{fastenings, Fasten, Unfasten};
use od_core::{
    AddBinding, AddSewingLine, AddTrim, Batch, Binding, BindingKind, Command, Pattern, PatternId,
    Pucker, RemoveTrim, SeamEdge, SetBinding, SetPucker, SetZipper, SewingLine, SewingLineId,
    TackPoint, Trim, TrimKind, Vec2, Zipper,
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
    /// The button picked to fasten, waiting for its buttonhole.
    pub button_start: Option<TackPoint>,
    /// Which way new buttonholes' slits run (see [`Trim::angle`]).
    pub hole_angle: f32,
    /// What the Binding tool sews on, and how wide.
    pub binding_kind: BindingKind,
    pub binding_width: f32,
}

impl Default for Details2D {
    fn default() -> Self {
        Self {
            zipper_first: None,
            drag_start: None,
            tape_width: Zipper::DEFAULT_TAPE_WIDTH,
            button_start: None,
            hole_angle: 0.0,
            binding_kind: BindingKind::Binding,
            binding_width: Binding::DEFAULT_WIDTH,
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

/// The topmost shown pattern with a trim of `kind` within reach of `p`:
/// the pattern, the trim's index and where it is.
fn trim_near(s: &Session, p: Vec2, kind: TrimKind) -> Option<(PatternId, usize, Vec2)> {
    let reach = s.canvas.px(PICK_PX);
    s.doc
        .patterns
        .iter()
        .rev()
        .filter(|pat| !pat.hidden)
        .find_map(|pat| {
            let i = pat
                .trims
                .iter()
                .position(|t| t.kind == kind && t.pos.distance(p) <= reach)?;
            Some((pat.id, i, pat.trims[i].pos))
        })
}

/// The Buttonhole tool: a click inside a piece places a buttonhole, a
/// click on one removes it (unfastening its button). A click on a button
/// and then on a buttonhole fastens the button into it; a click on a
/// fastened button unfastens it.
pub fn handle_buttonhole(ui: &egui::Ui, response: &egui::Response, rect: Rect, s: &mut Session) {
    let typing = ui.memory(|m| m.focused().is_some());
    if !typing && ui.input(|i| i.key_pressed(Key::Escape)) {
        s.canvas.details.button_start = None;
    }
    if !response.clicked() {
        return;
    }
    let Some(screen) = response.interact_pointer_pos() else {
        return;
    };
    let p = s.canvas.to_doc(rect, screen);
    if let Some(button) = s.canvas.details.button_start.take() {
        if let Some((pattern, _, at)) = trim_near(s, p, TrimKind::Buttonhole) {
            s.execute(Fasten::new(button, TackPoint { pattern, at }));
        }
        return;
    }
    if let Some((pattern, index, at)) = trim_near(s, p, TrimKind::Button) {
        s.selection.set(Some(pattern));
        let fastened = fastenings(&s.doc)
            .into_iter()
            .find(|f| f.button == (pattern, index));
        match fastened {
            Some(f) => s.execute(Unfasten::new(f.tack)),
            None => s.canvas.details.button_start = Some(TackPoint { pattern, at }),
        }
        return;
    }
    if let Some((pattern, index, _)) = trim_near(s, p, TrimKind::Buttonhole) {
        s.selection.set(Some(pattern));
        let mut steps: Vec<Box<dyn Command>> = fastenings(&s.doc)
            .into_iter()
            .filter(|f| f.hole == (pattern, index))
            .map(|f| Box::new(Unfasten::new(f.tack)) as Box<dyn Command>)
            .collect();
        let (target, _) = s.edit_target(pattern);
        steps.push(Box::new(RemoveTrim::new(target, index)));
        s.execute(Batch::new("Remove buttonhole", steps));
        return;
    }
    let Some(id) = s.doc.pattern_at(p) else {
        return;
    };
    s.selection.set(Some(id));
    let (target, link) = s.edit_target(id);
    let mut angle = s.canvas.details.hole_angle;
    let mut at = p;
    if let Some(link) = link {
        // Placed through a linked copy: on its source, turned to match.
        let (sin, cos) = angle.to_radians().sin_cos();
        let d = link.to_source(p + Vec2::new(cos, sin)) - link.to_source(p);
        angle = d.y.atan2(d.x).to_degrees();
        at = link.to_source(p);
    }
    s.execute(AddTrim::new(target, TrimKind::Buttonhole, at).at_angle(angle));
}

/// The Binding tool: a click on an edge binds that corner-to-corner
/// segment, a drag along an outline the stretch it covers; picking a
/// stretch that is already bound takes the binding off.
pub fn handle_binding(response: &egui::Response, rect: Rect, s: &mut Session) {
    let Some(edge) = picked_edge(response, rect, s) else {
        return;
    };
    s.selection.set(Some(edge.pattern));
    let (target, _) = s.edit_target(edge.pattern);
    let bound = s.doc.pattern(target).and_then(|p| {
        p.bindings
            .iter()
            .position(|b| same_stretch(&b.edge(target), &edge))
    });
    match bound {
        Some(index) => s.execute(SetBinding::new(target, index, None)),
        None => {
            let binding = Binding {
                kind: s.canvas.details.binding_kind,
                start: edge.start,
                end: edge.end,
                width: s.canvas.details.binding_width,
            };
            s.execute(AddBinding::new(target, binding));
        }
    }
}

/// The colour binding or piping is drawn in.
pub fn binding_color(kind: BindingKind, pal: &theme::Palette) -> Color32 {
    match kind {
        BindingKind::Binding => pal.seams[2],
        BindingKind::Piping => pal.seams[4],
    }
}

/// Binding along an edge: a band folded over it, half its width either
/// side. Piping: a cord standing just outside it.
fn draw_binding(
    painter: &egui::Painter,
    s: &Session,
    pattern: &Pattern,
    binding: &Binding,
    pal: &theme::Palette,
    to_screen: &impl Fn(Vec2) -> Pos2,
) {
    let points = pattern.edge_points(&binding.edge(pattern.id));
    let color = binding_color(binding.kind, pal);
    match binding.kind {
        BindingKind::Binding => {
            let half = binding.width * 0.5;
            for inner in band(pattern, &points, half) {
                // The inner half, then the outer half mirrored over the edge.
                let [a, b, c, d] = inner;
                let outer = [a, b, b * 2.0 - c, a * 2.0 - d];
                for q in [inner, outer] {
                    painter.add(Shape::convex_polygon(
                        q.map(to_screen).to_vec(),
                        color.gamma_multiply(0.45),
                        Stroke::NONE,
                    ));
                }
            }
        }
        BindingKind::Piping => {
            let width = (binding.width * 0.5).max(1.0);
            let cord: Vec<Pos2> = points
                .windows(2)
                .flat_map(|w| {
                    let n = -inward(pattern, w[0], w[1]) * (width * 0.5);
                    [w[0] + n, w[1] + n]
                })
                .map(to_screen)
                .collect();
            let px = (width / s.canvas.px(1.0)).max(2.0);
            painter.add(Shape::line(cord, Stroke::new(px, color)));
        }
    }
}

/// A buttonhole: its slit, with a bar across each end.
pub fn draw_buttonhole(
    painter: &egui::Painter,
    trim: &Trim,
    to_screen: &impl Fn(Vec2) -> Pos2,
    color: Color32,
) {
    let [a, b] = trim.slit().map(to_screen);
    painter.line_segment([a, b], Stroke::new(2.5, color));
    let d = (b - a).normalized();
    let bar = egui::vec2(-d.y, d.x) * 4.0;
    for end in [a, b] {
        painter.line_segment([end - bar, end + bar], Stroke::new(1.5, color));
    }
}

/// What the detail tools say they do, at the top of the 2D window.
pub fn hint(s: &Session) -> &'static str {
    match s.tool {
        Tool::Binding => {
            "Click an edge (or drag along one) to bind it · click a bound edge to take the binding off · kind and width in Properties"
        }
        Tool::Buttonhole if s.canvas.details.button_start.is_some() => {
            "Click the buttonhole to fasten the button into · Esc cancels"
        }
        Tool::Buttonhole => {
            "Click to place a buttonhole · click a button, then a buttonhole, to fasten them · click a buttonhole or fastened button to undo it"
        }
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
    for (i, line) in s.doc.sewing_lines.iter().enumerate() {
        let color = crate::panels::pattern2d::seam_color(s, i, pal);
        draw_pucker(painter, s, line, color, &to_screen);
    }
    for pattern in s.doc.patterns.iter().filter(|p| !p.hidden) {
        for binding in &pattern.bindings {
            draw_binding(painter, s, pattern, binding, pal, &to_screen);
        }
    }
    // A button over each buttonhole it is fastened into.
    for f in fastenings(&s.doc) {
        let hole = s
            .doc
            .pattern(f.hole.0)
            .filter(|p| !p.hidden)
            .and_then(|p| p.trims.get(f.hole.1));
        if let Some(hole) = hole {
            painter.circle_stroke(to_screen(hole.pos), 7.0, Stroke::new(1.5, pal.selected));
        }
    }
    if let Some(start) = &s.canvas.details.button_start {
        let from = to_screen(start.at);
        painter.circle_stroke(from, 7.0, Stroke::new(2.0, pal.preview));
        if let (Tool::Buttonhole, Some(hover)) = (s.tool, s.canvas.hover) {
            painter.extend(Shape::dashed_line(
                &[from, to_screen(hover)],
                Stroke::new(1.5, pal.preview),
                6.0,
                4.0,
            ));
        }
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
    if s.tool == Tool::Binding {
        ui.add_space(8.0);
        ui.heading("Binding tool");
        let d = &mut s.canvas.details;
        ui.horizontal(|ui| {
            for kind in [BindingKind::Binding, BindingKind::Piping] {
                ui.selectable_value(&mut d.binding_kind, kind, kind.label());
            }
            ui.label("Width");
            ui.add(
                egui::DragValue::new(&mut d.binding_width)
                    .range(Binding::WIDTH_RANGE)
                    .suffix(" mm")
                    .speed(0.5),
            );
        });
    }
    if s.tool == Tool::Buttonhole {
        ui.add_space(8.0);
        ui.heading("Buttonhole tool");
        let angle = &mut s.canvas.details.hole_angle;
        ui.horizontal(|ui| {
            ui.label("Slit");
            ui.selectable_value(angle, 0.0, "Across");
            ui.selectable_value(angle, 90.0, "Upright");
            ui.add(
                egui::DragValue::new(angle)
                    .range(-180.0..=180.0)
                    .suffix("°"),
            );
        });
    }
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

/// A selected seam's puckering: on or off, and how much further than its
/// shorter side it is gathered.
fn pucker_row(ui: &mut egui::Ui, s: &mut Session, id: SewingLineId) {
    let Some(line) = s.doc.sewing_line(id) else {
        return;
    };
    let pucker = line.details.pucker;
    ui.add_space(8.0);
    let mut set = None;
    ui.horizontal(|ui| {
        let mut on = pucker.is_some();
        if ui
            .checkbox(&mut on, "Pucker")
            .on_hover_text(
                "Gather the longer side in to the shorter one, so the extra cloth ripples along the seam",
            )
            .changed()
        {
            set = Some(on.then(Pucker::default));
        }
        if let Some(pucker) = pucker {
            ui.label("Extra gather");
            let percent = |r: std::ops::RangeInclusive<f32>| r.start() * 100.0..=r.end() * 100.0;
            if let Some(amount) = deferred_value(
                ui,
                ("pucker", id),
                pucker.amount * 100.0,
                percent(Pucker::AMOUNT_RANGE),
                "%",
            ) {
                set = Some(Some(Pucker {
                    amount: amount / 100.0,
                }));
            }
        }
    });
    if let Some(pucker) = set {
        s.execute(SetPucker::new(id, pucker));
    }
}

/// Gather marks along each side of a puckered seam that is drawn in:
/// short ticks, closer together the more it is gathered.
fn draw_pucker(
    painter: &egui::Painter,
    s: &Session,
    line: &SewingLine,
    color: Color32,
    to_screen: &impl Fn(Vec2) -> Pos2,
) {
    let Some(pucker) = line.details.pucker else {
        return;
    };
    let Some((a, b)) = line.side_lengths(&s.doc) else {
        return;
    };
    let sewn = pucker.sewn_length(a, b);
    for (group, length) in [(&line.a, a), (&line.b, b)] {
        if length <= 0.0 || sewn >= length * 0.999 {
            continue;
        }
        let step = GATHER_MM * sewn / length;
        let reach = s.canvas.px(5.0);
        for edge in group {
            let Some(pattern) = s.doc.pattern(edge.pattern).filter(|p| !p.hidden) else {
                continue;
            };
            let points = pattern.edge_points(edge);
            for (at, i) in every(&points, step) {
                let n = inward(pattern, points[i], points[i + 1]);
                let d = points[i + 1] - points[i];
                let along = d * (reach * 0.4 / d.length().max(1e-6));
                // A small arc of gathered cloth: in and back out.
                let pts = [at - along, at + n * reach, at + along].map(to_screen);
                painter.add(Shape::line(pts.to_vec(), Stroke::new(1.2, color)));
            }
        }
    }
}

/// Gather marks on a puckered side are this far apart when it isn't
/// drawn in at all, in mm; closer the more it is.
const GATHER_MM: f32 = 8.0;

/// A selected seam's zipper: open or closed and its tape, or a way to
/// make the seam one.
pub fn seam_section(ui: &mut egui::Ui, s: &mut Session, id: SewingLineId) {
    pucker_row(ui, s, id);
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

/// A piece's binding and piping, in Properties: each one's kind, width
/// and length, and a way to take it off.
pub fn pattern_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(pattern) = s.doc.pattern(id) else {
        return;
    };
    if pattern.bindings.is_empty() {
        return;
    }
    let bindings = pattern.bindings.clone();
    let lengths: Vec<f32> = bindings
        .iter()
        .map(|b| pattern.segment_length(b.start, b.end, od_core::document::TOLERANCE))
        .collect();
    let (target, _) = s.edit_target(id);
    ui.add_space(8.0);
    ui.label("Binding and piping");
    let mut set = None;
    for (i, (binding, length)) in bindings.iter().zip(lengths).enumerate() {
        ui.horizontal_wrapped(|ui| {
            for kind in [BindingKind::Binding, BindingKind::Piping] {
                if ui
                    .selectable_label(binding.kind == kind, kind.label())
                    .clicked()
                    && binding.kind != kind
                {
                    set = Some((i, Some(Binding { kind, ..*binding })));
                }
            }
            if let Some(width) = deferred_value(
                ui,
                ("binding_width", id, i),
                binding.width,
                Binding::WIDTH_RANGE,
                " mm wide",
            ) {
                set = Some((i, Some(Binding { width, ..*binding })));
            }
            ui.label(format!("{length:.0} mm long"));
            if ui.button(format!("Remove binding {}", i + 1)).clicked() {
                set = Some((i, None));
            }
        });
    }
    if let Some((i, binding)) = set {
        s.execute(SetBinding::new(target, i, binding));
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
