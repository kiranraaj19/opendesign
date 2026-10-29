//! Lining pieces up in the 2D layout: align their edges or centres, or
//! spread them out with equal gaps between them.

use crate::command::{Batch, Command, MovePattern};
use crate::document::{Document, PatternId};
use crate::geometry::Vec2;

/// How to line up several pieces. Left, right, top and bottom are as the
/// 2D window shows them (y grows downwards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    /// Centres on one vertical line.
    Centre,
    Right,
    Top,
    /// Centres on one horizontal line.
    Middle,
    Bottom,
    /// Equal gaps from left to right, the outermost two staying put.
    DistributeAcross,
    /// Equal gaps from top to bottom, the outermost two staying put.
    DistributeDown,
}

impl Align {
    pub const EDGES: [Align; 6] = [
        Align::Left,
        Align::Centre,
        Align::Right,
        Align::Top,
        Align::Middle,
        Align::Bottom,
    ];
    pub const DISTRIBUTE: [Align; 2] = [Align::DistributeAcross, Align::DistributeDown];

    /// Button text, which is also the undo step's name.
    pub fn label(self) -> &'static str {
        match self {
            Align::Left => "Align left",
            Align::Centre => "Align centre",
            Align::Right => "Align right",
            Align::Top => "Align top",
            Align::Middle => "Align middle",
            Align::Bottom => "Align bottom",
            Align::DistributeAcross => "Distribute across",
            Align::DistributeDown => "Distribute down",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Align::Left => "Line up the left edges",
            Align::Centre => "Line up the centres on one vertical line",
            Align::Right => "Line up the right edges",
            Align::Top => "Line up the top edges",
            Align::Middle => "Line up the centres on one horizontal line",
            Align::Bottom => "Line up the bottom edges",
            Align::DistributeAcross => {
                "Equal gaps from left to right; the outermost pieces stay put (3 or more)"
            }
            Align::DistributeDown => {
                "Equal gaps from top to bottom; the outermost pieces stay put (3 or more)"
            }
        }
    }

    /// Spreading out needs a piece between the two outer ones.
    pub fn min_pieces(self) -> usize {
        match self {
            Align::DistributeAcross | Align::DistributeDown => 3,
            _ => 2,
        }
    }
}

/// How far each of `ids` should move to line up as `how` says, measured
/// on the pieces' outline bounds. All of `ids` set the reference (the
/// bounds of them together), but locked pieces don't move. Pieces that
/// wouldn't move are left out.
pub fn moves(doc: &Document, ids: &[PatternId], how: Align) -> Vec<(PatternId, Vec2)> {
    let boxes: Vec<(PatternId, Vec2, Vec2, bool)> = ids
        .iter()
        .filter_map(|&id| doc.pattern(id))
        .filter_map(|p| p.bounds().map(|(min, max)| (p.id, min, max, p.locked)))
        .collect();
    if boxes.len() < how.min_pieces() {
        return Vec::new();
    }
    let min = boxes.iter().fold(Vec2::new(f32::MAX, f32::MAX), |m, b| {
        Vec2::new(m.x.min(b.1.x), m.y.min(b.1.y))
    });
    let max = boxes.iter().fold(Vec2::new(f32::MIN, f32::MIN), |m, b| {
        Vec2::new(m.x.max(b.2.x), m.y.max(b.2.y))
    });
    let centre = (min + max) * 0.5;
    let mut out: Vec<(PatternId, Vec2)> = match how {
        Align::DistributeAcross | Align::DistributeDown => {
            let across = how == Align::DistributeAcross;
            // Along the chosen axis: (id, low end, size, locked).
            let along = |v: Vec2| if across { v.x } else { v.y };
            let mut spans: Vec<_> = boxes
                .iter()
                .map(|&(id, lo, hi, locked)| (id, along(lo), along(hi) - along(lo), locked))
                .collect();
            spans.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let total: f32 = spans.iter().map(|s| s.2).sum();
            let gap = (along(max) - along(min) - total) / (spans.len() - 1) as f32;
            let mut at = along(min);
            spans
                .iter()
                .map(|&(id, lo, size, _)| {
                    let d = at - lo;
                    at += size + gap;
                    let delta = if across {
                        Vec2::new(d, 0.0)
                    } else {
                        Vec2::new(0.0, d)
                    };
                    (id, delta)
                })
                .collect()
        }
        _ => boxes
            .iter()
            .map(|&(id, lo, hi, _)| {
                let delta = match how {
                    Align::Left => Vec2::new(min.x - lo.x, 0.0),
                    Align::Right => Vec2::new(max.x - hi.x, 0.0),
                    Align::Centre => Vec2::new(centre.x - (lo.x + hi.x) * 0.5, 0.0),
                    Align::Top => Vec2::new(0.0, min.y - lo.y),
                    Align::Bottom => Vec2::new(0.0, max.y - hi.y),
                    _ => Vec2::new(0.0, centre.y - (lo.y + hi.y) * 0.5),
                };
                (id, delta)
            })
            .collect(),
    };
    let locked = |id: PatternId| boxes.iter().any(|b| b.0 == id && b.3);
    out.retain(|&(id, d)| !locked(id) && d.length() > 1e-4);
    out
}

/// Line up `ids` as one undo step, or `None` if nothing would move.
///
/// A linked copy follows its source after every command (see
/// [`crate::link::sync`]), so when both move, the copy's own move is
/// less whatever its source's move carries it by.
pub fn align(doc: &Document, ids: &[PatternId], how: Align) -> Option<Batch> {
    let moves = moves(doc, ids, how);
    if moves.is_empty() {
        return None;
    }
    let delta_of = |id: PatternId| moves.iter().find(|m| m.0 == id).map(|m| m.1);
    let commands: Vec<Box<dyn Command>> = moves
        .iter()
        .map(|&(id, delta)| {
            let carried = doc
                .pattern(id)
                .and_then(|p| p.link)
                .and_then(|link| {
                    delta_of(link.source).map(|d| link.apply(d) - link.apply(Vec2::ZERO))
                })
                .unwrap_or(Vec2::ZERO);
            Box::new(MovePattern::new(id, delta - carried)) as Box<dyn Command>
        })
        .collect();
    Some(Batch::new(how.label(), commands))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, PatternFlag, SetPatternFlag};
    use crate::document::Axis;
    use crate::geometry::rectangle;
    use crate::{History, LinkedCopy};

    /// Three boxes of different sizes at different places.
    fn setup() -> (Document, History, Vec<PatternId>) {
        let (mut doc, mut history) = (Document::default(), History::new());
        for (x, y, w, h) in [
            (0.0, 0.0, 100.0, 50.0),
            (300.0, 80.0, 60.0, 120.0),
            (150.0, -40.0, 40.0, 40.0),
        ] {
            let outline = rectangle(Vec2::new(x, y), Vec2::new(x + w, y + h));
            history.execute(&mut doc, Box::new(AddPattern::new("Box", outline)));
        }
        let ids = doc.patterns.iter().map(|p| p.id).collect();
        (doc, history, ids)
    }

    fn bounds(doc: &Document, id: PatternId) -> (Vec2, Vec2) {
        doc.pattern(id).unwrap().bounds().unwrap()
    }

    fn run(doc: &mut Document, history: &mut History, ids: &[PatternId], how: Align) {
        let batch = align(doc, ids, how).expect("something moves");
        history.execute(doc, Box::new(batch));
    }

    #[test]
    fn edges_and_centres_line_up_as_one_undo_step() {
        let (mut doc, mut history, ids) = setup();
        let before = doc.clone();
        run(&mut doc, &mut history, &ids, Align::Left);
        assert!(ids.iter().all(|&id| bounds(&doc, id).0.x == 0.0));
        assert_eq!(history.undo_label().as_deref(), Some("Align left"));
        // Only x changes.
        assert_eq!(bounds(&doc, ids[1]).0.y, 80.0);
        history.undo(&mut doc);
        assert_eq!(doc, before, "undo is exact");

        run(&mut doc, &mut history, &ids, Align::Bottom);
        assert!(ids.iter().all(|&id| bounds(&doc, id).1.y == 200.0));
        history.undo(&mut doc);

        run(&mut doc, &mut history, &ids, Align::Centre);
        for &id in &ids {
            let (lo, hi) = bounds(&doc, id);
            assert!(((lo.x + hi.x) * 0.5 - 180.0).abs() < 1e-3);
        }
        history.undo(&mut doc);
        run(&mut doc, &mut history, &ids, Align::Middle);
        for &id in &ids {
            let (lo, hi) = bounds(&doc, id);
            assert!(((lo.y + hi.y) * 0.5 - 80.0).abs() < 1e-3);
        }
        // Already lined up: nothing to do.
        assert!(align(&doc, &ids, Align::Middle).is_none());
    }

    #[test]
    fn distributing_leaves_equal_gaps_between_the_outer_two() {
        let (mut doc, mut history, ids) = setup();
        assert!(align(&doc, &ids[..2], Align::DistributeAcross).is_none());
        run(&mut doc, &mut history, &ids, Align::DistributeAcross);
        // Widths 100 + 40 + 60 in a 360 span: two gaps of 80.
        assert_eq!(bounds(&doc, ids[0]).0.x, 0.0);
        assert_eq!(bounds(&doc, ids[1]).1.x, 360.0);
        assert!((bounds(&doc, ids[2]).0.x - 180.0).abs() < 1e-3);

        run(&mut doc, &mut history, &ids, Align::DistributeDown);
        // Heights 50 + 40 + 120 in a 240 span: gaps of 15. Top to bottom:
        // the small box (-40), the wide one, the tall one.
        assert_eq!(bounds(&doc, ids[2]).0.y, -40.0);
        assert!((bounds(&doc, ids[0]).0.y - 15.0).abs() < 1e-3);
        assert_eq!(bounds(&doc, ids[1]).1.y, 200.0);
    }

    #[test]
    fn locked_pieces_count_but_stay_put() {
        let (mut doc, mut history, ids) = setup();
        history.execute(
            &mut doc,
            Box::new(SetPatternFlag::new(ids[1], PatternFlag::Locked, true)),
        );
        run(&mut doc, &mut history, &ids, Align::Right);
        assert_eq!(bounds(&doc, ids[0]).1.x, 360.0);
        assert_eq!(bounds(&doc, ids[1]).0.x, 300.0);
    }

    #[test]
    fn a_linked_copy_and_its_source_both_land_in_line() {
        let (mut doc, mut history, ids) = setup();
        history.execute(
            &mut doc,
            Box::new(LinkedCopy::new(ids[0], Some(Axis::Vertical))),
        );
        let copy = doc.patterns[3].id;
        let mut all = ids.clone();
        all.push(copy);
        run(&mut doc, &mut history, &all, Align::Top);
        for &id in &all {
            assert!((bounds(&doc, id).0.y + 40.0).abs() < 1e-3, "{id:?}");
        }
        history.undo(&mut doc);
        // The copy is mirrored, so its source moving right carries it left.
        run(&mut doc, &mut history, &all, Align::Right);
        for &id in &all {
            assert!((bounds(&doc, id).1.x - 360.0).abs() < 1e-3, "{id:?}");
        }
        assert!(doc.pattern(copy).unwrap().link.is_some());
    }
}
