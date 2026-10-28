//! Copying patterns, with the seams and tacks between them, and pasting
//! them back as new patterns.

use std::collections::HashMap;

use crate::command::Command;
use crate::document::{Axis, Document, Pattern, PatternId, SewingLine, Tack, TackTarget};
use crate::geometry::{self, Vec2};

/// Copied patterns, plus the seams and tacks that only join copied
/// patterns to each other. Seams or tacks to anything else are left out,
/// as are tacks to the avatar: a pasted copy sits elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    patterns: Vec<Pattern>,
    sewing_lines: Vec<SewingLine>,
    tacks: Vec<Tack>,
}

impl Clip {
    /// Copy `ids` from `doc`, in document order. `None` if none exist.
    pub fn copy(doc: &Document, ids: &[PatternId]) -> Option<Self> {
        let patterns: Vec<Pattern> = doc
            .patterns
            .iter()
            .filter(|p| ids.contains(&p.id))
            .cloned()
            .collect();
        if patterns.is_empty() {
            return None;
        }
        let copied = |id: PatternId| patterns.iter().any(|p| p.id == id);
        let sewing_lines = doc
            .sewing_lines
            .iter()
            .filter(|line| line.a.iter().chain(&line.b).all(|e| copied(e.pattern)))
            .cloned()
            .collect();
        let tacks = doc
            .tacks
            .iter()
            .filter(|t| match t.b {
                TackTarget::Pattern(b) => copied(t.a.pattern) && copied(b.pattern),
                _ => false,
            })
            .cloned()
            .collect();
        Some(Self {
            patterns,
            sewing_lines,
            tacks,
        })
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Bounds of every copied outline, in mm.
    fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let points: Vec<Vec2> = self
            .patterns
            .iter()
            .filter_map(|p| p.bounds())
            .flat_map(|(min, max)| [min, max])
            .collect();
        geometry::bounds(&points)
    }
}

/// Add a [`Clip`]'s patterns as new ones, moved by `offset` or, if
/// `mirror` is set, reflected about the clip's right (or bottom) edge like
/// the other half of a fold.
pub struct Paste {
    clip: Clip,
    offset: Vec2,
    mirror: Option<Axis>,
    /// What was added, made on first apply so redo restores the same ids.
    added: Option<Clip>,
}

impl Paste {
    pub fn new(clip: Clip, offset: Vec2, mirror: Option<Axis>) -> Self {
        Self {
            clip,
            offset,
            mirror,
            added: None,
        }
    }

    /// Ids of the pasted patterns, available after the command has run.
    pub fn ids(&self) -> Vec<PatternId> {
        self.added
            .iter()
            .flat_map(|c| c.patterns.iter().map(|p| p.id))
            .collect()
    }

    /// The clip with fresh ids, placed where it is pasted.
    fn place(&self, doc: &mut Document) -> Clip {
        let mut clip = self.clip.clone();
        // Mirrored: about the clip's right (or bottom) edge.
        let mirror = self.mirror.map(|axis| {
            let (_, max) = clip.bounds().unwrap_or_default();
            (axis, if axis == Axis::Vertical { max.x } else { max.y })
        });
        let offset = self.offset;
        let place = |p: Vec2| match mirror {
            Some((axis, about)) => axis.reflect(p, about),
            None => p + offset,
        };
        let mut ids = HashMap::new();
        for p in &mut clip.patterns {
            let id = doc.allocate_id();
            ids.insert(p.id, id);
            p.id = id;
            match mirror {
                Some((axis, about)) => p.mirror(axis, about),
                None => p.translate(offset),
            }
        }
        for line in &mut clip.sewing_lines {
            line.id = doc.allocate_sewing_id();
            for edge in line.a.iter_mut().chain(&mut line.b) {
                edge.pattern = ids[&edge.pattern];
            }
        }
        for tack in &mut clip.tacks {
            tack.id = doc.allocate_tack_id();
            for point in tack.points_mut() {
                point.pattern = ids[&point.pattern];
                point.at = place(point.at);
            }
        }
        clip
    }
}

impl Command for Paste {
    fn label(&self) -> String {
        let what = match self.clip.len() {
            1 => "pattern".to_owned(),
            n => format!("{n} patterns"),
        };
        match self.mirror {
            Some(_) => format!("Paste {what} mirrored"),
            None => format!("Paste {what}"),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if self.added.is_none() {
            self.added = Some(self.place(doc));
        }
        if let Some(added) = &self.added {
            doc.patterns.extend(added.patterns.iter().cloned());
            doc.sewing_lines.extend(added.sewing_lines.iter().cloned());
            doc.tacks.extend(added.tacks.iter().cloned());
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(added) = &self.added {
            doc.patterns
                .retain(|p| !added.patterns.iter().any(|a| a.id == p.id));
            doc.sewing_lines
                .retain(|l| !added.sewing_lines.iter().any(|a| a.id == l.id));
            doc.tacks
                .retain(|t| !added.tacks.iter().any(|a| a.id == t.id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, AddSewingLine, AddTack};
    use crate::document::{SeamEdge, TackPoint};
    use crate::geometry::rectangle;

    /// Three rectangles: the first two sewn and tacked together, the
    /// third sewn to the first.
    fn sewn() -> (Document, [PatternId; 3]) {
        let mut doc = Document::default();
        let mut ids = [PatternId(0); 3];
        for (i, id) in ids.iter_mut().enumerate() {
            let x = i as f32 * 150.0;
            let mut add = AddPattern::new(
                format!("P{i}"),
                rectangle(Vec2::new(x, 0.0), Vec2::new(x + 100.0, 80.0)),
            );
            add.apply(&mut doc);
            *id = add.id().unwrap();
        }
        let edge = |pattern, start: f32| SeamEdge {
            pattern,
            start,
            end: start + 1.0,
            forward: true,
        };
        AddSewingLine::new(vec![edge(ids[0], 1.0)], vec![edge(ids[1], 3.0)]).apply(&mut doc);
        AddSewingLine::new(vec![edge(ids[0], 3.0)], vec![edge(ids[2], 1.0)]).apply(&mut doc);
        let at = |pattern, x| TackPoint {
            pattern,
            at: Vec2::new(x, 40.0),
        };
        AddTack::new(at(ids[0], 50.0), TackTarget::Pattern(at(ids[1], 200.0))).apply(&mut doc);
        (doc, ids)
    }

    #[test]
    fn copies_keep_only_what_joins_copied_patterns() {
        let (doc, ids) = sewn();
        let clip = Clip::copy(&doc, &[ids[0], ids[1]]).unwrap();
        assert_eq!(clip.len(), 2);
        assert_eq!(clip.sewing_lines.len(), 1, "the seam to P2 is left out");
        assert_eq!(clip.tacks.len(), 1);
        assert!(Clip::copy(&doc, &[PatternId(99)]).is_none());
    }

    #[test]
    fn pasting_adds_new_patterns_joined_like_the_originals() {
        let (mut doc, ids) = sewn();
        let before = doc.clone();
        let clip = Clip::copy(&doc, &[ids[0], ids[1]]).unwrap();
        let mut paste = Paste::new(clip, Vec2::new(0.0, 100.0), None);
        paste.apply(&mut doc);
        assert_eq!(paste.label(), "Paste 2 patterns");
        let new = paste.ids();
        assert_eq!(new.len(), 2);
        assert!(new.iter().all(|id| !ids.contains(id)));
        assert_eq!(doc.patterns.len(), 5);
        let pasted = doc.pattern(new[0]).unwrap();
        assert_eq!(pasted.outline[0].pos, Vec2::new(0.0, 100.0));
        // The pasted seam joins the two copies, at the same params.
        let seam = doc.sewing_lines.last().unwrap();
        assert_eq!((seam.a[0].pattern, seam.b[0].pattern), (new[0], new[1]));
        assert_eq!(seam.b[0].start, 3.0);
        let tack = doc.tacks.last().unwrap();
        assert_eq!(tack.a.pattern, new[0]);
        assert_eq!(tack.a.at, Vec2::new(50.0, 140.0));

        paste.undo(&mut doc);
        assert_eq!(doc.patterns, before.patterns);
        assert_eq!(doc.sewing_lines, before.sewing_lines);
        assert_eq!(doc.tacks, before.tacks);
        paste.apply(&mut doc);
        assert_eq!(paste.ids(), new, "redo keeps the same ids");
    }

    #[test]
    fn a_mirrored_paste_lands_beside_the_copy() {
        let (mut doc, ids) = sewn();
        let clip = Clip::copy(&doc, &[ids[0]]).unwrap();
        let mut paste = Paste::new(clip, Vec2::ZERO, Some(Axis::Vertical));
        paste.apply(&mut doc);
        assert_eq!(paste.label(), "Paste pattern mirrored");
        let (min, max) = doc.pattern(paste.ids()[0]).unwrap().bounds().unwrap();
        assert_eq!((min.x, max.x), (100.0, 200.0));
    }
}
