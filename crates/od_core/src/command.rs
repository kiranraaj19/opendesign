//! Commands are the only way to change a [`Document`]. Each one knows how to
//! apply itself and how to reverse that change.

use crate::document::{
    normalize_degrees, rotate_point, Arrangement, Axis, Dart, Document, Fabric, Fold, Grading,
    InternalLine, Notch, OutlinePoint, Pattern, PatternId, PointKind, SeamEdge, SewingLine,
    SewingLineId, Tack, TackId, TackPoint, TackTarget, Topstitch, Trim, TrimKind, TOLERANCE,
};
use crate::geometry::{self, Vec2};
use crate::seam::{self, Side};

pub trait Command {
    /// Short label shown in the History panel and the Edit menu.
    fn label(&self) -> String;
    fn apply(&mut self, doc: &mut Document);
    fn undo(&mut self, doc: &mut Document);
}

/// Several commands as one undo step, e.g. moving every selected pattern.
/// They apply in order and undo in reverse.
pub struct Batch {
    label: String,
    commands: Vec<Box<dyn Command>>,
}

impl Batch {
    pub fn new(label: impl Into<String>, commands: Vec<Box<dyn Command>>) -> Self {
        Self {
            label: label.into(),
            commands,
        }
    }
}

impl Command for Batch {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        for command in &mut self.commands {
            command.apply(doc);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        for command in self.commands.iter_mut().rev() {
            command.undo(doc);
        }
    }
}

pub struct AddPattern {
    /// The pattern to add; its id is replaced by a fresh one.
    pattern: Pattern,
    /// Assigned on first apply so redo recreates the same id.
    id: Option<PatternId>,
}

impl AddPattern {
    /// A pattern with straight edges through `outline`.
    pub fn new(name: impl Into<String>, outline: Vec<Vec2>) -> Self {
        Self::with_outline(
            name,
            outline.into_iter().map(OutlinePoint::corner).collect(),
        )
    }

    pub fn with_outline(name: impl Into<String>, outline: Vec<OutlinePoint>) -> Self {
        Self::from_pattern(Pattern::new(PatternId(0), name, outline))
    }

    /// Add a copy of `pattern`, markings and all, under a new id.
    pub fn from_pattern(pattern: Pattern) -> Self {
        Self { pattern, id: None }
    }

    /// Id of the created pattern, available after the command has run.
    pub fn id(&self) -> Option<PatternId> {
        self.id
    }
}

impl Command for AddPattern {
    fn label(&self) -> String {
        format!("Add {}", self.pattern.name)
    }

    fn apply(&mut self, doc: &mut Document) {
        let id = *self.id.get_or_insert_with(|| doc.allocate_id());
        doc.patterns.push(Pattern {
            id,
            ..self.pattern.clone()
        });
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(id) = self.id {
            doc.patterns.retain(|p| p.id != id);
        }
    }
}

/// Delete a pattern, and the seams and tacks that hold it.
pub struct DeletePattern {
    id: PatternId,
    /// The removed pattern and its position, kept for undo.
    removed: Option<(usize, Pattern)>,
    /// All seams and tacks before the delete, restored on undo.
    sewing_lines: Vec<SewingLine>,
    tacks: Vec<Tack>,
}

impl DeletePattern {
    pub fn new(id: PatternId) -> Self {
        Self {
            id,
            removed: None,
            sewing_lines: Vec::new(),
            tacks: Vec::new(),
        }
    }
}

impl Command for DeletePattern {
    fn label(&self) -> String {
        match &self.removed {
            Some((_, p)) => format!("Delete {}", p.name),
            None => "Delete pattern".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(index) = doc.index_of(self.id) {
            self.removed = Some((index, doc.patterns.remove(index)));
            self.sewing_lines = doc.sewing_lines.clone();
            doc.sewing_lines.retain(|s| !s.touches(self.id));
            self.tacks = doc.tacks.clone();
            doc.tacks.retain(|t| !t.touches(self.id));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((index, pattern)) = self.removed.take() {
            doc.patterns.insert(index.min(doc.patterns.len()), pattern);
            doc.sewing_lines = std::mem::take(&mut self.sewing_lines);
            doc.tacks = std::mem::take(&mut self.tacks);
        }
    }
}

pub struct MovePattern {
    id: PatternId,
    delta: Vec2,
    /// The pattern and tacks before the move, restored on undo so that
    /// undo is exact (adding and subtracting a float delta can be off in
    /// the last bit).
    before: Option<Pattern>,
    tacks: Vec<Tack>,
}

impl MovePattern {
    pub fn new(id: PatternId, delta: Vec2) -> Self {
        Self {
            id,
            delta,
            before: None,
            tacks: Vec::new(),
        }
    }
}

impl Command for MovePattern {
    fn label(&self) -> String {
        "Move pattern".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            self.before = Some(p.clone());
            p.translate(self.delta);
            let delta = self.delta;
            self.tacks = move_tack_points(doc, self.id, |p| p + delta);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(before)) = (doc.pattern_mut(self.id), self.before.take()) {
            *p = before;
            doc.tacks = std::mem::take(&mut self.tacks);
        }
    }
}

/// A pattern's on/off settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternFlag {
    Hidden,
    Locked,
}

impl PatternFlag {
    pub fn get(self, p: &Pattern) -> bool {
        match self {
            PatternFlag::Hidden => p.hidden,
            PatternFlag::Locked => p.locked,
        }
    }

    fn get_mut(self, p: &mut Pattern) -> &mut bool {
        match self {
            PatternFlag::Hidden => &mut p.hidden,
            PatternFlag::Locked => &mut p.locked,
        }
    }
}

/// Hide or show, lock or unlock a pattern.
pub struct SetPatternFlag {
    id: PatternId,
    flag: PatternFlag,
    value: bool,
    before: Option<bool>,
}

impl SetPatternFlag {
    pub fn new(id: PatternId, flag: PatternFlag, value: bool) -> Self {
        Self {
            id,
            flag,
            value,
            before: None,
        }
    }
}

impl Command for SetPatternFlag {
    fn label(&self) -> String {
        match (self.flag, self.value) {
            (PatternFlag::Hidden, true) => "Hide pattern",
            (PatternFlag::Hidden, false) => "Show pattern",
            (PatternFlag::Locked, true) => "Lock pattern",
            (PatternFlag::Locked, false) => "Unlock pattern",
        }
        .into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            let flag = self.flag.get_mut(p);
            self.before = Some(*flag);
            *flag = self.value;
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(before)) = (doc.pattern_mut(self.id), self.before) {
            *self.flag.get_mut(p) = before;
        }
    }
}

/// Place a pattern in 3D, or (`None`) give it back to the automatic
/// layout around the avatar.
pub struct SetArrangement {
    id: PatternId,
    arrangement: Option<Arrangement>,
    before: Option<Option<Arrangement>>,
}

impl SetArrangement {
    pub fn new(id: PatternId, arrangement: Option<Arrangement>) -> Self {
        Self {
            id,
            arrangement,
            before: None,
        }
    }
}

impl Command for SetArrangement {
    fn label(&self) -> String {
        match self.arrangement {
            Some(_) => "Arrange pattern in 3D",
            None => "Reset 3D position",
        }
        .into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            self.before = Some(std::mem::replace(&mut p.arrangement, self.arrangement));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(before)) = (doc.pattern_mut(self.id), self.before.take()) {
            p.arrangement = before;
        }
    }
}

pub struct RenamePattern {
    id: PatternId,
    name: String,
}

impl RenamePattern {
    pub fn new(id: PatternId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

impl Command for RenamePattern {
    fn label(&self) -> String {
        format!("Rename to {}", self.name)
    }

    // Swapping makes apply and undo the same operation.
    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.name, &mut self.name);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Move one outline point to a new position.
pub struct MovePoint {
    id: PatternId,
    index: usize,
    /// Swapped with the point's position on apply and undo, so both are
    /// exact.
    pos: Vec2,
}

impl MovePoint {
    pub fn new(id: PatternId, index: usize, to: Vec2) -> Self {
        Self { id, index, pos: to }
    }
}

impl Command for MovePoint {
    fn label(&self) -> String {
        "Move point".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc
            .pattern_mut(self.id)
            .and_then(|p| p.outline.get_mut(self.index))
        {
            std::mem::swap(&mut p.pos, &mut self.pos);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Insert an outline point before `index` (`index == len` appends, which
/// puts it on the closing edge).
pub struct InsertPoint {
    id: PatternId,
    index: usize,
    point: OutlinePoint,
    /// Seams, grading and bindings before the insert, restored exactly on
    /// undo.
    sewing_lines: Vec<SewingLine>,
    grading: Grading,
    bindings: Vec<crate::details::Binding>,
}

impl InsertPoint {
    pub fn new(id: PatternId, index: usize, point: OutlinePoint) -> Self {
        Self {
            id,
            index,
            point,
            sewing_lines: Vec::new(),
            grading: Grading::default(),
            bindings: Vec::new(),
        }
    }
}

impl Command for InsertPoint {
    fn label(&self) -> String {
        "Insert point".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let Some(p) = doc.pattern_mut(self.id) else {
            return;
        };
        let before = p.clone();
        self.index = self.index.min(p.outline.len());
        p.outline.insert(self.index, self.point);
        p.grading = Grading::carried_over(&before, &p.outline);
        self.grading = before.grading.clone();
        let index = self.index;
        let remap = |b: &Pattern, a: &Pattern, t| seam::param_after_insert(b, a, index, t);
        self.sewing_lines = remap_sewing(doc, &before, &remap);
        self.bindings = rebind(doc, &before, &remap);
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.outline.len() {
                p.outline.remove(self.index);
                p.grading = std::mem::take(&mut self.grading);
                p.bindings = std::mem::take(&mut self.bindings);
                doc.sewing_lines = std::mem::take(&mut self.sewing_lines);
            }
        }
    }
}

/// Remove an outline point. Does nothing if the outline would be left
/// with fewer than three points.
pub struct DeletePoint {
    id: PatternId,
    index: usize,
    removed: Option<OutlinePoint>,
    /// Seams, grading and bindings before the delete, restored exactly on
    /// undo.
    sewing_lines: Vec<SewingLine>,
    grading: Grading,
    bindings: Vec<crate::details::Binding>,
}

impl DeletePoint {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
            sewing_lines: Vec::new(),
            grading: Grading::default(),
            bindings: Vec::new(),
        }
    }
}

impl Command for DeletePoint {
    fn label(&self) -> String {
        "Delete point".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let Some(p) = doc.pattern_mut(self.id) else {
            return;
        };
        if p.outline.len() > 3 && self.index < p.outline.len() {
            let before = p.clone();
            self.removed = Some(p.outline.remove(self.index));
            p.grading = Grading::carried_over(&before, &p.outline);
            self.grading = before.grading.clone();
            let index = self.index;
            let remap = |b: &Pattern, a: &Pattern, t| seam::param_after_delete(b, a, index, t);
            self.sewing_lines = remap_sewing(doc, &before, &remap);
            self.bindings = rebind(doc, &before, &remap);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(point)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.outline.insert(self.index.min(p.outline.len()), point);
            p.grading = std::mem::take(&mut self.grading);
            p.bindings = std::mem::take(&mut self.bindings);
            doc.sewing_lines = std::mem::take(&mut self.sewing_lines);
        }
    }
}

/// Replace a pattern's whole outline, e.g. after rounding a corner or
/// adding a dart. Seam ends and notches are placed again at the nearest
/// point of the new outline to where they were.
pub struct SetOutline {
    id: PatternId,
    label: String,
    outline: Vec<OutlinePoint>,
    /// A stretch of the new outline, as parameters from and to, that seams
    /// are kept off.
    clear: Option<(f32, f32)>,
    /// The pattern and seams before the change, restored exactly on undo.
    before: Option<(Pattern, Vec<SewingLine>)>,
}

impl SetOutline {
    pub fn new(id: PatternId, label: impl Into<String>, outline: Vec<OutlinePoint>) -> Self {
        Self {
            id,
            label: label.into(),
            outline,
            clear: None,
            before: None,
        }
    }

    /// Keep seams off the new outline from parameter `from` forwards to
    /// `to` (a dart's V, say): a seam edge that ends up running into it is
    /// shortened, or split into the parts either side.
    pub fn keeping_clear(mut self, from: f32, to: f32) -> Self {
        self.clear = Some((from, to));
        self
    }
}

impl Command for SetOutline {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        let Some(p) = doc.pattern_mut(self.id) else {
            return;
        };
        if self.outline.len() < 3 {
            return;
        }
        let before = p.clone();
        p.outline = self.outline.clone();
        p.notches = remap_notches(&before, p);
        p.grading = Grading::carried_over(&before, &p.outline);
        p.bindings = crate::details::bindings_by_position(&before, p);
        let seams = remap_sewing(doc, &before, &|b, a, t| {
            a.nearest_param(b.pos_at_param(t, TOLERANCE)).0
        });
        if let Some((from, to)) = self.clear {
            let (id, n) = (self.id, self.outline.len() as f32);
            for line in &mut doc.sewing_lines {
                for side in [&mut line.a, &mut line.b] {
                    *side = side
                        .iter()
                        .flat_map(|e| {
                            if e.pattern == id {
                                seam::without_range(e, from, to, n)
                            } else {
                                vec![*e]
                            }
                        })
                        .collect();
                }
            }
            doc.sewing_lines
                .retain(|l| !l.a.is_empty() && !l.b.is_empty());
        }
        self.before = Some((before, seams));
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((before, seams)) = self.before.take() {
            if let Some(p) = doc.pattern_mut(self.id) {
                *p = before;
                doc.sewing_lines = seams;
            }
        }
    }
}

/// `after`'s notches: `before`'s, each moved to the nearest point of
/// `after`'s outline.
pub(crate) fn remap_notches(before: &Pattern, after: &Pattern) -> Vec<Notch> {
    let (old, new) = (before.polyline(TOLERANCE), after.polyline(TOLERANCE));
    before
        .notches
        .iter()
        .filter_map(|n| {
            let (pos, _) = geometry::point_at_distance(&old, n.at)?;
            let (at, _) = geometry::distance_along(&new, pos)?;
            Some(Notch { at })
        })
        .collect()
}

/// Make an outline point a corner or a curve point.
pub struct SetPointKind {
    id: PatternId,
    index: usize,
    kind: PointKind,
}

impl SetPointKind {
    pub fn new(id: PatternId, index: usize, kind: PointKind) -> Self {
        Self { id, index, kind }
    }
}

impl Command for SetPointKind {
    fn label(&self) -> String {
        "Change point type".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc
            .pattern_mut(self.id)
            .and_then(|p| p.outline.get_mut(self.index))
        {
            std::mem::swap(&mut p.kind, &mut self.kind);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// A numeric pattern property edited in Properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternValue {
    SeamAllowance,
    GrainAngle,
}

impl PatternValue {
    pub fn get(self, p: &Pattern) -> f32 {
        match self {
            PatternValue::SeamAllowance => p.seam_allowance,
            PatternValue::GrainAngle => p.grain_angle,
        }
    }

    /// Bring a value into range: allowances aren't negative, angles wrap.
    pub fn clean(self, v: f32) -> f32 {
        match self {
            PatternValue::SeamAllowance => v.max(0.0),
            PatternValue::GrainAngle => normalize_degrees(v),
        }
    }

    fn slot(self, p: &mut Pattern) -> &mut f32 {
        match self {
            PatternValue::SeamAllowance => &mut p.seam_allowance,
            PatternValue::GrainAngle => &mut p.grain_angle,
        }
    }
}

pub struct SetPatternValue {
    id: PatternId,
    field: PatternValue,
    value: f32,
}

impl SetPatternValue {
    pub fn new(id: PatternId, field: PatternValue, value: f32) -> Self {
        Self {
            id,
            field,
            value: field.clean(value),
        }
    }
}

impl Command for SetPatternValue {
    fn label(&self) -> String {
        match self.field {
            PatternValue::SeamAllowance => "Set seam allowance",
            PatternValue::GrainAngle => "Set grainline",
        }
        .into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(self.field.slot(p), &mut self.value);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Particle distances a pattern may be given, in mm: finer than this is
/// too slow to simulate, coarser stops looking like cloth.
pub const PARTICLE_DISTANCE_RANGE: std::ops::RangeInclusive<f32> = 5.0..=100.0;

/// Set (or with `None`, clear back to the default) how finely a pattern is
/// meshed for simulation.
pub struct SetParticleDistance {
    id: PatternId,
    value: Option<f32>,
}

impl SetParticleDistance {
    pub fn new(id: PatternId, value: Option<f32>) -> Self {
        let (lo, hi) = (
            *PARTICLE_DISTANCE_RANGE.start(),
            *PARTICLE_DISTANCE_RANGE.end(),
        );
        Self {
            id,
            value: value.map(|v| v.clamp(lo, hi)),
        }
    }
}

impl Command for SetParticleDistance {
    fn label(&self) -> String {
        "Set particle distance".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.particle_distance, &mut self.value);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Assign a pattern's fabric, e.g. from the properties panel's picker.
pub struct SetFabric {
    id: PatternId,
    /// Swapped with the pattern's fabric on `apply`/`undo`, so after
    /// `apply` this holds the *previous* fabric (what undo restores) —
    /// `name` below is what the label should keep showing regardless.
    fabric: Fabric,
    /// The fabric being set, named once at construction so `label()` still
    /// reports it correctly after `apply` has swapped `fabric` to the old
    /// value.
    name: String,
}

impl SetFabric {
    pub fn new(id: PatternId, fabric: Fabric) -> Self {
        let name = fabric.name.clone();
        Self { id, fabric, name }
    }
}

impl Command for SetFabric {
    fn label(&self) -> String {
        format!("Set fabric: {}", self.name)
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.fabric, &mut self.fabric);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Assign a pattern's grading rule, e.g. from a "grade % per size" slider
/// in the properties panel.
pub struct SetGrading {
    id: PatternId,
    /// Swapped with the pattern's grading on `apply`/`undo`, so after
    /// `apply` this holds the *previous* rule (what undo restores).
    grading: Grading,
}

impl SetGrading {
    pub fn new(id: PatternId, grading: Grading) -> Self {
        Self { id, grading }
    }
}

impl Command for SetGrading {
    fn label(&self) -> String {
        "Set grading".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.grading, &mut self.grading);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

pub struct AddNotch {
    id: PatternId,
    notch: Notch,
}

impl AddNotch {
    /// A notch `at` mm along the outline (see [`Notch::at`]).
    pub fn new(id: PatternId, at: f32) -> Self {
        Self {
            id,
            notch: Notch { at },
        }
    }
}

impl Command for AddNotch {
    fn label(&self) -> String {
        "Add notch".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.notches.push(self.notch);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.notches.pop();
        }
    }
}

pub struct RemoveNotch {
    id: PatternId,
    index: usize,
    removed: Option<Notch>,
}

impl RemoveNotch {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
        }
    }
}

impl Command for RemoveNotch {
    fn label(&self) -> String {
        "Remove notch".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.notches.len() {
                self.removed = Some(p.notches.remove(self.index));
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(n)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.notches.insert(self.index.min(p.notches.len()), n);
        }
    }
}

pub struct AddInternalLine {
    id: PatternId,
    line: InternalLine,
}

impl AddInternalLine {
    pub fn new(id: PatternId, points: Vec<Vec2>) -> Self {
        Self {
            id,
            line: InternalLine::new(points),
        }
    }

    /// A fold line: the cloth folds along it when draped.
    pub fn folded(id: PatternId, points: Vec<Vec2>, fold: Fold) -> Self {
        Self {
            id,
            line: InternalLine {
                points,
                fold: Some(fold),
            },
        }
    }
}

/// Make an internal line a fold line, change its fold, or make it a plain
/// line again (`None`).
pub struct SetFold {
    id: PatternId,
    index: usize,
    /// Swapped with the line's fold on apply and undo, so both are exact.
    fold: Option<Fold>,
}

impl SetFold {
    pub fn new(id: PatternId, index: usize, fold: Option<Fold>) -> Self {
        Self { id, index, fold }
    }
}

impl Command for SetFold {
    fn label(&self) -> String {
        "Change fold".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(line) = doc
            .pattern_mut(self.id)
            .and_then(|p| p.internal_lines.get_mut(self.index))
        {
            std::mem::swap(&mut line.fold, &mut self.fold);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

impl Command for AddInternalLine {
    fn label(&self) -> String {
        "Add internal line".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.internal_lines.push(self.line.clone());
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.internal_lines.pop();
        }
    }
}

pub struct RemoveInternalLine {
    id: PatternId,
    index: usize,
    removed: Option<InternalLine>,
}

impl RemoveInternalLine {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
        }
    }
}

impl Command for RemoveInternalLine {
    fn label(&self) -> String {
        "Remove internal line".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.internal_lines.len() {
                self.removed = Some(p.internal_lines.remove(self.index));
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(line)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.internal_lines
                .insert(self.index.min(p.internal_lines.len()), line);
        }
    }
}

pub struct AddDart {
    id: PatternId,
    dart: Dart,
}

impl AddDart {
    pub fn new(id: PatternId, dart: Dart) -> Self {
        Self { id, dart }
    }
}

impl Command for AddDart {
    fn label(&self) -> String {
        "Add dart".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.darts.push(self.dart);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.darts.pop();
        }
    }
}

pub struct RemoveDart {
    id: PatternId,
    index: usize,
    removed: Option<Dart>,
}

impl RemoveDart {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
        }
    }
}

impl Command for RemoveDart {
    fn label(&self) -> String {
        "Remove dart".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.darts.len() {
                self.removed = Some(p.darts.remove(self.index));
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(dart)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.darts.insert(self.index.min(p.darts.len()), dart);
        }
    }
}

/// Change a dart inside a piece, e.g. its width.
pub struct SetDart {
    id: PatternId,
    index: usize,
    /// Swapped with the dart on apply and undo, so both are exact.
    dart: Dart,
}

impl SetDart {
    pub fn new(id: PatternId, index: usize, dart: Dart) -> Self {
        Self { id, index, dart }
    }
}

impl Command for SetDart {
    fn label(&self) -> String {
        "Edit dart".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(dart) = doc
            .pattern_mut(self.id)
            .and_then(|p| p.darts.get_mut(self.index))
        {
            std::mem::swap(dart, &mut self.dart);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

pub struct AddTopstitch {
    id: PatternId,
    line: Topstitch,
}

impl AddTopstitch {
    pub fn new(id: PatternId, points: Vec<Vec2>) -> Self {
        Self {
            id,
            line: Topstitch { points },
        }
    }
}

impl Command for AddTopstitch {
    fn label(&self) -> String {
        "Add topstitch".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.topstitches.push(self.line.clone());
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.topstitches.pop();
        }
    }
}

pub struct RemoveTopstitch {
    id: PatternId,
    index: usize,
    removed: Option<Topstitch>,
}

impl RemoveTopstitch {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
        }
    }
}

impl Command for RemoveTopstitch {
    fn label(&self) -> String {
        "Remove topstitch".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.topstitches.len() {
                self.removed = Some(p.topstitches.remove(self.index));
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(line)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.topstitches
                .insert(self.index.min(p.topstitches.len()), line);
        }
    }
}

pub struct AddTrim {
    id: PatternId,
    trim: Trim,
}

impl AddTrim {
    pub fn new(id: PatternId, kind: TrimKind, pos: Vec2) -> Self {
        Self {
            id,
            trim: Trim {
                kind,
                pos,
                angle: 0.0,
            },
        }
    }

    /// Turned `degrees` (a buttonhole's slit; see [`Trim::angle`]).
    pub fn at_angle(mut self, degrees: f32) -> Self {
        self.trim.angle = normalize_degrees(degrees);
        self
    }
}

impl Command for AddTrim {
    fn label(&self) -> String {
        match self.trim.kind {
            TrimKind::Buttonhole => "Add buttonhole".into(),
            _ => "Add trim".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.trims.push(self.trim);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            p.trims.pop();
        }
    }
}

pub struct RemoveTrim {
    id: PatternId,
    index: usize,
    removed: Option<Trim>,
}

impl RemoveTrim {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
        }
    }
}

impl Command for RemoveTrim {
    fn label(&self) -> String {
        "Remove trim".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.trims.len() {
                self.removed = Some(p.trims.remove(self.index));
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(trim)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.trims.insert(self.index.min(p.trims.len()), trim);
        }
    }
}

/// Flip a pattern in place about its own centre line.
pub struct MirrorPattern {
    id: PatternId,
    axis: Axis,
    /// The pattern and tacks before mirroring, restored exactly on undo.
    before: Option<Pattern>,
    tacks: Vec<Tack>,
}

impl MirrorPattern {
    pub fn new(id: PatternId, axis: Axis) -> Self {
        Self {
            id,
            axis,
            before: None,
            tacks: Vec::new(),
        }
    }
}

impl Command for MirrorPattern {
    fn label(&self) -> String {
        match self.axis {
            Axis::Vertical => "Flip horizontally",
            Axis::Horizontal => "Flip vertically",
        }
        .into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let axis = self.axis;
        if let Some(p) = doc.pattern_mut(self.id) {
            self.before = Some(p.clone());
            let c = p.center();
            let about = if axis == Axis::Vertical { c.x } else { c.y };
            p.mirror(axis, about);
            self.tacks = move_tack_points(doc, self.id, |p| axis.reflect(p, about));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            if let Some(p) = doc.pattern_mut(self.id) {
                *p = before;
                doc.tacks = std::mem::take(&mut self.tacks);
            }
        }
    }
}

/// Turn a pattern in place about the centre of its bounds.
pub struct RotatePattern {
    id: PatternId,
    degrees: f32,
    /// The pattern and tacks before turning, restored exactly on undo.
    before: Option<Pattern>,
    tacks: Vec<Tack>,
}

impl RotatePattern {
    /// Turn `degrees` clockwise on screen.
    pub fn new(id: PatternId, degrees: f32) -> Self {
        Self {
            id,
            degrees,
            before: None,
            tacks: Vec::new(),
        }
    }
}

impl Command for RotatePattern {
    fn label(&self) -> String {
        "Rotate pattern".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let degrees = self.degrees;
        if let Some(p) = doc.pattern_mut(self.id) {
            self.before = Some(p.clone());
            let c = p.center();
            p.rotate(degrees, c);
            self.tacks = move_tack_points(doc, self.id, |p| rotate_point(p, degrees, c));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            if let Some(p) = doc.pattern_mut(self.id) {
                *p = before;
                doc.tacks = std::mem::take(&mut self.tacks);
            }
        }
    }
}

/// Copy a pattern. A mirrored copy is reflected about the source's right
/// (or bottom) edge, so it sits beside it like the other half of a fold.
pub struct DuplicatePattern {
    source: PatternId,
    mirror: Option<Axis>,
    /// The copy, made on first apply so redo restores the same pattern.
    copy: Option<Pattern>,
}

/// How far a plain copy is offset from its source, in mm.
const COPY_OFFSET: f32 = 40.0;

impl DuplicatePattern {
    pub fn new(source: PatternId, mirror: Option<Axis>) -> Self {
        Self {
            source,
            mirror,
            copy: None,
        }
    }

    /// Id of the copy, available after the command has run.
    pub fn id(&self) -> Option<PatternId> {
        self.copy.as_ref().map(|p| p.id)
    }
}

impl Command for DuplicatePattern {
    fn label(&self) -> String {
        match self.mirror {
            Some(_) => "Duplicate mirrored".into(),
            None => "Duplicate pattern".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if self.copy.is_none() {
            let Some(source) = doc.pattern(self.source) else {
                return;
            };
            let mut copy = source.clone();
            let (_, max) = source.bounds().unwrap_or_default();
            match self.mirror {
                Some(axis) => {
                    copy.mirror(axis, if axis == Axis::Vertical { max.x } else { max.y });
                    copy.name = format!("{} (mirrored)", source.name);
                }
                None => {
                    copy.translate(Vec2::new(COPY_OFFSET, COPY_OFFSET));
                    copy.name = format!("{} (copy)", source.name);
                }
            }
            copy.id = doc.allocate_id();
            self.copy = Some(copy);
        }
        if let Some(copy) = &self.copy {
            doc.patterns.push(copy.clone());
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(id) = self.id() {
            doc.patterns.retain(|p| p.id != id);
        }
    }
}

/// Sew two sides together: each a group of one or more edges, so a group
/// with more than one edge makes a one-to-many seam.
pub struct AddSewingLine {
    a: Vec<SeamEdge>,
    b: Vec<SeamEdge>,
    details: crate::details::SeamDetails,
    /// Assigned on first apply so redo recreates the same id.
    id: Option<SewingLineId>,
}

impl AddSewingLine {
    pub fn new(a: Vec<SeamEdge>, b: Vec<SeamEdge>) -> Self {
        Self {
            a,
            b,
            details: Default::default(),
            id: None,
        }
    }

    /// Make the new seam a zipper.
    pub fn zipped(mut self, zipper: crate::details::Zipper) -> Self {
        self.details.zipper = Some(zipper);
        self
    }

    /// Id of the created seam, available after the command has run.
    pub fn id(&self) -> Option<SewingLineId> {
        self.id
    }
}

impl Command for AddSewingLine {
    fn label(&self) -> String {
        if self.details.zipper.is_some() {
            "Add zipper".into()
        } else {
            "Sew edges".into()
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        let id = *self.id.get_or_insert_with(|| doc.allocate_sewing_id());
        doc.sewing_lines.push(SewingLine {
            id,
            a: self.a.clone(),
            b: self.b.clone(),
            details: self.details,
        });
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(id) = self.id {
            doc.sewing_lines.retain(|s| s.id != id);
        }
    }
}

pub struct RemoveSewingLine {
    id: SewingLineId,
    /// The removed seam and its position, kept for undo.
    removed: Option<(usize, SewingLine)>,
}

impl RemoveSewingLine {
    pub fn new(id: SewingLineId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for RemoveSewingLine {
    fn label(&self) -> String {
        "Unsew edges".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(index) = doc.sewing_lines.iter().position(|s| s.id == self.id) {
            self.removed = Some((index, doc.sewing_lines.remove(index)));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((index, line)) = self.removed.take() {
            doc.sewing_lines
                .insert(index.min(doc.sewing_lines.len()), line);
        }
    }
}

/// After `before`'s outline points were renumbered, move every seam edge on
/// that pattern with `remap` (a parameter before, to one after). Seams left
/// with an empty edge are dropped. Returns all seams as they were, for undo.
pub(crate) fn remap_sewing(
    doc: &mut Document,
    before: &Pattern,
    remap: &impl Fn(&Pattern, &Pattern, f32) -> f32,
) -> Vec<SewingLine> {
    let old = doc.sewing_lines.clone();
    let Some(after) = doc.pattern(before.id).cloned() else {
        return old;
    };
    doc.sewing_lines.retain_mut(|line| {
        line.a
            .iter_mut()
            .chain(line.b.iter_mut())
            .filter(|e| e.pattern == before.id)
            .all(|e| match seam::remap_edge(e, before, &after, remap) {
                Some(moved) => {
                    *e = moved;
                    true
                }
                None => false,
            })
    });
    old
}

/// After `before`'s outline points were renumbered, move its bindings
/// with `remap` as [`remap_sewing`] moves seams. Returns them as they
/// were, for undo.
fn rebind(
    doc: &mut Document,
    before: &Pattern,
    remap: &impl Fn(&Pattern, &Pattern, f32) -> f32,
) -> Vec<crate::details::Binding> {
    let Some(after) = doc.pattern(before.id).cloned() else {
        return before.bindings.clone();
    };
    let moved = crate::details::remap_bindings(before, &after, remap);
    if let Some(p) = doc.pattern_mut(before.id) {
        p.bindings = moved;
    }
    before.bindings.clone()
}

/// Move every tack point on pattern `id` with `f`, as the pattern itself
/// moved. Returns all tacks as they were, for undo.
fn move_tack_points(doc: &mut Document, id: PatternId, f: impl Fn(Vec2) -> Vec2) -> Vec<Tack> {
    let old = doc.tacks.clone();
    for tack in &mut doc.tacks {
        for point in tack.points_mut().filter(|p| p.pattern == id) {
            point.at = f(point.at);
        }
    }
    old
}

/// Sew one side of a seam the other way round, to untwist it.
pub struct FlipSeamSide {
    id: SewingLineId,
    side: Side,
}

impl FlipSeamSide {
    pub fn new(id: SewingLineId, side: Side) -> Self {
        Self { id, side }
    }
}

impl Command for FlipSeamSide {
    fn label(&self) -> String {
        "Flip seam".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(line) = doc.sewing_line_mut(self.id) {
            let edges = line.side_mut(self.side);
            edges.reverse();
            for e in edges.iter_mut() {
                *e = e.reversed();
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        // Flipping twice restores the side exactly.
        self.apply(doc);
    }
}

/// Replace one edge of a seam, e.g. after dragging one of its ends.
pub struct SetSeamEdge {
    id: SewingLineId,
    side: Side,
    index: usize,
    /// Swapped with the seam's edge on apply and undo, so both are exact.
    edge: SeamEdge,
}

impl SetSeamEdge {
    pub fn new(id: SewingLineId, side: Side, index: usize, edge: SeamEdge) -> Self {
        Self {
            id,
            side,
            index,
            edge,
        }
    }
}

impl Command for SetSeamEdge {
    fn label(&self) -> String {
        "Edit sewing".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(edge) = doc
            .sewing_line_mut(self.id)
            .and_then(|line| line.side_mut(self.side).get_mut(self.index))
        {
            std::mem::swap(edge, &mut self.edge);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Hold a pattern point to another pattern point, or the avatar.
pub struct AddTack {
    a: TackPoint,
    b: TackTarget,
    /// Assigned on first apply so redo recreates the same id.
    id: Option<TackId>,
}

impl AddTack {
    pub fn new(a: TackPoint, b: TackTarget) -> Self {
        Self { a, b, id: None }
    }

    /// Id of the created tack, available after the command has run.
    pub fn id(&self) -> Option<TackId> {
        self.id
    }
}

impl Command for AddTack {
    fn label(&self) -> String {
        "Add tack".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let id = *self.id.get_or_insert_with(|| doc.allocate_tack_id());
        doc.tacks.push(Tack {
            id,
            a: self.a,
            b: self.b,
        });
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(id) = self.id {
            doc.tacks.retain(|t| t.id != id);
        }
    }
}

pub struct RemoveTack {
    id: TackId,
    /// The removed tack and its position, kept for undo.
    removed: Option<(usize, Tack)>,
}

impl RemoveTack {
    pub fn new(id: TackId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for RemoveTack {
    fn label(&self) -> String {
        "Remove tack".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(index) = doc.tacks.iter().position(|t| t.id == self.id) {
            self.removed = Some((index, doc.tacks.remove(index)));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((index, tack)) = self.removed.take() {
            doc.tacks.insert(index.min(doc.tacks.len()), tack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::rectangle;

    /// A document with one 100 × 50 rectangle, and that pattern's id.
    fn doc_with_rect() -> (Document, PatternId) {
        let mut doc = Document::default();
        let mut add = AddPattern::new("Panel", rectangle(Vec2::ZERO, Vec2::new(100.0, 50.0)));
        add.apply(&mut doc);
        let id = add.id().unwrap();
        (doc, id)
    }

    /// Apply, check, undo back to the start, redo, check again.
    fn round_trip(mut cmd: impl Command, check: impl Fn(&Document)) {
        let (mut doc, _) = doc_with_rect();
        // Patterns only: ids handed out stay used after undo, by design.
        let before = doc.patterns.clone();
        cmd.apply(&mut doc);
        assert_ne!(
            doc.patterns,
            before,
            "{} should change something",
            cmd.label()
        );
        check(&doc);
        let after = doc.patterns.clone();
        cmd.undo(&mut doc);
        assert_eq!(doc.patterns, before, "{} undo", cmd.label());
        cmd.apply(&mut doc);
        assert_eq!(doc.patterns, after, "{} redo", cmd.label());
    }

    const ID: PatternId = PatternId(1);

    fn pat(doc: &Document) -> &Pattern {
        doc.pattern(ID).unwrap()
    }

    #[test]
    fn add_keeps_markings_and_takes_a_new_id() {
        let (doc, id) = doc_with_rect();
        let mut template = doc.pattern(id).unwrap().clone();
        template.notches.push(Notch { at: 5.0 });
        let mut add = AddPattern::from_pattern(template);
        let mut doc2 = doc.clone();
        add.apply(&mut doc2);
        assert_ne!(add.id(), Some(id));
        assert_eq!(doc2.patterns[1].notches, [Notch { at: 5.0 }]);
        assert_eq!(doc2.patterns[1].seam_allowance, 10.0);
    }

    #[test]
    fn a_batch_applies_in_order_and_undoes_in_reverse() {
        let (mut doc, id) = doc_with_rect();
        let mut add = AddPattern::new("Other", rectangle(Vec2::ZERO, Vec2::new(10.0, 10.0)));
        add.apply(&mut doc);
        let other = add.id().unwrap();
        let before = doc.patterns.clone();
        let mut batch = Batch::new(
            "Move 2 patterns",
            vec![
                Box::new(MovePattern::new(id, Vec2::new(5.0, 0.0))),
                Box::new(MovePattern::new(other, Vec2::new(0.0, 7.0))),
                // Deleting after moving: undo must restore before un-moving.
                Box::new(DeletePattern::new(other)),
            ],
        );
        batch.apply(&mut doc);
        assert_eq!(batch.label(), "Move 2 patterns");
        assert_eq!(doc.patterns.len(), 1);
        assert_eq!(doc.pattern(id).unwrap().outline[0].pos, Vec2::new(5.0, 0.0));
        batch.undo(&mut doc);
        assert_eq!(doc.patterns, before);
    }

    #[test]
    fn set_outline_keeps_seams_and_notches_in_place() {
        let (mut doc, id) = doc_with_rect();
        doc.pattern_mut(id)
            .unwrap()
            .notches
            .push(Notch { at: 125.0 });
        // The right side, sewn to itself backwards just to have a seam.
        let mut sew = AddSewingLine::new(
            vec![SeamEdge {
                pattern: id,
                start: 1.0,
                end: 2.0,
                forward: true,
            }],
            vec![SeamEdge {
                pattern: id,
                start: 3.0,
                end: 4.0,
                forward: false,
            }],
        );
        sew.apply(&mut doc);
        let before = doc.clone();
        let rounded = crate::edit::round_corner(&pat(&doc).outline, 0, 10.0).unwrap();
        let mut cmd = SetOutline::new(id, "Round corner", rounded);
        cmd.apply(&mut doc);
        let p = pat(&doc);
        assert_eq!(p.outline.len(), 6);
        // Point 0 became three points, so the right side is now 3 to 4.
        let edge = doc.sewing_lines[0].a[0];
        assert!((edge.start - 3.0).abs() < 1e-3 && (edge.end - 4.0).abs() < 1e-3);
        // The notch half way down the right side stays there.
        let poly = p.polyline(TOLERANCE);
        let (pos, _) = geometry::point_at_distance(&poly, p.notches[0].at).unwrap();
        assert!(pos.distance(Vec2::new(100.0, 25.0)) < 1e-3, "{pos:?}");
        cmd.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn hiding_and_locking_round_trip() {
        round_trip(SetPatternFlag::new(ID, PatternFlag::Hidden, true), |d| {
            assert!(pat(d).hidden && !pat(d).locked);
        });
        round_trip(SetPatternFlag::new(ID, PatternFlag::Locked, true), |d| {
            assert!(pat(d).locked);
        });
    }

    #[test]
    fn rotating_turns_markings_and_tacks_and_four_quarters_go_round() {
        let (mut doc, id) = doc_with_rect();
        let p = doc.pattern_mut(id).unwrap();
        p.notches.push(Notch { at: 0.5 });
        p.trims.push(Trim {
            kind: TrimKind::Button,
            pos: Vec2::new(10.0, 25.0),
            angle: 0.0,
        });
        let tack_at = TackPoint {
            pattern: id,
            at: Vec2::new(90.0, 25.0),
        };
        AddTack::new(tack_at, TackTarget::Avatar([0.0; 3])).apply(&mut doc);
        let before = doc.clone();

        let mut turn = RotatePattern::new(id, 90.0);
        turn.apply(&mut doc);
        let p = doc.pattern(id).unwrap();
        // A 100 × 50 rectangle centred on (50, 25) becomes 50 × 100.
        let (min, max) = p.bounds().unwrap();
        assert_eq!((min, max), (Vec2::new(25.0, -25.0), Vec2::new(75.0, 75.0)));
        assert_eq!(p.grain_angle, 90.0);
        assert_eq!(p.trims[0].pos, Vec2::new(50.0, -15.0));
        assert_eq!(
            p.notches, before.patterns[0].notches,
            "notches ride the outline"
        );
        assert_eq!(doc.tacks[0].a.at, Vec2::new(50.0, 65.0));
        turn.undo(&mut doc);
        assert_eq!(doc, before);

        for _ in 0..4 {
            RotatePattern::new(id, 90.0).apply(&mut doc);
        }
        assert_eq!(doc.patterns, before.patterns);
        assert_eq!(doc.tacks, before.tacks);
    }

    #[test]
    fn arranging_in_3d_round_trips() {
        let at = Arrangement {
            center: [0.0, 1.2, 0.3],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
        };
        round_trip(SetArrangement::new(ID, Some(at)), |d| {
            assert_eq!(pat(d).arrangement, Some(at));
        });
    }

    #[test]
    fn move_point_round_trip() {
        round_trip(MovePoint::new(ID, 2, Vec2::new(130.0, 70.0)), |d| {
            assert_eq!(pat(d).outline[2].pos, Vec2::new(130.0, 70.0));
        });
    }

    #[test]
    fn insert_point_round_trip() {
        let point = OutlinePoint::curve(Vec2::new(50.0, -20.0));
        round_trip(InsertPoint::new(ID, 1, point), |d| {
            assert_eq!(pat(d).outline.len(), 5);
            assert_eq!(pat(d).outline[1], point);
        });
        // Appending lands on the closing edge.
        round_trip(InsertPoint::new(ID, 4, point), |d| {
            assert_eq!(pat(d).outline[4], point);
        });
    }

    #[test]
    fn delete_point_round_trip_and_minimum() {
        round_trip(DeletePoint::new(ID, 0), |d| {
            assert_eq!(pat(d).outline.len(), 3);
            assert_eq!(pat(d).outline[0].pos, Vec2::new(100.0, 0.0));
        });
        // A triangle keeps its three points.
        let (mut doc, id) = doc_with_rect();
        DeletePoint::new(id, 0).apply(&mut doc);
        let triangle = doc.clone();
        let mut again = DeletePoint::new(id, 0);
        again.apply(&mut doc);
        assert_eq!(doc, triangle);
        again.undo(&mut doc);
        assert_eq!(doc, triangle);
    }

    #[test]
    fn set_point_kind_round_trip() {
        round_trip(SetPointKind::new(ID, 3, PointKind::Curve), |d| {
            assert_eq!(pat(d).outline[3].kind, PointKind::Curve);
            assert!(pat(d).area() < 5000.0, "curve cuts the corner");
        });
    }

    #[test]
    fn set_values_round_trip_and_clean() {
        round_trip(
            SetPatternValue::new(ID, PatternValue::SeamAllowance, 15.0),
            |d| assert_eq!(pat(d).seam_allowance, 15.0),
        );
        round_trip(
            SetPatternValue::new(ID, PatternValue::GrainAngle, 270.0),
            |d| assert_eq!(pat(d).grain_angle, -90.0),
        );
        assert_eq!(PatternValue::SeamAllowance.clean(-3.0), 0.0);
    }

    #[test]
    fn set_fabric_round_trip() {
        round_trip(SetFabric::new(ID, Fabric::denim()), |d| {
            assert_eq!(pat(d).fabric.name, "Denim");
        });
        let (doc, id) = doc_with_rect();
        assert_eq!(doc.pattern(id).unwrap().fabric.name, "Cotton", "default");
    }

    #[test]
    fn set_grading_round_trip() {
        let (doc, id) = doc_with_rect();
        let grading = Grading::uniform_scale(&doc.pattern(id).unwrap().outline, 5.0);
        round_trip(SetGrading::new(ID, grading.clone()), |d| {
            assert_eq!(pat(d).grading, grading);
        });
        assert_eq!(
            doc.pattern(id).unwrap().grading,
            Grading::default(),
            "no grading by default"
        );
    }

    #[test]
    fn notches_round_trip() {
        round_trip(AddNotch::new(ID, 120.0), |d| {
            assert_eq!(pat(d).notches, [Notch { at: 120.0 }]);
        });
        let (mut doc, id) = doc_with_rect();
        AddNotch::new(id, 10.0).apply(&mut doc);
        AddNotch::new(id, 20.0).apply(&mut doc);
        let before = doc.clone();
        let mut remove = RemoveNotch::new(id, 0);
        remove.apply(&mut doc);
        assert_eq!(doc.pattern(id).unwrap().notches, [Notch { at: 20.0 }]);
        remove.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn internal_lines_round_trip() {
        let line = vec![Vec2::new(10.0, 10.0), Vec2::new(50.0, 40.0)];
        round_trip(AddInternalLine::new(ID, line.clone()), |d| {
            assert_eq!(pat(d).internal_lines[0].points, line);
        });
        let (mut doc, id) = doc_with_rect();
        AddInternalLine::new(id, line).apply(&mut doc);
        let before = doc.clone();
        let mut remove = RemoveInternalLine::new(id, 0);
        remove.apply(&mut doc);
        assert!(doc.pattern(id).unwrap().internal_lines.is_empty());
        remove.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn topstitches_round_trip() {
        let line = vec![Vec2::new(5.0, 5.0), Vec2::new(45.0, 5.0)];
        round_trip(AddTopstitch::new(ID, line.clone()), |d| {
            assert_eq!(pat(d).topstitches[0].points, line);
        });
        let (mut doc, id) = doc_with_rect();
        AddTopstitch::new(id, line).apply(&mut doc);
        let before = doc.clone();
        let mut remove = RemoveTopstitch::new(id, 0);
        remove.apply(&mut doc);
        assert!(doc.pattern(id).unwrap().topstitches.is_empty());
        remove.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn trims_round_trip() {
        round_trip(
            AddTrim::new(ID, TrimKind::Button, Vec2::new(20.0, 20.0)),
            |d| {
                assert_eq!(
                    pat(d).trims,
                    [Trim {
                        kind: TrimKind::Button,
                        pos: Vec2::new(20.0, 20.0),
                        angle: 0.0,
                    }]
                );
            },
        );
        let (mut doc, id) = doc_with_rect();
        AddTrim::new(id, TrimKind::Button, Vec2::new(10.0, 10.0)).apply(&mut doc);
        AddTrim::new(id, TrimKind::Snap, Vec2::new(30.0, 10.0)).apply(&mut doc);
        let before = doc.clone();
        let mut remove = RemoveTrim::new(id, 0);
        remove.apply(&mut doc);
        assert_eq!(doc.pattern(id).unwrap().trims.len(), 1);
        assert_eq!(doc.pattern(id).unwrap().trims[0].kind, TrimKind::Snap);
        remove.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn move_pattern_undo_is_exact() {
        // 0.1 and 0.7 are not exact in binary, so p + d - d can drift.
        let (mut doc, id) = doc_with_rect();
        MovePoint::new(id, 1, Vec2::new(-220.0, -300.0)).apply(&mut doc);
        let before = doc.patterns.clone();
        let mut mv = MovePattern::new(id, Vec2::new(0.1, 0.7));
        mv.apply(&mut doc);
        mv.undo(&mut doc);
        assert_eq!(doc.patterns, before);
    }

    #[test]
    fn move_pattern_carries_internal_lines() {
        let (mut doc, id) = doc_with_rect();
        AddInternalLine::new(id, vec![Vec2::ZERO, Vec2::new(5.0, 5.0)]).apply(&mut doc);
        AddTopstitch::new(id, vec![Vec2::ZERO, Vec2::new(5.0, 5.0)]).apply(&mut doc);
        AddTrim::new(id, TrimKind::Button, Vec2::ZERO).apply(&mut doc);
        MovePattern::new(id, Vec2::new(10.0, 0.0)).apply(&mut doc);
        assert_eq!(
            doc.pattern(id).unwrap().internal_lines[0].points[0],
            Vec2::new(10.0, 0.0)
        );
        assert_eq!(
            doc.pattern(id).unwrap().topstitches[0].points[0],
            Vec2::new(10.0, 0.0)
        );
        assert_eq!(doc.pattern(id).unwrap().trims[0].pos, Vec2::new(10.0, 0.0));
    }

    #[test]
    fn mirror_round_trip_is_exact() {
        let (mut doc, id) = doc_with_rect();
        // Make it asymmetric so the flip shows.
        MovePoint::new(id, 1, Vec2::new(80.0, 0.0)).apply(&mut doc);
        let before = doc.clone();
        let mut flip = MirrorPattern::new(id, Axis::Vertical);
        flip.apply(&mut doc);
        let p = doc.pattern(id).unwrap();
        assert_eq!(p.outline[1].pos, Vec2::new(20.0, 0.0));
        assert_eq!(p.bounds(), before.pattern(id).unwrap().bounds());
        flip.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn duplicate_round_trip() {
        round_trip(DuplicatePattern::new(ID, None), |d| {
            assert_eq!(d.patterns.len(), 2);
            assert_eq!(d.patterns[1].name, "Panel (copy)");
            assert_ne!(d.patterns[1].id, ID);
            assert_eq!(d.patterns[1].outline[0].pos, Vec2::new(40.0, 40.0));
        });
        round_trip(DuplicatePattern::new(ID, Some(Axis::Vertical)), |d| {
            let copy = &d.patterns[1];
            assert_eq!(copy.name, "Panel (mirrored)");
            // Reflected about x = 100: the copy spans 100..200.
            let (min, max) = copy.bounds().unwrap();
            assert_eq!((min.x, max.x), (100.0, 200.0));
        });
    }

    #[test]
    fn sewing_lines_round_trip() {
        let (mut doc, id) = doc_with_rect();
        let mut add2 = AddPattern::new("Panel 2", rectangle(Vec2::ZERO, Vec2::new(100.0, 50.0)));
        add2.apply(&mut doc);
        let id2 = add2.id().unwrap();
        // Ids handed out stay used after undo, by design (as for patterns),
        // so compare the sewing lines rather than the whole document.
        let before = doc.sewing_lines.clone();

        let a = vec![SeamEdge {
            pattern: id,
            start: 0.0,
            end: 1.0,
            forward: true,
        }];
        let b = vec![
            SeamEdge {
                pattern: id2,
                start: 2.0,
                end: 3.0,
                forward: true,
            },
            SeamEdge {
                pattern: id2,
                start: 3.0,
                end: 4.0,
                forward: true,
            },
        ];
        let mut sew = AddSewingLine::new(a.clone(), b.clone());
        sew.apply(&mut doc);
        let seam_id = sew.id().unwrap();
        assert_eq!(doc.sewing_lines.len(), 1);
        assert_eq!(doc.sewing_lines[0].a, a);
        assert_eq!(doc.sewing_lines[0].b, b);
        sew.undo(&mut doc);
        assert_eq!(doc.sewing_lines, before);
        sew.apply(&mut doc);
        assert_eq!(doc.sewing_lines[0].id, seam_id);

        let mut remove = RemoveSewingLine::new(seam_id);
        remove.apply(&mut doc);
        assert!(doc.sewing_lines.is_empty());
        remove.undo(&mut doc);
        assert_eq!(doc.sewing_lines[0].id, seam_id);
    }

    #[test]
    fn tacks_round_trip() {
        let (mut doc, id) = doc_with_rect();
        let before = doc.tacks.clone();

        let a = TackPoint {
            pattern: id,
            at: Vec2::ZERO,
        };
        let b = TackTarget::Pattern(TackPoint {
            pattern: id,
            at: Vec2::new(100.0, 50.0),
        });
        let mut tack = AddTack::new(a, b);
        tack.apply(&mut doc);
        let tack_id = tack.id().unwrap();
        assert_eq!(doc.tacks.len(), 1);
        assert_eq!(doc.tacks[0].a, a);
        assert_eq!(doc.tacks[0].b, b);
        tack.undo(&mut doc);
        assert_eq!(doc.tacks, before);
        tack.apply(&mut doc);
        assert_eq!(doc.tacks[0].id, tack_id);

        let mut remove = RemoveTack::new(tack_id);
        remove.apply(&mut doc);
        assert!(doc.tacks.is_empty());
        remove.undo(&mut doc);
        assert_eq!(doc.tacks[0].id, tack_id);
    }

    /// Two 100 × 50 rectangles side by side: "Panel" (0..100) and "Other"
    /// (150..250), Panel's right edge sewn top-down to Other's left edge,
    /// which runs bottom-up around its outline so is sewn backwards. Plus a
    /// tack between them and one from Panel to the avatar.
    fn sewn_pair() -> (Document, PatternId, PatternId, SewingLineId) {
        let (mut doc, a) = doc_with_rect();
        let mut add = AddPattern::new(
            "Other",
            rectangle(Vec2::new(150.0, 0.0), Vec2::new(250.0, 50.0)),
        );
        add.apply(&mut doc);
        let b = add.id().unwrap();
        let edge = |pattern, start, end, forward| SeamEdge {
            pattern,
            start,
            end,
            forward,
        };
        let mut sew = AddSewingLine::new(
            vec![edge(a, 1.0, 2.0, true)],
            vec![edge(b, 3.0, 4.0, false)],
        );
        sew.apply(&mut doc);
        let line = sew.id().unwrap();
        let point = |pattern, x, y| TackPoint {
            pattern,
            at: Vec2::new(x, y),
        };
        AddTack::new(
            point(a, 20.0, 10.0),
            TackTarget::Pattern(point(b, 170.0, 10.0)),
        )
        .apply(&mut doc);
        AddTack::new(point(a, 50.0, 25.0), TackTarget::Avatar([0.0, 1.0, 0.1])).apply(&mut doc);
        (doc, a, b, line)
    }

    #[test]
    fn backwards_edges_pair_matching_ends() {
        let (doc, _, _, id) = sewn_pair();
        let line = doc.sewing_line(id).unwrap();
        let start = |side| doc.side_point(line, side, 0.0).unwrap().1;
        assert_eq!(start(Side::A), Vec2::new(100.0, 0.0));
        assert_eq!(start(Side::B), Vec2::new(150.0, 0.0));
    }

    #[test]
    fn seams_follow_inserted_and_deleted_points() {
        let (mut doc, a, _, id) = sewn_pair();
        let before = doc.clone();
        // A point in the middle of the sewn right edge keeps the seam on it.
        let mut insert = InsertPoint::new(a, 2, OutlinePoint::corner(Vec2::new(100.0, 25.0)));
        insert.apply(&mut doc);
        let edge = doc.sewing_line(id).unwrap().a[0];
        assert_eq!((edge.start, edge.end, edge.forward), (1.0, 3.0, true));
        // A point before it shifts it along.
        let mut first = InsertPoint::new(a, 1, OutlinePoint::corner(Vec2::new(50.0, 0.0)));
        first.apply(&mut doc);
        assert_eq!(doc.sewing_line(id).unwrap().a[0].start, 2.0);
        first.undo(&mut doc);
        insert.undo(&mut doc);
        assert_eq!(doc, before);

        // Deleting the edge's lower corner joins it to the next edge.
        let mut delete = DeletePoint::new(a, 2);
        delete.apply(&mut doc);
        let edge = doc.sewing_line(id).unwrap().a[0];
        assert_eq!(edge.start, 1.0);
        assert!(edge.end > 1.0 && edge.end < 2.0, "{edge:?}");
        delete.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn deleting_a_pattern_takes_its_seams_and_tacks() {
        let (mut doc, _, b, _) = sewn_pair();
        let before = doc.clone();
        let mut delete = DeletePattern::new(b);
        delete.apply(&mut doc);
        assert!(doc.sewing_lines.is_empty());
        assert_eq!(doc.tacks.len(), 1, "the avatar tack stays");
        delete.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn tack_points_move_and_flip_with_their_pattern() {
        let (mut doc, a, b, _) = sewn_pair();
        let before = doc.clone();
        let mut mv = MovePattern::new(a, Vec2::new(5.0, -5.0));
        mv.apply(&mut doc);
        assert_eq!(doc.tacks[0].a.at, Vec2::new(25.0, 5.0));
        assert_eq!(doc.tacks[1].a.at, Vec2::new(55.0, 20.0));
        assert_eq!(doc.tacks[0].b, before.tacks[0].b, "the other end stays");
        mv.undo(&mut doc);
        assert_eq!(doc, before);

        // Other spans 150..250: flipping left-right about x = 200.
        let mut flip = MirrorPattern::new(b, Axis::Vertical);
        flip.apply(&mut doc);
        let TackTarget::Pattern(end) = doc.tacks[0].b else {
            panic!("pattern tack");
        };
        assert_eq!(end.at, Vec2::new(230.0, 10.0));
        flip.undo(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn flipping_and_editing_a_seam_side() {
        let (mut doc, a, _, id) = sewn_pair();
        let before = doc.clone();
        let mut flip = FlipSeamSide::new(id, Side::B);
        flip.apply(&mut doc);
        assert!(doc.sewing_line(id).unwrap().b[0].forward);
        flip.undo(&mut doc);
        assert_eq!(doc, before);

        let shorter = SeamEdge {
            pattern: a,
            start: 1.0,
            end: 1.5,
            forward: true,
        };
        let mut edit = SetSeamEdge::new(id, Side::A, 0, shorter);
        edit.apply(&mut doc);
        let line = doc.sewing_line(id).unwrap();
        assert_eq!(line.a[0], shorter);
        assert_eq!(line.side_lengths(&doc), Some((25.0, 50.0)));
        assert_eq!(edit.label(), "Edit sewing");
        edit.undo(&mut doc);
        assert_eq!(doc, before);
    }
}
