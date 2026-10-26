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
    next_id: u64,
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
            next_id: 1,
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
        Ok(())
    }

    /// Reserve a fresh id. Ids are never reused, even after undo.
    pub fn allocate_id(&mut self) -> PatternId {
        let id = PatternId(self.next_id);
        self.next_id += 1;
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
}
