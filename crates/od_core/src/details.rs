//! Garment details: things sewn onto a piece's edges or set on a seam
//! rather than drawn as lines of their own. A zipper is a seam with a
//! tape either side that can be opened ([`SeamDetails::zipper`]).
//!
//! Seam details live on the [`SewingLine`] itself, so every command that
//! moves, re-places, splits or copies seams carries them along.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::document::{Document, SewingLine, SewingLineId};

/// What a seam is besides two sides sewn together. The default is a
/// plain seam.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SeamDetails {
    /// The seam is a zipper: its sides are taped, and held together only
    /// while it is closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zipper: Option<Zipper>,
}

impl SeamDetails {
    /// A plain seam, with nothing to save.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// A zipper along a seam: each side is a stiff tape with teeth, and the
/// slider runs from where the seam starts sewing (the bottom stop) to
/// where it ends.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Zipper {
    /// Zipped up: the two sides are held together when draping. Open,
    /// they hang apart, each still stiffened by its tape.
    pub closed: bool,
    /// How wide each side's tape is, in mm, teeth included.
    pub tape_width: f32,
}

impl Zipper {
    pub const DEFAULT_TAPE_WIDTH: f32 = 12.0;
    /// The tape widths offered, in mm.
    pub const TAPE_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 4.0..=40.0;
}

impl Default for Zipper {
    fn default() -> Self {
        Self {
            closed: true,
            tape_width: Self::DEFAULT_TAPE_WIDTH,
        }
    }
}

impl SewingLine {
    /// Is this seam a zipper?
    pub fn is_zipper(&self) -> bool {
        self.details.zipper.is_some()
    }

    /// Are the two sides held together when draping? Every seam is,
    /// except an open zipper.
    pub fn holds_closed(&self) -> bool {
        self.details.zipper.is_none_or(|z| z.closed)
    }
}

/// Make a seam a zipper, change its zipper (open or close it, change its
/// tape), or make it a plain seam again (`None`).
pub struct SetZipper {
    id: SewingLineId,
    zipper: Option<Zipper>,
    /// The zipper before, restored on undo.
    before: Option<Option<Zipper>>,
}

impl SetZipper {
    pub fn new(id: SewingLineId, zipper: Option<Zipper>) -> Self {
        Self {
            id,
            zipper,
            before: None,
        }
    }
}

impl Command for SetZipper {
    fn label(&self) -> String {
        match (self.before.flatten(), self.zipper) {
            (None, Some(_)) => "Make zipper",
            (Some(_), None) => "Remove zipper",
            (Some(old), Some(new)) if old.closed && !new.closed => "Open zipper",
            (Some(old), Some(new)) if !old.closed && new.closed => "Close zipper",
            _ => "Edit zipper",
        }
        .into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(line) = doc.sewing_line_mut(self.id) {
            self.before = Some(line.details.zipper);
            line.details.zipper = self.zipper;
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(line), Some(before)) = (doc.sewing_line_mut(self.id), self.before) {
            line.details.zipper = before;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{OutlinePoint, Pattern, PatternId, SeamEdge};
    use crate::geometry::{self, Vec2};
    use crate::{AddSewingLine, History};

    /// Two 100 × 300 panels, their facing long edges ready to sew.
    fn doc() -> (Document, [PatternId; 2]) {
        let mut doc = Document::new("Test");
        let ids = [doc.allocate_id(), doc.allocate_id()];
        for (k, id) in ids.into_iter().enumerate() {
            let x = k as f32 * 150.0;
            doc.patterns.push(Pattern::new(
                id,
                "Panel",
                geometry::rectangle(Vec2::new(x, 0.0), Vec2::new(x + 100.0, 300.0))
                    .into_iter()
                    .map(OutlinePoint::corner)
                    .collect(),
            ));
        }
        (doc, ids)
    }

    fn run(history: &mut History, doc: &mut Document, command: impl Command + 'static) {
        history.execute(doc, Box::new(command));
    }

    fn edge(pattern: PatternId, start: f32, forward: bool) -> SeamEdge {
        SeamEdge {
            pattern,
            start,
            end: start + 1.0,
            forward,
        }
    }

    #[test]
    fn a_zipper_is_made_opened_closed_and_removed_with_undo() {
        let (mut doc, [a, b]) = doc();
        let mut history = History::new();
        run(
            &mut history,
            &mut doc,
            AddSewingLine::new(vec![edge(a, 1.0, true)], vec![edge(b, 3.0, false)]),
        );
        let id = doc.sewing_lines[0].id;
        assert!(!doc.sewing_lines[0].is_zipper());
        let plain = doc.clone();

        run(
            &mut history,
            &mut doc,
            SetZipper::new(id, Some(Zipper::default())),
        );
        assert_eq!(history.undo_label(), Some("Make zipper".into()));
        assert!(doc.sewing_lines[0].is_zipper() && doc.sewing_lines[0].holds_closed());

        let open = Zipper {
            closed: false,
            ..Zipper::default()
        };
        run(&mut history, &mut doc, SetZipper::new(id, Some(open)));
        assert_eq!(history.undo_label(), Some("Open zipper".into()));
        assert!(!doc.sewing_lines[0].holds_closed());
        let opened = doc.clone();

        run(&mut history, &mut doc, SetZipper::new(id, None));
        assert_eq!(history.undo_label(), Some("Remove zipper".into()));
        assert_eq!(doc, plain);
        history.undo(&mut doc);
        assert_eq!(doc, opened);
        history.undo(&mut doc);
        history.undo(&mut doc);
        assert_eq!(doc, plain);
    }

    #[test]
    fn a_zipped_seam_is_added_as_one_step() {
        let (mut doc, [a, b]) = doc();
        let mut history = History::new();
        run(
            &mut history,
            &mut doc,
            AddSewingLine::new(vec![edge(a, 1.0, true)], vec![edge(b, 3.0, false)])
                .zipped(Zipper::default()),
        );
        assert_eq!(history.undo_label(), Some("Add zipper".into()));
        assert_eq!(doc.sewing_lines[0].details.zipper, Some(Zipper::default()));
        history.undo(&mut doc);
        assert!(doc.sewing_lines.is_empty());
    }

    #[test]
    fn zippers_are_saved_and_plain_seams_save_as_before() {
        let (mut doc, [a, b]) = doc();
        AddSewingLine::new(vec![edge(a, 1.0, true)], vec![edge(b, 3.0, false)]).apply(&mut doc);
        let json = crate::project::to_json(&doc);
        assert!(!json.contains("details"), "a plain seam writes no details");
        let id = doc.sewing_lines[0].id;
        SetZipper::new(
            id,
            Some(Zipper {
                closed: false,
                tape_width: 8.0,
            }),
        )
        .apply(&mut doc);
        let loaded = crate::project::from_json(&crate::project::to_json(&doc)).unwrap();
        assert_eq!(loaded, doc);
    }
}
