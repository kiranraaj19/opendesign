//! Garment details: things sewn onto a piece's edges or set on a seam
//! rather than drawn as lines of their own. A zipper is a seam with a
//! tape either side that can be opened ([`SeamDetails::zipper`]). A
//! buttonhole is a [`Trim`] with a slit, and a button is fastened into
//! one by a tack between their points ([`Fastening`]).
//!
//! Seam details live on the [`SewingLine`] itself, so every command that
//! moves, re-places, splits or copies seams carries them along.

use serde::{Deserialize, Serialize};

use crate::command::{AddTack, Command, RemoveTack};
use crate::document::{
    normalize_degrees, Document, PatternId, SewingLine, SewingLineId, TackId, TackPoint,
    TackTarget, Trim, TrimKind,
};
use crate::geometry::Vec2;

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

impl Trim {
    /// How long a buttonhole's slit is, in mm: room for a 15 mm button.
    pub const BUTTONHOLE_LENGTH: f32 = 18.0;

    /// Move the trim with `f` (an affine map of the pattern's points),
    /// turning its slit with it.
    pub(crate) fn map(&mut self, f: impl Fn(Vec2) -> Vec2) {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let d = f(self.pos + Vec2::new(cos, sin)) - f(self.pos);
        self.angle = normalize_degrees(d.y.atan2(d.x).to_degrees());
        self.pos = f(self.pos);
    }

    /// A buttonhole's two ends, in the pattern's mm space.
    pub fn slit(&self) -> [Vec2; 2] {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let half = Vec2::new(cos, sin) * (Self::BUTTONHOLE_LENGTH * 0.5);
        [self.pos - half, self.pos + half]
    }
}

/// A button held in a buttonhole: a [`Tack`] from a button trim's point
/// to a buttonhole trim's. The tack is what holds them together when
/// draping; this says which trims it joins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fastening {
    pub tack: TackId,
    /// The button trim, as its pattern and index in `trims`.
    pub button: (PatternId, usize),
    pub hole: (PatternId, usize),
}

/// Trims this close (in mm) to a tack's point are the ones it holds.
const ON_TRIM: f32 = 1e-2;

/// The index of the trim of `kind` at `at` on `pattern`, if there is one.
pub fn trim_at(doc: &Document, pattern: PatternId, at: Vec2, kind: TrimKind) -> Option<usize> {
    doc.pattern(pattern)?
        .trims
        .iter()
        .position(|t| t.kind == kind && t.pos.distance(at) < ON_TRIM)
}

/// Every button fastened into a buttonhole: each tack that joins a button
/// trim's point to a buttonhole trim's, either way round.
pub fn fastenings(doc: &Document) -> Vec<Fastening> {
    let on = |p: &TackPoint, kind| trim_at(doc, p.pattern, p.at, kind).map(|i| (p.pattern, i));
    doc.tacks
        .iter()
        .filter_map(|tack| {
            let TackTarget::Pattern(b) = &tack.b else {
                return None;
            };
            let (button, hole) = match (on(&tack.a, TrimKind::Button), on(b, TrimKind::Buttonhole))
            {
                (Some(button), Some(hole)) => (button, hole),
                _ => (on(b, TrimKind::Button)?, on(&tack.a, TrimKind::Buttonhole)?),
            };
            Some(Fastening {
                tack: tack.id,
                button,
                hole,
            })
        })
        .collect()
}

/// Fasten a button into a buttonhole: a tack from one's point to the
/// other's, which pulls them together when draping.
pub struct Fasten(AddTack);

impl Fasten {
    pub fn new(button: TackPoint, hole: TackPoint) -> Self {
        Self(AddTack::new(button, TackTarget::Pattern(hole)))
    }
}

impl Command for Fasten {
    fn label(&self) -> String {
        "Fasten button".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        self.0.apply(doc);
    }

    fn undo(&mut self, doc: &mut Document) {
        self.0.undo(doc);
    }
}

/// Take a button out of its buttonhole: remove the tack that held it.
pub struct Unfasten(RemoveTack);

impl Unfasten {
    pub fn new(tack: TackId) -> Self {
        Self(RemoveTack::new(tack))
    }
}

impl Command for Unfasten {
    fn label(&self) -> String {
        "Unfasten button".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        self.0.apply(doc);
    }

    fn undo(&mut self, doc: &mut Document) {
        self.0.undo(doc);
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

    fn point(pattern: PatternId, x: f32, y: f32) -> TackPoint {
        TackPoint {
            pattern,
            at: Vec2::new(x, y),
        }
    }

    #[test]
    fn a_buttonholes_slit_turns_and_flips_with_its_pattern() {
        use crate::{AddTrim, Axis, MirrorPattern, RotatePattern};
        let (mut doc, [a, _]) = doc();
        AddTrim::new(a, TrimKind::Buttonhole, Vec2::new(30.0, 50.0))
            .at_angle(20.0)
            .apply(&mut doc);
        let hole = |doc: &Document| doc.pattern(a).unwrap().trims[0];
        let [from, to] = hole(&doc).slit();
        assert!((from.distance(to) - Trim::BUTTONHOLE_LENGTH).abs() < 1e-4);

        RotatePattern::new(a, 90.0).apply(&mut doc);
        assert!((hole(&doc).angle - 110.0).abs() < 1e-4);
        MirrorPattern::new(a, Axis::Vertical).apply(&mut doc);
        assert!((hole(&doc).angle - 70.0).abs() < 1e-4);
        MirrorPattern::new(a, Axis::Horizontal).apply(&mut doc);
        assert!((hole(&doc).angle + 70.0).abs() < 1e-4);

        // A general map (as a linked copy's) turns the slit the same way.
        let mut trim = hole(&doc);
        trim.map(|p| crate::document::rotate_point(p, 30.0, Vec2::ZERO));
        assert!((trim.angle + 40.0).abs() < 1e-3, "{}", trim.angle);
    }

    #[test]
    fn a_button_is_fastened_into_a_buttonhole_and_unfastened() {
        use crate::{AddTrim, MovePattern};
        let (mut doc, [a, b]) = doc();
        AddTrim::new(a, TrimKind::Button, Vec2::new(80.0, 40.0)).apply(&mut doc);
        AddTrim::new(b, TrimKind::Buttonhole, Vec2::new(170.0, 40.0)).apply(&mut doc);
        // A plain tack elsewhere is no fastening.
        AddTack::new(
            point(a, 10.0, 10.0),
            TackTarget::Pattern(point(b, 160.0, 10.0)),
        )
        .apply(&mut doc);
        assert!(fastenings(&doc).is_empty());

        let mut history = History::new();
        run(
            &mut history,
            &mut doc,
            Fasten::new(point(a, 80.0, 40.0), point(b, 170.0, 40.0)),
        );
        assert_eq!(history.undo_label(), Some("Fasten button".into()));
        let found = fastenings(&doc);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].button, found[0].hole), ((a, 0), (b, 0)));

        // It follows the pieces about, as the tack and trims move together.
        run(
            &mut history,
            &mut doc,
            MovePattern::new(b, Vec2::new(25.0, 5.0)),
        );
        assert_eq!(fastenings(&doc).len(), 1);

        run(&mut history, &mut doc, Unfasten::new(found[0].tack));
        assert_eq!(history.undo_label(), Some("Unfasten button".into()));
        assert!(fastenings(&doc).is_empty());
        assert_eq!(doc.tacks.len(), 1, "the plain tack stays");
        history.undo(&mut doc);
        assert_eq!(fastenings(&doc).len(), 1);
    }

    #[test]
    fn trims_saved_before_buttonholes_load_unturned() {
        let trim: Trim =
            serde_json::from_str(r#"{"kind": "snap", "pos": {"x": 1.0, "y": 2.0}}"#).unwrap();
        assert_eq!(trim.angle, 0.0);
        assert_eq!(trim.kind, TrimKind::Snap);
    }
}
