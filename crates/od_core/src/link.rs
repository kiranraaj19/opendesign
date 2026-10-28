//! Symmetric editing: linked copies (see [`Link`]) kept up to date with
//! their source after every command, and the commands that make and
//! break the link.

use crate::command::{remap_sewing, Command};
use crate::document::{
    Axis, Document, Link, OutlinePoint, Pattern, PatternId, SewingLine, TOLERANCE,
};
use crate::geometry::Vec2;

/// Bring every linked copy back in line with its source: outline, internal
/// lines, darts, topstitches, trims, notches, grainline and grading moved
/// by the link's transform, and the same fabric and seam allowance. A
/// copy whose source is gone (or is itself a copy) stops
/// being linked. Seams on a copy whose point count changed are placed
/// again by position.
pub fn sync(doc: &mut Document) {
    for i in 0..doc.patterns.len() {
        let Some(link) = doc.patterns[i].link else {
            continue;
        };
        let id = doc.patterns[i].id;
        let source = doc
            .pattern(link.source)
            .filter(|s| s.id != id && s.link.is_none())
            .cloned();
        let Some(source) = source else {
            doc.patterns[i].link = None;
            continue;
        };
        let before = doc.patterns[i].clone();
        let copy = &mut doc.patterns[i];
        let f = |p: Vec2| link.apply(p);
        copy.outline = source
            .outline
            .iter()
            .map(|p| OutlinePoint {
                pos: f(p.pos),
                kind: p.kind,
            })
            .collect();
        copy.internal_lines = source.internal_lines.clone();
        copy.darts = source.darts.clone();
        copy.topstitches = source.topstitches.clone();
        copy.trims = source.trims.clone();
        copy.notches = source.notches.clone();
        copy.fabric = source.fabric.clone();
        copy.seam_allowance = source.seam_allowance;
        copy.grain_angle = link.grain_from_source(source.grain_angle);
        copy.grading.point_deltas = source
            .grading
            .point_deltas
            .iter()
            .zip(&source.outline)
            .map(|(d, p)| f(p.pos + *d) - f(p.pos))
            .collect();
        for line in &mut copy.internal_lines {
            line.points.iter_mut().for_each(|p| *p = f(*p));
        }
        for line in &mut copy.topstitches {
            line.points.iter_mut().for_each(|p| *p = f(*p));
        }
        for trim in &mut copy.trims {
            trim.pos = f(trim.pos);
        }
        for dart in &mut copy.darts {
            dart.start = f(dart.start);
            dart.end = f(dart.end);
        }
        if copy.outline.len() != before.outline.len() {
            remap_sewing(doc, &before, &|b, a, t| {
                a.nearest_param(b.pos_at_param(t, TOLERANCE)).0
            });
        }
    }
}

/// Runs a command, then [`sync`]; undoes it exactly, linked copies and
/// seams included. [`crate::History`] wraps every command in one.
pub(crate) struct Linked {
    inner: Box<dyn Command>,
    before: Option<(Vec<Pattern>, Vec<SewingLine>)>,
}

impl Linked {
    pub(crate) fn new(inner: Box<dyn Command>) -> Self {
        Self {
            inner,
            before: None,
        }
    }
}

impl Command for Linked {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn apply(&mut self, doc: &mut Document) {
        let copies = doc
            .patterns
            .iter()
            .filter(|p| p.link.is_some())
            .cloned()
            .collect();
        self.before = Some((copies, doc.sewing_lines.clone()));
        self.inner.apply(doc);
        sync(doc);
    }

    fn undo(&mut self, doc: &mut Document) {
        self.inner.undo(doc);
        if let Some((copies, seams)) = self.before.take() {
            for copy in copies {
                if let Some(p) = doc.pattern_mut(copy.id) {
                    *p = copy;
                }
            }
            doc.sewing_lines = seams;
        }
    }
}

/// Add a linked copy of a pattern: mirrored beside it like the other half
/// of a fold, or just offset. Editing either one changes both.
pub struct LinkedCopy {
    source: PatternId,
    mirror: Option<Axis>,
    copy: Option<Pattern>,
}

/// How far an unmirrored linked copy is offset from its source, in mm.
const COPY_OFFSET: f32 = 40.0;

impl LinkedCopy {
    pub fn new(source: PatternId, mirror: Option<Axis>) -> Self {
        Self {
            source,
            mirror,
            copy: None,
        }
    }
}

impl Command for LinkedCopy {
    fn label(&self) -> String {
        "Linked copy".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if self.copy.is_none() {
            // A copy of a copy links to the original instead.
            let Some(source) = doc.pattern(self.source) else {
                return;
            };
            let source = match source.link {
                Some(link) => match doc.pattern(link.source) {
                    Some(original) => original,
                    None => return,
                },
                None => source,
            };
            let mut copy = source.clone();
            copy.link = Some(Link::identity(source.id));
            copy.arrangement = None;
            let (_, max) = source.bounds().unwrap_or_default();
            match self.mirror {
                Some(axis) => {
                    copy.mirror(axis, if axis == Axis::Vertical { max.x } else { max.y });
                    copy.name = format!("{} (linked, mirrored)", source.name);
                }
                None => {
                    copy.translate(Vec2::new(COPY_OFFSET, COPY_OFFSET));
                    copy.name = format!("{} (linked)", source.name);
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
        if let Some(copy) = &self.copy {
            doc.patterns.retain(|p| p.id != copy.id);
        }
    }
}

/// Stop a linked copy following its source; it keeps its current shape.
pub struct Unlink {
    id: PatternId,
    /// Swapped with the pattern's link on apply and undo.
    link: Option<Link>,
}

impl Unlink {
    pub fn new(id: PatternId) -> Self {
        Self { id, link: None }
    }
}

impl Command for Unlink {
    fn label(&self) -> String {
        "Unlink copy".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.link, &mut self.link);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, AddSewingLine, InsertPoint, MovePattern, MovePoint};
    use crate::document::SeamEdge;
    use crate::geometry::rectangle;
    use crate::History;

    fn setup() -> (Document, History, PatternId, PatternId) {
        let (mut doc, mut history) = (Document::default(), History::new());
        history.execute(
            &mut doc,
            Box::new(AddPattern::new(
                "Front",
                rectangle(Vec2::ZERO, Vec2::new(100.0, 200.0)),
            )),
        );
        let source = doc.patterns[0].id;
        history.execute(
            &mut doc,
            Box::new(LinkedCopy::new(source, Some(Axis::Vertical))),
        );
        let copy = doc.patterns[1].id;
        (doc, history, source, copy)
    }

    #[test]
    fn a_mirrored_linked_copy_follows_edits_to_its_source() {
        let (mut doc, mut history, source, copy) = setup();
        let c = doc.pattern(copy).unwrap();
        // Mirrored about the source's right edge, x = 100.
        assert_eq!(c.outline[0].pos, Vec2::new(200.0, 0.0));
        assert!(c.link.unwrap().flips());

        // Move a point of the source: the copy's mirror image moves too.
        history.execute(
            &mut doc,
            Box::new(MovePoint::new(source, 2, Vec2::new(120.0, 220.0))),
        );
        assert_eq!(
            doc.pattern(copy).unwrap().outline[2].pos,
            Vec2::new(80.0, 220.0)
        );
        // Moving the copy keeps it linked, in its new place.
        history.execute(
            &mut doc,
            Box::new(MovePattern::new(copy, Vec2::new(50.0, 0.0))),
        );
        history.execute(
            &mut doc,
            Box::new(MovePoint::new(source, 0, Vec2::new(-10.0, 0.0))),
        );
        let c = doc.pattern(copy).unwrap();
        assert_eq!(c.outline[0].pos, Vec2::new(260.0, 0.0));
        assert_eq!(c.outline[2].pos, Vec2::new(130.0, 220.0));
        // Undo steps back through all of it.
        history.undo(&mut doc);
        history.undo(&mut doc);
        history.undo(&mut doc);
        assert_eq!(
            doc.pattern(copy).unwrap().outline[2].pos,
            Vec2::new(100.0, 200.0)
        );
    }

    #[test]
    fn seams_on_a_copy_stay_put_when_the_source_gains_a_point() {
        let (mut doc, mut history, source, copy) = setup();
        // The copy's bottom edge (index 2 → 3), sewn to itself reversed
        // just to have a seam on it.
        let edge = |start: f32, forward| SeamEdge {
            pattern: copy,
            start,
            end: start + 1.0,
            forward,
        };
        history.execute(
            &mut doc,
            Box::new(AddSewingLine::new(
                vec![edge(2.0, true)],
                vec![edge(0.0, false)],
            )),
        );
        let length = |doc: &Document| doc.sewing_lines[0].side_lengths(doc).unwrap();
        let before = length(&doc);
        // A point on the source's top edge (before index 1) renumbers the
        // copy's points after it.
        history.execute(
            &mut doc,
            Box::new(InsertPoint::new(
                source,
                1,
                OutlinePoint::corner(Vec2::new(50.0, 0.0)),
            )),
        );
        assert_eq!(doc.pattern(copy).unwrap().outline.len(), 5);
        let e = doc.sewing_lines[0].a[0];
        assert!((e.start - 3.0).abs() < 1e-3, "{e:?}");
        assert_eq!(length(&doc).0, before.0);
        let snapshot = doc.clone();
        history.undo(&mut doc);
        assert_eq!(doc.pattern(copy).unwrap().outline.len(), 4);
        assert_eq!(doc.sewing_lines[0].a[0].start, 2.0);
        history.redo(&mut doc);
        assert_eq!(doc, snapshot);
    }

    #[test]
    fn a_linked_copy_follows_fabric_grain_allowance_and_grading() {
        use crate::command::{PatternValue, SetFabric, SetGrading, SetPatternValue};
        use crate::document::{Fabric, Grading};
        let (mut doc, mut history, source, copy) = setup();
        history.execute(
            &mut doc,
            Box::new(SetPatternValue::new(source, PatternValue::GrainAngle, 20.0)),
        );
        history.execute(
            &mut doc,
            Box::new(SetPatternValue::new(
                source,
                PatternValue::SeamAllowance,
                15.0,
            )),
        );
        history.execute(&mut doc, Box::new(SetFabric::new(source, Fabric::denim())));
        let outline = doc.pattern(source).unwrap().outline.clone();
        history.execute(
            &mut doc,
            Box::new(SetGrading::new(
                source,
                Grading::uniform_scale(&outline, 10.0),
            )),
        );
        let c = doc.pattern(copy).unwrap();
        // Mirrored left to right, so the grainline leans the other way.
        assert!((c.grain_angle + 20.0).abs() < 1e-3, "{}", c.grain_angle);
        assert_eq!(c.seam_allowance, 15.0);
        assert_eq!(c.fabric, Fabric::denim());
        // The copy grades as the mirror image of its source.
        let mut mirrored = doc.pattern(source).unwrap().graded(1);
        mirrored.mirror(Axis::Vertical, 100.0);
        for (x, y) in c.graded_outline(1).iter().zip(&mirrored.outline) {
            assert!(x.pos.distance(y.pos) < 1e-3, "{x:?} {y:?}");
        }
        // And the grain maps back.
        let link = c.link.unwrap();
        assert!((link.grain_to_source(c.grain_angle) - 20.0).abs() < 1e-3);
    }

    #[test]
    fn unlinking_keeps_the_shape_and_stops_following() {
        let (mut doc, mut history, source, copy) = setup();
        history.execute(&mut doc, Box::new(Unlink::new(copy)));
        let shape = doc.pattern(copy).unwrap().outline.clone();
        history.execute(
            &mut doc,
            Box::new(MovePoint::new(source, 2, Vec2::new(150.0, 250.0))),
        );
        assert_eq!(doc.pattern(copy).unwrap().outline, shape);
        // Deleting a source unlinks its copies too.
        let (mut doc, mut history, source, copy) = setup();
        history.execute(&mut doc, Box::new(crate::DeletePattern::new(source)));
        assert!(doc.pattern(copy).unwrap().link.is_none());
        history.undo(&mut doc);
        assert!(doc.pattern(copy).unwrap().link.is_some());
    }

    #[test]
    fn link_transforms_map_back_and_forth() {
        let mut p = Pattern::new(PatternId(1), "P", Vec::new());
        p.link = Some(Link::identity(PatternId(2)));
        p.rotate(30.0, Vec2::new(5.0, 5.0));
        p.mirror(Axis::Horizontal, 3.0);
        p.translate(Vec2::new(7.0, -2.0));
        let link = p.link.unwrap();
        let q = Vec2::new(12.0, -4.0);
        assert!(link.to_source(link.apply(q)).distance(q) < 1e-4);
    }
}
