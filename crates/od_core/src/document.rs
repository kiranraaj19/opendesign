//! The document: everything that is saved in a project file.

use kurbo::ParamCurve;
use serde::{Deserialize, Serialize};

use crate::geometry::{self, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PatternId(pub u64);

/// How the outline passes a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointKind {
    /// The outline goes through the point, with a sharp turn.
    #[default]
    Corner,
    /// A Bezier control point: the outline curves towards it without
    /// touching it. Between two curve points the outline passes through
    /// their midpoint, so a run of them makes one smooth curve.
    Curve,
}

impl PointKind {
    pub fn toggled(self) -> Self {
        match self {
            PointKind::Corner => PointKind::Curve,
            PointKind::Curve => PointKind::Corner,
        }
    }
}

/// One point of a pattern outline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OutlinePoint {
    #[serde(flatten)]
    pub pos: Vec2,
    pub kind: PointKind,
}

impl OutlinePoint {
    pub const fn corner(pos: Vec2) -> Self {
        Self {
            pos,
            kind: PointKind::Corner,
        }
    }

    pub const fn curve(pos: Vec2) -> Self {
        Self {
            pos,
            kind: PointKind::Curve,
        }
    }
}

/// A small cut in the seam allowance that lines pieces up when sewing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Notch {
    /// Millimetres along the outline from where it starts (its first
    /// corner point), in drawing order.
    pub at: f32,
}

/// A line drawn inside a pattern: a dart, pocket or placement mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InternalLine {
    pub points: Vec<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SewingLineId(pub u64);

/// One side of a seam: a corner-to-corner run of a pattern's outline (see
/// [`Pattern::segment_at`]). `end` is the outline index that closes the run;
/// it may be `>= outline.len()` when the run wraps past the end of the
/// array, in the same way [`FlatPoint::param`] wraps are represented
/// elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SeamEdge {
    pub pattern: PatternId,
    pub start: usize,
    pub end: usize,
}

/// A seam: two edges, on the same or different patterns, sewn together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SewingLine {
    pub id: SewingLineId,
    pub a: SeamEdge,
    pub b: SeamEdge,
}

/// How much two sewn edges may differ in length, as a fraction of the
/// shorter one, before it is flagged as a construction problem (easing and
/// gathers aside, a seam this far off will pucker or strain badly once the
/// garment is draped).
pub const LENGTH_MISMATCH_TOLERANCE: f32 = 0.03;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TackId(pub u64);

/// A single outline point, referenced by a [`Tack`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SeamPoint {
    pub pattern: PatternId,
    pub index: usize,
}

/// A point-to-point stitch: pins two outline points together (a dart tip, a
/// single alignment mark) without implying a sewn edge between them, the
/// way a [`SewingLine`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tack {
    pub id: TackId,
    pub a: SeamPoint,
    pub b: SeamPoint,
}

impl SewingLine {
    /// The lengths in mm of edges `a` and `b`. `None` if either referenced
    /// pattern no longer exists.
    fn edge_lengths(&self, doc: &Document) -> Option<(f32, f32)> {
        let len = |e: &SeamEdge| {
            doc.pattern(e.pattern)
                .map(|p| p.segment_length(e.start, e.end, TOLERANCE))
        };
        Some((len(&self.a)?, len(&self.b)?))
    }

    /// Difference in length between the two sewn edges, in mm. Positive
    /// means `a` is longer than `b`. `None` if either referenced pattern no
    /// longer exists.
    pub fn length_mismatch(&self, doc: &Document) -> Option<f32> {
        self.edge_lengths(doc).map(|(a, b)| a - b)
    }

    /// Whether the two edges differ in length by more than
    /// [`LENGTH_MISMATCH_TOLERANCE`] of the shorter one.
    pub fn is_length_mismatched(&self, doc: &Document) -> bool {
        self.edge_lengths(doc).is_some_and(|(a, b)| {
            let shorter = a.min(b);
            shorter > 0.0 && (a - b).abs() / shorter > LENGTH_MISMATCH_TOLERANCE
        })
    }
}

/// Mirror axis through a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// A vertical line: left and right swap.
    Vertical,
    /// A horizontal line: top and bottom swap.
    Horizontal,
}

impl Axis {
    fn reflect(self, p: Vec2, about: f32) -> Vec2 {
        match self {
            Axis::Vertical => Vec2::new(2.0 * about - p.x, p.y),
            Axis::Horizontal => Vec2::new(p.x, 2.0 * about - p.y),
        }
    }
}

/// Flattening tolerance used for drawing, measuring and hit-testing, in mm.
pub const TOLERANCE: f32 = 0.2;

/// A flattened outline point. `param` says where it lies along the outline
/// points: `i + f` is a fraction `f` of the way from point `i` towards
/// point `i + 1` (wrapping at the end).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatPoint {
    pub pos: Vec2,
    pub param: f32,
}

/// A flat garment piece, drawn in the 2D window and draped in 3D.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub id: PatternId,
    pub name: String,
    /// Closed outline in millimetres.
    pub outline: Vec<OutlinePoint>,
    /// Width of the seam allowance drawn around the outline, in mm. Zero
    /// for none.
    pub seam_allowance: f32,
    /// Grainline direction in degrees, clockwise on screen from straight
    /// down the pattern.
    pub grain_angle: f32,
    pub notches: Vec<Notch>,
    pub internal_lines: Vec<InternalLine>,
}

/// Seam allowance given to new patterns, in mm.
pub const DEFAULT_SEAM_ALLOWANCE: f32 = 10.0;

impl Pattern {
    /// A pattern with default seam allowance, grainline and no markings.
    pub fn new(id: PatternId, name: impl Into<String>, outline: Vec<OutlinePoint>) -> Self {
        Self {
            id,
            name: name.into(),
            outline,
            seam_allowance: DEFAULT_SEAM_ALLOWANCE,
            grain_angle: 0.0,
            notches: Vec::new(),
            internal_lines: Vec::new(),
        }
    }

    /// The outline flattened to straight segments no further than
    /// `tolerance` mm from the true curves. Closed: the last point joins
    /// the first.
    pub fn polyline(&self, tolerance: f32) -> Vec<Vec2> {
        self.flatten(tolerance).into_iter().map(|f| f.pos).collect()
    }

    /// Like [`Pattern::polyline`], with where each point came from.
    pub fn flatten(&self, tolerance: f32) -> Vec<FlatPoint> {
        flatten(&self.outline, tolerance)
    }

    pub fn positions(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.outline.iter().map(|p| p.pos)
    }

    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        geometry::bounds(&self.polyline(TOLERANCE))
    }

    pub fn area(&self) -> f32 {
        geometry::area(&self.polyline(TOLERANCE))
    }

    pub fn perimeter(&self) -> f32 {
        geometry::perimeter(&self.polyline(TOLERANCE))
    }

    pub fn contains(&self, p: Vec2) -> bool {
        geometry::contains(&self.polyline(TOLERANCE), p)
    }

    /// Where the grainline crosses the pattern: the centre of its bounds.
    pub fn center(&self) -> Vec2 {
        self.bounds()
            .map_or(Vec2::ZERO, |(min, max)| (min + max) * 0.5)
    }

    /// Mirror everything about the line `axis` = `about` (an x for a
    /// vertical axis, a y for a horizontal one). Point order is kept, so
    /// indices and notch distances stay valid.
    pub fn mirror(&mut self, axis: Axis, about: f32) {
        for p in &mut self.outline {
            p.pos = axis.reflect(p.pos, about);
        }
        for line in &mut self.internal_lines {
            for p in &mut line.points {
                *p = axis.reflect(*p, about);
            }
        }
        self.grain_angle = normalize_degrees(-self.grain_angle);
    }

    /// Move everything by `delta`.
    pub fn translate(&mut self, delta: Vec2) {
        self.outline.iter_mut().for_each(|p| p.pos += delta);
        for line in &mut self.internal_lines {
            line.points.iter_mut().for_each(|p| *p += delta);
        }
    }

    /// Indices of the outline's corner points, in outline order.
    fn corner_indices(&self) -> Vec<usize> {
        (0..self.outline.len())
            .filter(|&i| self.outline[i].kind == PointKind::Corner)
            .collect()
    }

    /// The corner-to-corner span of the outline that outline parameter
    /// `param` (see [`FlatPoint::param`]) falls on: the previous corner
    /// point up to the next one, carrying along any curve points between
    /// them. This is the natural unit to sew a seam to, since a curved
    /// edge is one span even though it is several outline points.
    ///
    /// `end` may be `>= outline.len()` if the span wraps past the end of
    /// the array; a pattern with no corners (all curve points) is one span,
    /// the whole outline.
    pub fn segment_at(&self, param: f32) -> (usize, usize) {
        let n = self.outline.len();
        let corners = self.corner_indices();
        let Some(&base) = corners.first() else {
            return (0, n);
        };
        let rel = |i: usize| (i as f32 - base as f32).rem_euclid(n as f32);
        let p_rel = (param - base as f32).rem_euclid(n as f32);
        // The corner with the greatest `rel` not past `p_rel` starts the span.
        let best = corners
            .iter()
            .copied()
            .filter(|&c| rel(c) <= p_rel)
            .max_by(|&a, &b| rel(a).total_cmp(&rel(b)))
            .unwrap_or(base);
        let idx = corners.iter().position(|&c| c == best).unwrap();
        let next = corners[(idx + 1) % corners.len()];
        let end = if next as f32 <= best as f32 {
            next + n
        } else {
            next
        };
        (best, end)
    }

    /// The flattened points of the outline between outline parameters
    /// `start` and `end` (as returned by [`Pattern::segment_at`]), in
    /// travel order.
    pub fn segment_points(&self, start: usize, end: usize, tolerance: f32) -> Vec<Vec2> {
        let n = self.outline.len();
        if n == 0 || end <= start {
            return Vec::new();
        }
        let nf = n as f32;
        // Three laps of the flattened outline's parameters, so a window
        // anywhere within one lap of `start..end` is contiguous in the
        // combined, sorted list regardless of where the outline itself
        // happens to wrap.
        let mut pts: Vec<(f32, Vec2)> = self
            .flatten(tolerance)
            .into_iter()
            .flat_map(|f| [-nf, 0.0, nf].map(|shift| (f.param + shift, f.pos)))
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (s, e) = (start as f32, end as f32);
        pts.into_iter()
            .filter(|&(param, _)| param >= s - 1e-3 && param <= e + 1e-3)
            .map(|(_, pos)| pos)
            .collect()
    }

    /// Length in mm of the outline between outline parameters `start` and
    /// `end`. Used to check that two sewn edges are a comparable length.
    pub fn segment_length(&self, start: usize, end: usize, tolerance: f32) -> f32 {
        self.segment_points(start, end, tolerance)
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum()
    }
}

/// An angle in degrees brought into -180..180.
pub fn normalize_degrees(a: f32) -> f32 {
    let r = (a + 180.0).rem_euclid(360.0) - 180.0;
    if r == -0.0 {
        0.0
    } else {
        r
    }
}

/// Flatten a closed outline of corner and curve points. Runs of curve
/// points are quadratic Bezier splines (as in TrueType fonts): each curve
/// point is a control point, with an implied on-curve point halfway between
/// two neighbouring curve points.
fn flatten(outline: &[OutlinePoint], tolerance: f32) -> Vec<FlatPoint> {
    let n = outline.len();
    if n < 3 {
        return outline
            .iter()
            .enumerate()
            .map(|(i, p)| FlatPoint {
                pos: p.pos,
                param: i as f32,
            })
            .collect();
    }
    let at = |u: usize| outline[u % n];

    // Start on a point the outline passes through: the first corner, or
    // with only curve points, the midpoint of the first two.
    let (mut cur, mut ctrl, first) = match outline.iter().position(|p| p.kind == PointKind::Corner)
    {
        Some(s) => ((at(s).pos, s as f32), None, s + 1),
        None => (
            (at(0).pos.lerp(at(1).pos, 0.5), 0.5),
            Some((at(1).pos, 1.0)),
            2,
        ),
    };
    let end = first + n;
    let mut out = vec![FlatPoint {
        pos: cur.0,
        param: cur.1,
    }];
    for u in first..end {
        let q = at(u);
        let uq = u as f32;
        match (q.kind, ctrl) {
            (PointKind::Corner, None) => {
                out.push(FlatPoint {
                    pos: q.pos,
                    param: uq,
                });
                cur = (q.pos, uq);
            }
            (PointKind::Corner, Some(c)) => {
                quad(&mut out, cur, c, (q.pos, uq), tolerance);
                cur = (q.pos, uq);
                ctrl = None;
            }
            (PointKind::Curve, None) => ctrl = Some((q.pos, uq)),
            (PointKind::Curve, Some(c)) => {
                let mid = (c.0.lerp(q.pos, 0.5), c.1 + 0.5);
                quad(&mut out, cur, c, mid, tolerance);
                cur = mid;
                ctrl = Some((q.pos, uq));
            }
        }
    }
    // The walk ends back at the start; a closed polyline doesn't repeat it.
    out.pop();
    for f in &mut out {
        f.param = f.param.rem_euclid(n as f32);
    }
    out
}

/// Append a quadratic Bezier from `a` (already in `out`) via `c` to `b`.
/// Each tuple is a position and its outline parameter.
fn quad(out: &mut Vec<FlatPoint>, a: (Vec2, f32), c: (Vec2, f32), b: (Vec2, f32), tolerance: f32) {
    let kp = |v: Vec2| kurbo::Point::new(f64::from(v.x), f64::from(v.y));
    let curve = kurbo::QuadBez::new(kp(a.0), kp(c.0), kp(b.0));
    // A quadratic strays at most |a - 2c + b| / (4 n²) from n even chords.
    let bend = (a.0 - c.0 * 2.0 + b.0).length();
    let steps = ((bend / (4.0 * tolerance.max(1e-3))).sqrt().ceil() as usize).clamp(1, 256);
    for k in 1..=steps {
        let t = k as f32 / steps as f32;
        let p = curve.eval(f64::from(t));
        let param = if t < 0.5 {
            a.1 + (c.1 - a.1) * (t * 2.0)
        } else {
            c.1 + (b.1 - c.1) * (t * 2.0 - 1.0)
        };
        out.push(FlatPoint {
            pos: Vec2::new(p.x as f32, p.y as f32),
            param,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    pub patterns: Vec<Pattern>,
    pub sewing_lines: Vec<SewingLine>,
    pub tacks: Vec<Tack>,
    next_id: u64,
    next_sewing_id: u64,
    next_tack_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            patterns: Vec::new(),
            sewing_lines: Vec::new(),
            tacks: Vec::new(),
            next_id: 1,
            next_sewing_id: 1,
            next_tack_id: 1,
        }
    }

    /// Check a document read from a file: ids must be unique, and new ids
    /// must not collide with existing ones even if the file says otherwise.
    pub(crate) fn validate(&mut self) -> Result<(), PatternId> {
        let mut ids: Vec<PatternId> = self.patterns.iter().map(|p| p.id).collect();
        ids.sort();
        if let Some(pair) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(pair[0]);
        }
        let max = ids.last().map_or(0, |id| id.0);
        self.next_id = self.next_id.max(max + 1);
        let smax = self.sewing_lines.iter().map(|s| s.id.0).max().unwrap_or(0);
        self.next_sewing_id = self.next_sewing_id.max(smax + 1);
        let tmax = self.tacks.iter().map(|t| t.id.0).max().unwrap_or(0);
        self.next_tack_id = self.next_tack_id.max(tmax + 1);
        Ok(())
    }

    /// Reserve a fresh id. Ids are never reused, even after undo.
    pub fn allocate_id(&mut self) -> PatternId {
        let id = PatternId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Reserve a fresh sewing line id. Ids are never reused, even after undo.
    pub fn allocate_sewing_id(&mut self) -> SewingLineId {
        let id = SewingLineId(self.next_sewing_id);
        self.next_sewing_id += 1;
        id
    }

    /// Reserve a fresh tack id. Ids are never reused, even after undo.
    pub fn allocate_tack_id(&mut self) -> TackId {
        let id = TackId(self.next_tack_id);
        self.next_tack_id += 1;
        id
    }

    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut Pattern> {
        self.patterns.iter_mut().find(|p| p.id == id)
    }

    pub fn index_of(&self, id: PatternId) -> Option<usize> {
        self.patterns.iter().position(|p| p.id == id)
    }

    /// Topmost pattern under `p` (later patterns draw on top).
    pub fn pattern_at(&self, p: Vec2) -> Option<PatternId> {
        self.patterns
            .iter()
            .rev()
            .find(|pat| pat.contains(p))
            .map(|pat| pat.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(points: &[(f32, f32, PointKind)]) -> Pattern {
        let outline = points
            .iter()
            .map(|&(x, y, kind)| OutlinePoint {
                pos: Vec2::new(x, y),
                kind,
            })
            .collect();
        Pattern::new(PatternId(1), "Test", outline)
    }

    use PointKind::{Corner as C, Curve as Q};

    #[test]
    fn corner_outline_flattens_to_itself() {
        let p = pattern(&[(0.0, 0.0, C), (100.0, 0.0, C), (100.0, 50.0, C)]);
        assert_eq!(
            p.polyline(TOLERANCE),
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(100.0, 0.0),
                Vec2::new(100.0, 50.0)
            ]
        );
        let params: Vec<f32> = p.flatten(TOLERANCE).iter().map(|f| f.param).collect();
        assert_eq!(params, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn curve_point_bends_the_outline_within_tolerance() {
        // A corner-curve-corner run is one quadratic Bezier.
        let p = pattern(&[
            (0.0, 0.0, C),
            (50.0, 100.0, Q),
            (100.0, 0.0, C),
            (50.0, -50.0, C),
        ]);
        let poly = p.polyline(0.1);
        assert!(poly.len() > 10, "curve should be subdivided");
        // The quadratic's peak is halfway to the control point.
        let peak = poly.iter().map(|v| v.y).fold(f32::MIN, f32::max);
        assert!((peak - 50.0).abs() < 0.1, "peak {peak}");
        // Every flattened point lies on the true curve.
        for v in &poly[1..poly.len() - 1] {
            // B(t) = (100 t, 200 t (1 - t)) → y = 2x (1 - x/100)
            let expected = 2.0 * v.x * (1.0 - v.x / 100.0);
            if v.y >= 0.0 {
                assert!((v.y - expected).abs() < 0.01, "{v:?}");
            }
        }
        // Chords stay within tolerance of the curve: check midpoints.
        for w in poly.windows(2).filter(|w| w[0].y >= 0.0 && w[1].y >= 0.0) {
            let m = w[0].lerp(w[1], 0.5);
            let expected = 2.0 * m.x * (1.0 - m.x / 100.0);
            assert!((m.y - expected).abs() <= 0.1 + 1e-3, "chord {m:?}");
        }
        // Tighter tolerance means more points.
        assert!(p.polyline(0.01).len() > poly.len());
    }

    #[test]
    fn neighbouring_curve_points_join_smoothly_through_their_midpoint() {
        let p = pattern(&[
            (0.0, 0.0, C),
            (0.0, 100.0, Q),
            (100.0, 100.0, Q),
            (100.0, 0.0, C),
        ]);
        let poly = p.polyline(0.05);
        assert!(poly
            .iter()
            .any(|v| v.distance(Vec2::new(50.0, 100.0)) < 1e-3));
        assert!(poly.iter().all(|v| v.y <= 100.0 + 1e-3));
    }

    #[test]
    fn all_curve_outline_is_a_smooth_loop() {
        let p = pattern(&[
            (0.0, 0.0, Q),
            (100.0, 0.0, Q),
            (100.0, 100.0, Q),
            (0.0, 100.0, Q),
        ]);
        let poly = p.polyline(0.05);
        // Passes through every side's midpoint, never the corners.
        for mid in [(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)] {
            let mid = Vec2::new(mid.0, mid.1);
            assert!(poly.iter().any(|v| v.distance(mid) < 1e-3), "{mid:?}");
        }
        assert!(!p.contains(Vec2::new(1.0, 1.0)));
        assert!(p.contains(Vec2::new(50.0, 50.0)));
        assert!(poly.first() != poly.last(), "closing point not repeated");
    }

    #[test]
    fn params_increase_around_the_outline() {
        let p = pattern(&[
            (0.0, 0.0, C),
            (50.0, -40.0, Q),
            (100.0, 0.0, C),
            (100.0, 100.0, Q),
            (0.0, 100.0, Q),
        ]);
        let flat = p.flatten(TOLERANCE);
        assert_eq!(flat[0].param, 0.0);
        for w in flat.windows(2) {
            assert!(w[1].param > w[0].param, "{w:?}");
        }
        assert!(flat.last().unwrap().param < 5.0);
    }

    #[test]
    fn mirror_keeps_order_and_flips_grain() {
        let mut p = pattern(&[(0.0, 0.0, C), (100.0, 0.0, Q), (100.0, 50.0, C)]);
        p.grain_angle = 30.0;
        p.internal_lines.push(InternalLine {
            points: vec![Vec2::new(10.0, 10.0), Vec2::new(20.0, 10.0)],
        });
        p.mirror(Axis::Vertical, 0.0);
        assert_eq!(p.outline[1].pos, Vec2::new(-100.0, 0.0));
        assert_eq!(p.outline[1].kind, PointKind::Curve);
        assert_eq!(p.internal_lines[0].points[1], Vec2::new(-20.0, 10.0));
        assert_eq!(p.grain_angle, -30.0);
        p.mirror(Axis::Horizontal, 10.0);
        assert_eq!(p.outline[2].pos, Vec2::new(-100.0, -30.0));
    }

    #[test]
    fn degrees_wrap_into_range() {
        assert_eq!(normalize_degrees(190.0), -170.0);
        assert_eq!(normalize_degrees(-0.0), 0.0);
        assert_eq!(normalize_degrees(360.0), 0.0);
    }

    /// A 100 × 50 rectangle with a curve point part way along the top edge
    /// (indices: 0 corner, 1 curve, 2 corner, 3 corner, 4 corner).
    fn rect_with_curve() -> Pattern {
        pattern(&[
            (0.0, 0.0, C),
            (50.0, -10.0, Q),
            (100.0, 0.0, C),
            (100.0, 50.0, C),
            (0.0, 50.0, C),
        ])
    }

    #[test]
    fn segment_at_spans_a_whole_curved_edge() {
        let p = rect_with_curve();
        // Anywhere from the first corner up to (not including) the next
        // corner is the same span, curve point and all.
        assert_eq!(p.segment_at(0.0), (0, 2));
        assert_eq!(p.segment_at(0.5), (0, 2));
        assert_eq!(p.segment_at(1.9), (0, 2));
        // The last edge wraps past the end of the array.
        assert_eq!(p.segment_at(4.0), (4, 5));
        assert_eq!(p.segment_at(4.5), (4, 5));
    }

    #[test]
    fn segment_at_with_no_corners_is_the_whole_outline() {
        let p = pattern(&[(0.0, 0.0, Q), (100.0, 0.0, Q), (50.0, 50.0, Q)]);
        assert_eq!(p.segment_at(1.3), (0, 3));
    }

    #[test]
    fn segment_points_cover_only_the_requested_span() {
        let p = rect_with_curve();
        let straight = p.segment_points(2, 3, TOLERANCE);
        assert_eq!(
            straight,
            vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 50.0)]
        );
        // The curved span starts and ends at the same points as the plain
        // outline, with extra points for the bend in between.
        let curved = p.segment_points(0, 2, TOLERANCE);
        assert_eq!(curved.first(), Some(&Vec2::new(0.0, 0.0)));
        assert_eq!(curved.last(), Some(&Vec2::new(100.0, 0.0)));
        assert!(curved.len() > 2, "the curve should be subdivided");
        // A span that wraps past the end of the array still joins up.
        let wrapped = p.segment_points(4, 5, TOLERANCE);
        assert_eq!(wrapped, vec![Vec2::new(0.0, 50.0), Vec2::new(0.0, 0.0)]);
    }

    #[test]
    fn segment_length_matches_a_straight_edge_and_exceeds_the_chord_on_a_curve() {
        let p = rect_with_curve();
        assert_eq!(p.segment_length(2, 3, TOLERANCE), 50.0);
        let curved = p.segment_length(0, 2, TOLERANCE);
        assert!(curved > 100.0, "{curved} should be longer than the chord");
    }

    fn doc_with_two_rects(sizes: [(f32, f32); 2]) -> Document {
        let mut doc = Document::new("Test");
        for (i, (w, h)) in sizes.into_iter().enumerate() {
            doc.patterns.push(Pattern::new(
                PatternId(i as u64 + 1),
                format!("Panel {i}"),
                geometry::rectangle(Vec2::ZERO, Vec2::new(w, h))
                    .into_iter()
                    .map(OutlinePoint::corner)
                    .collect(),
            ));
        }
        doc
    }

    #[test]
    fn matching_edges_are_not_mismatched() {
        let doc = doc_with_two_rects([(100.0, 50.0), (100.0, 80.0)]);
        // The 100 mm sides of each rectangle (edge 0: index 0 to 1).
        let seam = SewingLine {
            id: SewingLineId(1),
            a: SeamEdge {
                pattern: PatternId(1),
                start: 0,
                end: 1,
            },
            b: SeamEdge {
                pattern: PatternId(2),
                start: 0,
                end: 1,
            },
        };
        assert_eq!(seam.length_mismatch(&doc), Some(0.0));
        assert!(!seam.is_length_mismatched(&doc));
    }

    #[test]
    fn differing_edges_are_flagged_as_mismatched() {
        let doc = doc_with_two_rects([(100.0, 50.0), (80.0, 50.0)]);
        // The 50 mm sides of the first rectangle (edge 1: index 1 to 2)
        // against the 80 mm side of the second (edge 0: index 0 to 1).
        let seam = SewingLine {
            id: SewingLineId(1),
            a: SeamEdge {
                pattern: PatternId(1),
                start: 1,
                end: 2,
            },
            b: SeamEdge {
                pattern: PatternId(2),
                start: 0,
                end: 1,
            },
        };
        assert_eq!(seam.length_mismatch(&doc), Some(-30.0));
        assert!(seam.is_length_mismatched(&doc));
    }

    #[test]
    fn mismatch_is_none_for_a_missing_pattern() {
        let doc = doc_with_two_rects([(100.0, 50.0), (100.0, 50.0)]);
        let seam = SewingLine {
            id: SewingLineId(1),
            a: SeamEdge {
                pattern: PatternId(1),
                start: 0,
                end: 1,
            },
            b: SeamEdge {
                pattern: PatternId(99),
                start: 0,
                end: 1,
            },
        };
        assert_eq!(seam.length_mismatch(&doc), None);
        assert!(!seam.is_length_mismatched(&doc));
    }
}
