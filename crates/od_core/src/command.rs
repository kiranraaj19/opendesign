//! Commands are the only way to change a [`Document`]. Each one knows how to
//! apply itself and how to reverse that change.

use crate::document::{
    normalize_degrees, Axis, Document, InternalLine, Notch, OutlinePoint, Pattern, PatternId,
    PointKind, SeamEdge, SeamPoint, SewingLine, SewingLineId, Tack, TackId,
};
use crate::geometry::Vec2;

pub trait Command {
    /// Short label shown in the History panel and the Edit menu.
    fn label(&self) -> String;
    fn apply(&mut self, doc: &mut Document);
    fn undo(&mut self, doc: &mut Document);
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

pub struct DeletePattern {
    id: PatternId,
    /// The removed pattern and its position, kept for undo.
    removed: Option<(usize, Pattern)>,
}

impl DeletePattern {
    pub fn new(id: PatternId) -> Self {
        Self { id, removed: None }
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
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((index, pattern)) = self.removed.take() {
            doc.patterns.insert(index.min(doc.patterns.len()), pattern);
        }
    }
}

pub struct MovePattern {
    id: PatternId,
    delta: Vec2,
    /// The pattern before the move, restored on undo so that undo is exact
    /// (adding and subtracting a float delta can be off in the last bit).
    before: Option<Pattern>,
}

impl MovePattern {
    pub fn new(id: PatternId, delta: Vec2) -> Self {
        Self {
            id,
            delta,
            before: None,
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
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(before)) = (doc.pattern_mut(self.id), self.before.take()) {
            *p = before;
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
}

impl InsertPoint {
    pub fn new(id: PatternId, index: usize, point: OutlinePoint) -> Self {
        Self { id, index, point }
    }
}

impl Command for InsertPoint {
    fn label(&self) -> String {
        "Insert point".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            let index = self.index.min(p.outline.len());
            p.outline.insert(index, self.point);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if self.index < p.outline.len() {
                p.outline.remove(self.index);
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
}

impl DeletePoint {
    pub fn new(id: PatternId, index: usize) -> Self {
        Self {
            id,
            index,
            removed: None,
        }
    }
}

impl Command for DeletePoint {
    fn label(&self) -> String {
        "Delete point".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            if p.outline.len() > 3 && self.index < p.outline.len() {
                self.removed = Some(p.outline.remove(self.index));
            }
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(p), Some(point)) = (doc.pattern_mut(self.id), self.removed.take()) {
            p.outline.insert(self.index.min(p.outline.len()), point);
        }
    }
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

/// Tag a pattern with a fabric from the Library panel, by name; `None`
/// clears it back to the default fill.
pub struct SetFabric {
    id: PatternId,
    /// The fabric to assign, kept as-is (not swapped) so `label` always
    /// describes the intended change, even once `apply` has run.
    name: Option<String>,
    /// The pattern's fabric before this command, filled in by the first
    /// `apply` so `undo` can restore it.
    previous: Option<Option<String>>,
}

impl SetFabric {
    pub fn new(id: PatternId, name: Option<String>) -> Self {
        Self {
            id,
            name,
            previous: None,
        }
    }
}

impl Command for SetFabric {
    fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("Set fabric to {name}"),
            None => "Clear fabric".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            self.previous = Some(p.fabric.clone());
            p.fabric = self.name.clone();
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(previous) = self.previous.take() {
            if let Some(p) = doc.pattern_mut(self.id) {
                p.fabric = previous;
            }
        }
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
            line: InternalLine { points },
        }
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

/// Flip a pattern in place about its own centre line.
pub struct MirrorPattern {
    id: PatternId,
    axis: Axis,
    /// The pattern before mirroring, restored exactly on undo.
    before: Option<Pattern>,
}

impl MirrorPattern {
    pub fn new(id: PatternId, axis: Axis) -> Self {
        Self {
            id,
            axis,
            before: None,
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
            p.mirror(axis, if axis == Axis::Vertical { c.x } else { c.y });
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            if let Some(p) = doc.pattern_mut(self.id) {
                *p = before;
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
    /// Assigned on first apply so redo recreates the same id.
    id: Option<SewingLineId>,
}

impl AddSewingLine {
    pub fn new(a: Vec<SeamEdge>, b: Vec<SeamEdge>) -> Self {
        Self { a, b, id: None }
    }

    /// Id of the created seam, available after the command has run.
    pub fn id(&self) -> Option<SewingLineId> {
        self.id
    }
}

impl Command for AddSewingLine {
    fn label(&self) -> String {
        "Sew edges".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let id = *self.id.get_or_insert_with(|| doc.allocate_sewing_id());
        doc.sewing_lines.push(SewingLine {
            id,
            a: self.a.clone(),
            b: self.b.clone(),
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

/// Pin two outline points together with a tack.
pub struct AddTack {
    a: SeamPoint,
    b: SeamPoint,
    /// Assigned on first apply so redo recreates the same id.
    id: Option<TackId>,
}

impl AddTack {
    pub fn new(a: SeamPoint, b: SeamPoint) -> Self {
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
        MovePattern::new(id, Vec2::new(10.0, 0.0)).apply(&mut doc);
        assert_eq!(
            doc.pattern(id).unwrap().internal_lines[0].points[0],
            Vec2::new(10.0, 0.0)
        );
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
        }];
        let b = vec![
            SeamEdge {
                pattern: id2,
                start: 2.0,
                end: 3.0,
            },
            SeamEdge {
                pattern: id2,
                start: 3.0,
                end: 4.0,
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

        let a = SeamPoint {
            pattern: id,
            index: 0,
        };
        let b = SeamPoint {
            pattern: id,
            index: 2,
        };
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
}
