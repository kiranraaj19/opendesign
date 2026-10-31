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

/// A line drawn inside a pattern: a dart, pocket or placement mark, or a
/// fold line when `fold` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InternalLine {
    pub points: Vec<Vec2>,
    /// The cloth folds along this line when draped, as for a pleat.
    #[serde(default)]
    pub fold: Option<Fold>,
}

impl InternalLine {
    pub fn new(points: Vec<Vec2>) -> Self {
        Self { points, fold: None }
    }
}

/// How the cloth folds along a fold line.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fold {
    /// How far the cloth turns at the line, in degrees (0 stays flat, 180
    /// folds it right back on itself). Positive folds the two sides
    /// towards the viewer of the 2D window (a valley seen from there),
    /// negative away from them (a mountain).
    pub angle: f32,
    /// How firmly the fold holds its angle, 0 (as soft as the fabric) to
    /// 1 (as stiff as the fabric is against stretching).
    pub strength: f32,
}

impl Fold {
    pub const DEFAULT_ANGLE: f32 = 170.0;
    pub const DEFAULT_STRENGTH: f32 = 0.5;

    pub fn valley() -> Self {
        Self {
            angle: Self::DEFAULT_ANGLE,
            strength: Self::DEFAULT_STRENGTH,
        }
    }

    pub fn mountain() -> Self {
        Self {
            angle: -Self::DEFAULT_ANGLE,
            ..Self::valley()
        }
    }
}

/// A dart inside a piece: a diamond from `start` to `end` (its two tips),
/// `width` mm across at the middle. When draping, its two legs (the sides
/// either side of the line between the tips) are stitched together, so
/// the fabric between them folds away and the piece takes on shape.
///
/// A dart on the outline needs no type of its own: it is a V in the
/// outline whose legs are sewn together by an ordinary [`SewingLine`]
/// (see [`crate::edit::outline_dart`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Dart {
    pub start: Vec2,
    pub end: Vec2,
    pub width: f32,
}

impl Dart {
    /// The diamond's two side points, left and right of `start` → `end`.
    pub fn sides(&self) -> [Vec2; 2] {
        let mid = self.start.lerp(self.end, 0.5);
        let d = self.end - self.start;
        let len = d.length();
        if len < 1e-6 {
            return [mid, mid];
        }
        let perp = Vec2::new(-d.y, d.x) * (self.width * 0.5 / len);
        [mid + perp, mid - perp]
    }

    /// The two legs, each from `start` via one side point to `end`. They
    /// are sewn together point for point.
    pub fn legs(&self) -> [[Vec2; 3]; 2] {
        let [a, b] = self.sides();
        [[self.start, a, self.end], [self.start, b, self.end]]
    }

    fn map(&mut self, f: impl Fn(Vec2) -> Vec2) {
        self.start = f(self.start);
        self.end = f(self.end);
    }
}

/// A topstitch line: visually and structurally like an [`InternalLine`]
/// (an open polyline in the pattern's own mm coordinates), but drawn and
/// draped as a decorative/reinforcing line of stitching — typically offset
/// a few mm in from a seamed edge — rather than a cut or fold mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topstitch {
    pub points: Vec<Vec2>,
}

/// What kind of point trim is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimKind {
    #[default]
    Button,
    Snap,
    /// A slit a button is pushed through (see [`crate::details`]).
    Buttonhole,
}

/// A point trim (button, snap, buttonhole) placed on a pattern's outline
/// or interior, in the pattern's own mm coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    pub kind: TrimKind,
    pub pos: Vec2,
    /// Which way a buttonhole's slit runs, in degrees clockwise on screen
    /// from the pattern's x axis. It turns and flips with the pattern.
    #[serde(default)]
    pub angle: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SewingLineId(pub u64);

/// One run of a pattern's outline, as outline parameters (see
/// [`FlatPoint::param`]): a whole number of corner spans for segment sewing
/// (see [`Pattern::segment_at`]), or any sub-range for free sewing. `end`
/// may exceed `outline.len()` when the run wraps past the end of the array.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SeamEdge {
    pub pattern: PatternId,
    pub start: f32,
    pub end: f32,
    /// Sewn from `start` to `end` (`true`), or from `end` back to `start`.
    /// Two edges that face opposite ways around their outlines (a front's
    /// right side and a back's left side) are sewn untwisted by giving
    /// them opposite directions.
    pub forward: bool,
}

/// A seam: two groups of edges, on the same or different patterns, sewn
/// together along their combined length. Both groups normally hold one
/// edge each; a group with more than one is the "many" side of a
/// one-to-many seam (a sleeve cap sewn to a front and a back armhole, say).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SewingLine {
    pub id: SewingLineId,
    pub a: Vec<SeamEdge>,
    pub b: Vec<SeamEdge>,
    /// What else the seam is: a zipper, say (see [`crate::details`]).
    #[serde(default, skip_serializing_if = "crate::details::SeamDetails::is_plain")]
    pub details: crate::details::SeamDetails,
}

/// How much two sewn edges may differ in length, as a fraction of the
/// shorter one, before it is flagged as a construction problem (easing and
/// gathers aside, a seam this far off will pucker or strain badly once the
/// garment is draped).
pub const LENGTH_MISMATCH_TOLERANCE: f32 = 0.03;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TackId(pub u64);

/// A point on a pattern, in the same millimetre space as its outline: on
/// the outline or inside it. It moves with the pattern when the pattern is
/// moved or flipped.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TackPoint {
    pub pattern: PatternId,
    pub at: Vec2,
}

/// What a [`Tack`] holds its pattern point to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TackTarget {
    Pattern(TackPoint),
    /// A point on the avatar's surface, in metres in the 3D view's world
    /// (Y up). Kept as placed: resizing the avatar doesn't move it. Tacks
    /// placed before format 10 hold these.
    Avatar([f32; 3]),
    /// A point on the avatar's surface: one triangle of its mesh (by index
    /// into its triangle list) and barycentric weights in it. The built-in
    /// bodies keep the same triangles at every size, so the point follows
    /// the body when it is resized.
    AvatarSurface {
        triangle: u32,
        weights: [f32; 3],
    },
}

/// A point-to-point stitch: holds a point on a pattern to a point on
/// another (a dart tip, an alignment mark) or to the avatar, without
/// implying a sewn edge between them the way a [`SewingLine`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tack {
    pub id: TackId,
    pub a: TackPoint,
    pub b: TackTarget,
}

impl Tack {
    /// This tack's pattern points: `a`, and `b` unless it is on the avatar.
    pub fn points(&self) -> impl Iterator<Item = &TackPoint> {
        let b = match &self.b {
            TackTarget::Pattern(p) => Some(p),
            _ => None,
        };
        std::iter::once(&self.a).chain(b)
    }

    pub fn points_mut(&mut self) -> impl Iterator<Item = &mut TackPoint> {
        let b = match &mut self.b {
            TackTarget::Pattern(p) => Some(p),
            _ => None,
        };
        std::iter::once(&mut self.a).chain(b)
    }

    pub fn touches(&self, pattern: PatternId) -> bool {
        self.points().any(|p| p.pattern == pattern)
    }
}

impl SewingLine {
    /// Total length in mm of one side's edges. `None` if any of their
    /// patterns no longer exists.
    fn group_length(doc: &Document, group: &[SeamEdge]) -> Option<f32> {
        group
            .iter()
            .map(|e| {
                doc.pattern(e.pattern)
                    .map(|p| p.segment_length(e.start, e.end, TOLERANCE))
            })
            .sum()
    }

    /// The combined lengths in mm of sides `a` and `b`. `None` if either
    /// side has an edge whose pattern no longer exists.
    pub fn side_lengths(&self, doc: &Document) -> Option<(f32, f32)> {
        Some((
            Self::group_length(doc, &self.a)?,
            Self::group_length(doc, &self.b)?,
        ))
    }

    /// Difference in combined length between the two sides, in mm. Positive
    /// means `a` is longer than `b`. `None` if either side has an edge
    /// whose pattern no longer exists.
    pub fn length_mismatch(&self, doc: &Document) -> Option<f32> {
        self.side_lengths(doc).map(|(a, b)| a - b)
    }

    /// Whether the two sides differ in combined length by more than
    /// [`LENGTH_MISMATCH_TOLERANCE`] of the shorter one.
    pub fn is_length_mismatched(&self, doc: &Document) -> bool {
        self.is_mismatched_by(doc, LENGTH_MISMATCH_TOLERANCE)
    }

    /// Whether the two sides differ in combined length by more than
    /// `fraction` of the shorter one.
    pub fn is_mismatched_by(&self, doc: &Document, fraction: f32) -> bool {
        self.side_lengths(doc).is_some_and(|(a, b)| {
            let shorter = a.min(b);
            shorter > 0.0 && (a - b).abs() / shorter > fraction
        })
    }
}

/// How a fabric's surface looks in the 3D view, besides its flat tint: a
/// hint for [`od_render`] to pick a procedural weave texture. Kept as a tag
/// here rather than pixels, since `od_core` has no GPU or image dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FabricTexture {
    /// A flat tint with no weave pattern (e.g. silk's smooth surface).
    #[default]
    Flat,
    /// A fine, even plain weave (e.g. cotton).
    Plain,
    /// A coarser diagonal twill weave (e.g. denim).
    Twill,
}

/// A physical fabric: how heavy and stiff it drapes, and how it looks.
/// Every [`Pattern`] has one (see [`Pattern::fabric`]); [`od_sim`] reads the
/// physical fields to set particle mass and constraint compliance, and
/// `od_render`/`od_app` read `color`/`texture` to shade it in the 3D view.
///
/// Woven cloth behaves differently along its threads: the warp runs along
/// the pattern's grainline ([`Pattern::grain_angle`]), the weft across it,
/// and pulling on the bias (diagonally) shears the weave. So stretching
/// and bending are set per direction; see [`Fabric::stretch_compliance_at`]
/// for how a constraint at any angle mixes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "crate::fabric::FabricFields")]
pub struct Fabric {
    pub name: String,
    /// Mass of one square metre of fabric, in kg.
    pub mass_per_area: f32,
    /// XPBD compliance (inverse stiffness, m/N) of the constraints that
    /// resist stretching along the warp (the grainline). Lower is stiffer.
    pub stretch_compliance: f32,
    /// Like `stretch_compliance`, across the grain (along the weft).
    pub weft_stretch_compliance: f32,
    /// Compliance against shearing the weave, for pulls on the bias.
    pub shear_compliance: f32,
    /// XPBD compliance of the constraints that resist folding the warp
    /// threads (bending along the grainline). Real cloth bends far more
    /// easily than it stretches, so this is normally much higher (softer)
    /// than `stretch_compliance`.
    pub bending_compliance: f32,
    /// Like `bending_compliance`, for folding the weft threads.
    pub weft_bending_compliance: f32,
    /// Coefficient of friction against other surfaces, 0..1. Not yet read
    /// by `od_sim`'s collision (reserved for a future pass).
    pub friction: f32,
    /// Base tint, straight sRGB 0..255.
    pub color: [u8; 3],
    pub texture: FabricTexture,
}

impl Fabric {
    pub fn cotton() -> Self {
        Self {
            name: "Cotton".into(),
            mass_per_area: 0.15,
            stretch_compliance: 1e-7,
            weft_stretch_compliance: 2e-7,
            shear_compliance: 5e-7,
            bending_compliance: 1e-5,
            weft_bending_compliance: 1.5e-5,
            friction: 0.3,
            color: [235, 230, 210],
            texture: FabricTexture::Plain,
        }
    }

    pub fn silk() -> Self {
        Self {
            name: "Silk".into(),
            mass_per_area: 0.05,
            stretch_compliance: 4e-7,
            weft_stretch_compliance: 6e-7,
            shear_compliance: 2e-6,
            bending_compliance: 8e-5,
            weft_bending_compliance: 1e-4,
            friction: 0.1,
            color: [232, 226, 236],
            texture: FabricTexture::Flat,
        }
    }

    pub fn denim() -> Self {
        Self {
            name: "Denim".into(),
            mass_per_area: 0.40,
            stretch_compliance: 2e-8,
            weft_stretch_compliance: 4e-8,
            shear_compliance: 1e-7,
            bending_compliance: 2e-6,
            weft_bending_compliance: 3e-6,
            friction: 0.5,
            color: [68, 88, 128],
            texture: FabricTexture::Twill,
        }
    }

    /// The built-in fabrics offered by the properties panel's picker, in
    /// display order.
    pub fn presets() -> [Fabric; 3] {
        [Self::cotton(), Self::silk(), Self::denim()]
    }
}

impl Default for Fabric {
    /// A medium-weight cotton, so an unassigned pattern still drapes
    /// plausibly.
    fn default() -> Self {
        Self::cotton()
    }
}

/// The sizes offered across a document, in small-to-large order, with one
/// marked as the base size that every pattern is drawn at and every
/// [`Grading`] rule is defined relative to (zero delta at the base size).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeChart {
    /// Size names, smallest to largest (e.g. `["S", "M", "L", "XL"]`).
    pub names: Vec<String>,
    /// Index into `names` of the size patterns are drawn at.
    pub base: usize,
}

impl SizeChart {
    /// The common four-size chart, based on M.
    pub fn standard() -> Self {
        Self {
            names: ["S", "M", "L", "XL"]
                .into_iter()
                .map(String::from)
                .collect(),
            base: 1,
        }
    }

    /// How many sizes `index` is from the base size (negative for smaller,
    /// positive for larger); the unit [`Grading`] deltas are scaled by.
    pub fn steps(&self, index: usize) -> i32 {
        index as i32 - self.base as i32
    }

    pub fn base_name(&self) -> &str {
        self.names.get(self.base).map_or("", String::as_str)
    }
}

impl Default for SizeChart {
    fn default() -> Self {
        Self::standard()
    }
}

/// A grading rule: how far each of a pattern's outline points moves per
/// size step away from the document's base size (see [`SizeChart::steps`]).
/// Grading one size up moves every point by its delta once; two sizes up,
/// twice; one size down, the opposite way. Empty (the default) means the
/// pattern is the same at every size.
///
/// Indices line up with `Pattern::outline`; a point added after grading was
/// set has no delta (treated as zero) until the rule is updated.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Grading {
    pub point_deltas: Vec<Vec2>,
}

impl Grading {
    /// A simple grading rule, proportional to size: every point moves
    /// `percent_per_step`% of the way from the pattern's centre to that
    /// point, for each size step. This is the standard shortcut for pieces
    /// without hand-graded rules — real grading can differ point to point
    /// (e.g. a collar grades less than a hem), which is why `point_deltas`
    /// is stored explicitly rather than just the percentage.
    pub fn uniform_scale(outline: &[OutlinePoint], percent_per_step: f32) -> Self {
        let center = geometry::bounds(&outline.iter().map(|p| p.pos).collect::<Vec<_>>())
            .map_or(Vec2::ZERO, |(min, max)| (min + max) * 0.5);
        let factor = percent_per_step / 100.0;
        Self {
            point_deltas: outline.iter().map(|p| (p.pos - center) * factor).collect(),
        }
    }

    /// How far the point of `pattern`'s outline at `pos` grades: the delta
    /// of the outline point there, or for a place between two, the blend
    /// of theirs. Zero without a grading rule.
    pub fn delta_at(pattern: &Pattern, pos: Vec2) -> Vec2 {
        let deltas = &pattern.grading.point_deltas;
        let n = pattern.outline.len();
        if deltas.is_empty() || n == 0 {
            return Vec2::ZERO;
        }
        let get = |k: usize| deltas.get(k % n).copied().unwrap_or(Vec2::ZERO);
        if let Some(k) = pattern
            .outline
            .iter()
            .position(|p| p.pos.distance(pos) < 1e-3)
        {
            return get(k);
        }
        let t = pattern.nearest_param(pos).0;
        get(t.floor() as usize).lerp(get(t.floor() as usize + 1), t - t.floor())
    }

    /// `before`'s grading carried over by position (see
    /// [`Grading::delta_at`]) to a new outline for it, after points were
    /// added or removed. No rule stays no rule.
    pub fn carried_over(before: &Pattern, outline: &[OutlinePoint]) -> Grading {
        if before.grading.point_deltas.is_empty() {
            return Grading::default();
        }
        Grading {
            point_deltas: outline
                .iter()
                .map(|p| Self::delta_at(before, p.pos))
                .collect(),
        }
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
    pub(crate) fn reflect(self, p: Vec2, about: f32) -> Vec2 {
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
    pub darts: Vec<Dart>,
    /// What this piece is cut from. Defaults to [`Fabric::default`] for a
    /// new pattern.
    pub fabric: Fabric,
    pub topstitches: Vec<Topstitch>,
    pub trims: Vec<Trim>,
    /// How this piece's outline changes across the document's size chart.
    pub grading: Grading,
    /// Target spacing between this piece's simulation particles, in mm.
    /// `None` uses the simulation's default; smaller is finer and slower.
    pub particle_distance: Option<f32>,
    /// Left out of both views and the drape.
    pub hidden: bool,
    /// Can be selected but not moved or edited.
    pub locked: bool,
    /// Where the piece starts in 3D before draping. `None` places it
    /// automatically, around the avatar.
    pub arrangement: Option<Arrangement>,
    /// Set for a linked copy of another pattern (symmetric editing).
    pub link: Option<Link>,
    /// Binding and piping along stretches of the outline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<crate::details::Binding>,
}

/// What makes a pattern a linked copy of another: its outline and
/// markings are always the source's, moved by a fixed transform (turned,
/// flipped and shifted the way the copy has been since). Editing the
/// source edits the copy; edits made through the copy go to the source.
///
/// The transform is kept as where the source's origin and unit axes land,
/// so moving, turning or flipping the copy updates it the same way it
/// moves the copy's points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub source: PatternId,
    pub origin: Vec2,
    pub x: Vec2,
    pub y: Vec2,
}

impl Link {
    /// A copy that sits exactly on its source.
    pub fn identity(source: PatternId) -> Self {
        Self {
            source,
            origin: Vec2::ZERO,
            x: Vec2::new(1.0, 0.0),
            y: Vec2::new(0.0, 1.0),
        }
    }

    /// Where a point of the source lands on the copy.
    pub fn apply(&self, p: Vec2) -> Vec2 {
        self.origin + (self.x - self.origin) * p.x + (self.y - self.origin) * p.y
    }

    /// Where a point of the copy came from on the source.
    pub fn to_source(&self, p: Vec2) -> Vec2 {
        let (a, b) = (self.x - self.origin, self.y - self.origin);
        let det = a.cross(b);
        if det.abs() < 1e-12 {
            return p;
        }
        let d = p - self.origin;
        Vec2::new(d.cross(b) / det, a.cross(d) / det)
    }

    /// Does the transform flip the source over?
    pub fn flips(&self) -> bool {
        (self.x - self.origin).cross(self.y - self.origin) < 0.0
    }

    /// A grainline angle (see [`Pattern::grain_angle`]) on the source, as
    /// it lies on the copy.
    pub fn grain_from_source(&self, degrees: f32) -> f32 {
        grain_through(degrees, |p| self.apply(p))
    }

    /// A grainline angle on the copy, as it lies on the source.
    pub fn grain_to_source(&self, degrees: f32) -> f32 {
        grain_through(degrees, |p| self.to_source(p))
    }

    fn map(&mut self, f: impl Fn(Vec2) -> Vec2) {
        self.origin = f(self.origin);
        self.x = f(self.x);
        self.y = f(self.y);
    }
}

/// A grainline angle's direction moved through the affine map `f`, as an
/// angle again.
fn grain_through(degrees: f32, f: impl Fn(Vec2) -> Vec2) -> f32 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let d = f(Vec2::new(-sin, cos)) - f(Vec2::ZERO);
    normalize_degrees((-d.x).atan2(d.y).to_degrees())
}

/// A pattern's plane in the 3D scene, in metres: where its centre is and
/// which ways its flat x (right) and y (up on screen) axes point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Arrangement {
    pub center: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
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
            darts: Vec::new(),
            fabric: Fabric::default(),
            topstitches: Vec::new(),
            trims: Vec::new(),
            grading: Grading::default(),
            particle_distance: None,
            hidden: false,
            locked: false,
            arrangement: None,
            link: None,
            bindings: Vec::new(),
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
        for line in &mut self.topstitches {
            for p in &mut line.points {
                *p = axis.reflect(*p, about);
            }
        }
        for trim in &mut self.trims {
            trim.pos = axis.reflect(trim.pos, about);
            trim.angle = match axis {
                Axis::Vertical => normalize_degrees(180.0 - trim.angle),
                Axis::Horizontal => normalize_degrees(-trim.angle),
            };
        }
        for dart in &mut self.darts {
            dart.map(|p| axis.reflect(p, about));
        }
        if let Some(link) = &mut self.link {
            link.map(|p| axis.reflect(p, about));
        }
        for d in &mut self.grading.point_deltas {
            *d = axis.reflect(*d, 0.0);
        }
        self.grain_angle = normalize_degrees(-self.grain_angle);
    }

    /// Turn everything `degrees` clockwise on screen about `about`. The
    /// grainline turns with it.
    pub fn rotate(&mut self, degrees: f32, about: Vec2) {
        let turn = |p: Vec2| rotate_point(p, degrees, about);
        self.outline.iter_mut().for_each(|p| p.pos = turn(p.pos));
        for line in &mut self.internal_lines {
            line.points.iter_mut().for_each(|p| *p = turn(*p));
        }
        for line in &mut self.topstitches {
            line.points.iter_mut().for_each(|p| *p = turn(*p));
        }
        for trim in &mut self.trims {
            trim.pos = turn(trim.pos);
            trim.angle = normalize_degrees(trim.angle + degrees);
        }
        for dart in &mut self.darts {
            dart.map(turn);
        }
        if let Some(link) = &mut self.link {
            link.map(turn);
        }
        for d in &mut self.grading.point_deltas {
            *d = rotate_point(*d, degrees, Vec2::ZERO);
        }
        self.grain_angle = normalize_degrees(self.grain_angle + degrees);
    }

    /// Move every point of the pattern and its markings with `f` (turning,
    /// flipping or shifting it), grading deltas with them. The grainline
    /// and notches are left as they are.
    pub fn map_points(&mut self, f: impl Fn(Vec2) -> Vec2) {
        for (d, p) in self.grading.point_deltas.iter_mut().zip(&self.outline) {
            *d = f(p.pos + *d) - f(p.pos);
        }
        self.outline.iter_mut().for_each(|p| p.pos = f(p.pos));
        for line in &mut self.internal_lines {
            line.points.iter_mut().for_each(|p| *p = f(*p));
        }
        for line in &mut self.topstitches {
            line.points.iter_mut().for_each(|p| *p = f(*p));
        }
        for trim in &mut self.trims {
            trim.map(&f);
        }
        for dart in &mut self.darts {
            dart.map(&f);
        }
        if let Some(link) = &mut self.link {
            link.map(&f);
        }
    }

    /// Move everything by `delta`.
    pub fn translate(&mut self, delta: Vec2) {
        self.outline.iter_mut().for_each(|p| p.pos += delta);
        for line in &mut self.internal_lines {
            line.points.iter_mut().for_each(|p| *p += delta);
        }
        for line in &mut self.topstitches {
            line.points.iter_mut().for_each(|p| *p += delta);
        }
        for trim in &mut self.trims {
            trim.pos += delta;
        }
        for dart in &mut self.darts {
            dart.map(|p| p + delta);
        }
        if let Some(link) = &mut self.link {
            link.map(|p| p + delta);
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
    pub fn segment_at(&self, param: f32) -> (f32, f32) {
        let n = self.outline.len();
        let corners = self.corner_indices();
        let Some(&base) = corners.first() else {
            return (0.0, n as f32);
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
            (next + n) as f32
        } else {
            next as f32
        };
        (best as f32, end)
    }

    /// The flattened outline's points, unwrapped into three laps (at
    /// `param - n`, `param`, `param + n`), sorted by parameter. A window
    /// anywhere within one lap of any outline parameter range is therefore
    /// contiguous in the result, regardless of where the outline itself
    /// happens to wrap.
    pub(crate) fn unwrapped_flat(&self, tolerance: f32) -> Vec<(f32, Vec2)> {
        let nf = self.outline.len() as f32;
        let mut pts: Vec<(f32, Vec2)> = self
            .flatten(tolerance)
            .into_iter()
            .flat_map(|f| [-nf, 0.0, nf].map(|shift| (f.param + shift, f.pos)))
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts
    }

    /// The position on the outline at outline parameter `param`, including
    /// fractions between flattened points (for a free-sewn edge that
    /// doesn't start or end exactly on one).
    pub fn pos_at_param(&self, param: f32, tolerance: f32) -> Vec2 {
        let pts = self.unwrapped_flat(tolerance);
        let Some(&(_, first)) = pts.first() else {
            return Vec2::ZERO;
        };
        let i = pts.partition_point(|&(p, _)| p <= param);
        if i == 0 {
            return first;
        }
        let Some(&(_, last)) = pts.last() else {
            return first;
        };
        if i >= pts.len() {
            return last;
        }
        let (p0, v0) = pts[i - 1];
        let (p1, v1) = pts[i];
        if p1 <= p0 {
            return v0;
        }
        v0.lerp(v1, (param - p0) / (p1 - p0))
    }

    /// The outline's points between outline parameters `start` and `end`
    /// (as returned by [`Pattern::segment_at`], or any sub-range for free
    /// sewing), in travel order, starting and ending exactly at `start` and
    /// `end` even if they fall between flattened points.
    pub fn segment_points(&self, start: f32, end: f32, tolerance: f32) -> Vec<Vec2> {
        if self.outline.is_empty() || end <= start {
            return Vec::new();
        }
        let mut out = vec![self.pos_at_param(start, tolerance)];
        out.extend(
            self.unwrapped_flat(tolerance)
                .into_iter()
                .filter(|&(p, _)| p > start + 1e-4 && p < end - 1e-4)
                .map(|(_, pos)| pos),
        );
        out.push(self.pos_at_param(end, tolerance));
        out
    }

    /// Length in mm of the outline between outline parameters `start` and
    /// `end`. Used to check that two sewn edges are a comparable length.
    pub fn segment_length(&self, start: f32, end: f32, tolerance: f32) -> f32 {
        self.segment_points(start, end, tolerance)
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum()
    }

    /// The outline graded `steps` sizes away from the document's base size
    /// (see [`SizeChart::steps`]): each point moved by its [`Grading`] delta
    /// that many times over. Unchanged at `steps == 0` or with no grading
    /// rule set; a point added since the rule was set moves by zero.
    pub fn graded_outline(&self, steps: i32) -> Vec<OutlinePoint> {
        if steps == 0 || self.grading.point_deltas.is_empty() {
            return self.outline.clone();
        }
        self.outline
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let delta = self
                    .grading
                    .point_deltas
                    .get(i)
                    .copied()
                    .unwrap_or(Vec2::ZERO);
                OutlinePoint {
                    pos: p.pos + delta * steps as f32,
                    kind: p.kind,
                }
            })
            .collect()
    }

    /// A copy of this pattern with its outline graded `steps` sizes away
    /// from the base size, for measuring or drawing a size other than the
    /// one it was drafted at without mutating the original.
    pub fn graded(&self, steps: i32) -> Pattern {
        Pattern {
            outline: self.graded_outline(steps),
            ..self.clone()
        }
    }
}

/// `p` turned `degrees` clockwise on screen (y points down) about `about`.
/// Quarter turns are exact, so four of them land back where they started.
pub fn rotate_point(p: Vec2, degrees: f32, about: Vec2) -> Vec2 {
    let quarter = degrees / 90.0;
    let (sin, cos) = if quarter == quarter.round() {
        match (quarter as i32).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        degrees.to_radians().sin_cos()
    };
    let d = p - about;
    about + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
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

/// Why a document read from a file was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invalid {
    DuplicatePattern(PatternId),
    /// A seam sews a pattern that doesn't exist, has an empty side, or has
    /// an edge that ends before it starts.
    BadSewingLine(SewingLineId),
    /// A tack holds a pattern that doesn't exist.
    BadTack(TackId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    pub patterns: Vec<Pattern>,
    pub sewing_lines: Vec<SewingLine>,
    pub tacks: Vec<Tack>,
    pub size_chart: SizeChart,
    /// Named colour variants of the garment (see [`crate::colourway`]).
    #[serde(default)]
    pub colourways: Vec<crate::colourway::Colourway>,
    /// The colourway last switched to, if any.
    #[serde(default)]
    pub active_colourway: Option<usize>,
    /// Named sets of pieces (see [`crate::workspace::Group`]).
    #[serde(default)]
    pub groups: Vec<crate::workspace::Group>,
    /// Saved 3D camera views.
    #[serde(default)]
    pub camera_views: Vec<crate::workspace::CameraView>,
    next_id: u64,
    next_sewing_id: u64,
    next_tack_id: u64,
    /// Named garment measurements (see [`crate::measure`]).
    #[serde(default)]
    pub measurements: Vec<crate::measure::Measurement>,
    #[serde(default)]
    pub(crate) next_measurement_id: u64,
    /// How the avatar stands (see [`crate::pose`]).
    #[serde(default)]
    pub avatar_pose: crate::pose::Pose,
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
            size_chart: SizeChart::default(),
            colourways: Vec::new(),
            active_colourway: None,
            groups: Vec::new(),
            camera_views: Vec::new(),
            next_id: 1,
            next_sewing_id: 1,
            next_tack_id: 1,
            measurements: Vec::new(),
            next_measurement_id: 0,
            avatar_pose: crate::pose::Pose::rest(),
        }
    }

    /// Check a document read from a file: ids must be unique, seams and
    /// tacks must hold patterns that exist, and new ids must not collide
    /// with existing ones even if the file says otherwise.
    pub(crate) fn validate(&mut self) -> Result<(), Invalid> {
        let mut ids: Vec<PatternId> = self.patterns.iter().map(|p| p.id).collect();
        ids.sort();
        if let Some(pair) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(Invalid::DuplicatePattern(pair[0]));
        }
        let known = |id: &PatternId| ids.binary_search(id).is_ok();
        for line in &self.sewing_lines {
            let ok = line.edges().all(|e| known(&e.pattern) && e.end > e.start);
            if !ok || line.a.is_empty() || line.b.is_empty() {
                return Err(Invalid::BadSewingLine(line.id));
            }
        }
        for tack in &self.tacks {
            if !tack.points().all(|p| known(&p.pattern)) {
                return Err(Invalid::BadTack(tack.id));
            }
        }
        // Groups forget pieces that aren't there.
        for group in &mut self.groups {
            group.members.retain(known);
        }
        // A linked copy of a pattern that isn't there is just a pattern.
        for pattern in &mut self.patterns {
            if pattern.link.is_some_and(|l| !known(&l.source)) {
                pattern.link = None;
            }
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
    pub fn sewing_line(&self, id: SewingLineId) -> Option<&SewingLine> {
        self.sewing_lines.iter().find(|s| s.id == id)
    }

    pub fn sewing_line_mut(&mut self, id: SewingLineId) -> Option<&mut SewingLine> {
        self.sewing_lines.iter_mut().find(|s| s.id == id)
    }

    pub fn tack(&self, id: TackId) -> Option<&Tack> {
        self.tacks.iter().find(|t| t.id == id)
    }

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

    /// Topmost shown pattern under `p` (later patterns draw on top).
    pub fn pattern_at(&self, p: Vec2) -> Option<PatternId> {
        self.patterns
            .iter()
            .rev()
            .find(|pat| !pat.hidden && pat.contains(p))
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
        p.internal_lines.push(InternalLine::new(vec![
            Vec2::new(10.0, 10.0),
            Vec2::new(20.0, 10.0),
        ]));
        p.topstitches.push(Topstitch {
            points: vec![Vec2::new(5.0, 5.0), Vec2::new(15.0, 5.0)],
        });
        p.trims.push(Trim {
            kind: TrimKind::Button,
            pos: Vec2::new(30.0, 10.0),
            angle: 0.0,
        });
        p.mirror(Axis::Vertical, 0.0);
        assert_eq!(p.outline[1].pos, Vec2::new(-100.0, 0.0));
        assert_eq!(p.outline[1].kind, PointKind::Curve);
        assert_eq!(p.internal_lines[0].points[1], Vec2::new(-20.0, 10.0));
        assert_eq!(p.topstitches[0].points[1], Vec2::new(-15.0, 5.0));
        assert_eq!(p.trims[0].pos, Vec2::new(-30.0, 10.0));
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
        assert_eq!(p.segment_at(0.0), (0.0, 2.0));
        assert_eq!(p.segment_at(0.5), (0.0, 2.0));
        assert_eq!(p.segment_at(1.9), (0.0, 2.0));
        // The last edge wraps past the end of the array.
        assert_eq!(p.segment_at(4.0), (4.0, 5.0));
        assert_eq!(p.segment_at(4.5), (4.0, 5.0));
    }

    #[test]
    fn segment_at_with_no_corners_is_the_whole_outline() {
        let p = pattern(&[(0.0, 0.0, Q), (100.0, 0.0, Q), (50.0, 50.0, Q)]);
        assert_eq!(p.segment_at(1.3), (0.0, 3.0));
    }

    #[test]
    fn segment_points_cover_only_the_requested_span() {
        let p = rect_with_curve();
        let straight = p.segment_points(2.0, 3.0, TOLERANCE);
        assert_eq!(
            straight,
            vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 50.0)]
        );
        // The curved span starts and ends at the same points as the plain
        // outline, with extra points for the bend in between.
        let curved = p.segment_points(0.0, 2.0, TOLERANCE);
        assert_eq!(curved.first(), Some(&Vec2::new(0.0, 0.0)));
        assert_eq!(curved.last(), Some(&Vec2::new(100.0, 0.0)));
        assert!(curved.len() > 2, "the curve should be subdivided");
        // A span that wraps past the end of the array still joins up.
        let wrapped = p.segment_points(4.0, 5.0, TOLERANCE);
        assert_eq!(wrapped, vec![Vec2::new(0.0, 50.0), Vec2::new(0.0, 0.0)]);
        // Free sewing: a sub-range that starts and ends mid-edge.
        let free = p.segment_points(2.25, 2.75, TOLERANCE);
        assert_eq!(free, vec![Vec2::new(100.0, 12.5), Vec2::new(100.0, 37.5)]);
    }

    #[test]
    fn segment_length_matches_a_straight_edge_and_exceeds_the_chord_on_a_curve() {
        let p = rect_with_curve();
        assert_eq!(p.segment_length(2.0, 3.0, TOLERANCE), 50.0);
        let curved = p.segment_length(0.0, 2.0, TOLERANCE);
        assert!(curved > 100.0, "{curved} should be longer than the chord");
        // Half of the 50 mm straight edge.
        assert_eq!(p.segment_length(2.0, 2.5, TOLERANCE), 25.0);
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

    fn edge(pattern: PatternId, start: f32, end: f32) -> SeamEdge {
        SeamEdge {
            pattern,
            start,
            end,
            forward: true,
        }
    }

    #[test]
    fn matching_edges_are_not_mismatched() {
        let doc = doc_with_two_rects([(100.0, 50.0), (100.0, 80.0)]);
        // The 100 mm sides of each rectangle (edge 0: index 0 to 1).
        let seam = SewingLine {
            id: SewingLineId(1),
            a: vec![edge(PatternId(1), 0.0, 1.0)],
            b: vec![edge(PatternId(2), 0.0, 1.0)],
            details: Default::default(),
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
            a: vec![edge(PatternId(1), 1.0, 2.0)],
            b: vec![edge(PatternId(2), 0.0, 1.0)],
            details: Default::default(),
        };
        assert_eq!(seam.length_mismatch(&doc), Some(-30.0));
        assert!(seam.is_length_mismatched(&doc));
    }

    #[test]
    fn mismatch_is_none_for_a_missing_pattern() {
        let doc = doc_with_two_rects([(100.0, 50.0), (100.0, 50.0)]);
        let seam = SewingLine {
            id: SewingLineId(1),
            a: vec![edge(PatternId(1), 0.0, 1.0)],
            b: vec![edge(PatternId(99), 0.0, 1.0)],
            details: Default::default(),
        };
        assert_eq!(seam.length_mismatch(&doc), None);
        assert!(!seam.is_length_mismatched(&doc));
    }

    #[test]
    fn size_chart_steps_are_relative_to_the_base() {
        let chart = SizeChart::standard();
        assert_eq!(chart.names, ["S", "M", "L", "XL"]);
        assert_eq!(chart.base_name(), "M");
        assert_eq!(chart.steps(0), -1);
        assert_eq!(chart.steps(1), 0);
        assert_eq!(chart.steps(3), 2);
    }

    #[test]
    fn ungraded_pattern_is_the_same_at_every_size() {
        let p = rect_with_curve();
        assert_eq!(p.graded_outline(2), p.outline);
        assert_eq!(p.graded_outline(-1), p.outline);
    }

    #[test]
    fn uniform_scale_grading_moves_points_away_from_centre() {
        let mut p = pattern(&[
            (0.0, 0.0, C),
            (100.0, 0.0, C),
            (100.0, 100.0, C),
            (0.0, 100.0, C),
        ]);
        p.grading = Grading::uniform_scale(&p.outline, 10.0);
        // Centre is (50, 50); the top-right corner is 50 away from it on
        // each axis, so one size up moves it 10% of that, outward.
        let bigger = p.graded_outline(1);
        assert_eq!(bigger[2].pos, Vec2::new(105.0, 105.0));
        // Two sizes up doubles the move; one size down reverses it.
        let biggest = p.graded_outline(2);
        assert_eq!(biggest[2].pos, Vec2::new(110.0, 110.0));
        let smaller = p.graded_outline(-1);
        assert_eq!(smaller[2].pos, Vec2::new(95.0, 95.0));
        // Grading scales the area up with size.
        let graded = p.graded(1);
        assert!(graded.area() > p.area());
    }

    #[test]
    fn grading_turns_and_flips_with_the_pattern() {
        let mut p = pattern(&[
            (0.0, 0.0, C),
            (100.0, 0.0, C),
            (100.0, 100.0, C),
            (0.0, 100.0, C),
        ]);
        p.grading = Grading::uniform_scale(&p.outline, 10.0);
        // Graded then turned is the same as turned then graded.
        let mut turned = p.clone();
        turned.rotate(30.0, Vec2::new(20.0, -5.0));
        let mut graded = p.graded(1);
        graded.rotate(30.0, Vec2::new(20.0, -5.0));
        let mut flipped = p.clone();
        flipped.mirror(Axis::Vertical, 140.0);
        let mut graded_flipped = p.graded(1);
        graded_flipped.mirror(Axis::Vertical, 140.0);
        for (a, b) in [(turned, graded), (flipped, graded_flipped)] {
            for (x, y) in a.graded_outline(1).iter().zip(&b.outline) {
                assert!(x.pos.distance(y.pos) < 1e-3, "{x:?} {y:?}");
            }
        }
        // A point added half way along an edge grades half way between.
        let (outline, _) = crate::edit::split_outline(&p.outline, 0.5).unwrap();
        let g = Grading::carried_over(&p, &outline);
        assert_eq!(g.point_deltas.len(), 5);
        assert_eq!(g.point_deltas[1], Vec2::new(0.0, -5.0));
        assert_eq!(g.point_deltas[2], Vec2::new(5.0, -5.0));
    }

    #[test]
    fn one_to_many_seam_sums_the_many_side() {
        // A 100 mm edge sewn to two 50 mm edges that together match it.
        let doc = doc_with_two_rects([(100.0, 50.0), (50.0, 20.0)]);
        let seam = SewingLine {
            id: SewingLineId(1),
            a: vec![edge(PatternId(1), 0.0, 1.0)],
            b: vec![edge(PatternId(2), 0.0, 1.0), edge(PatternId(2), 2.0, 3.0)],
            details: Default::default(),
        };
        assert_eq!(seam.length_mismatch(&doc), Some(0.0));
        assert!(!seam.is_length_mismatched(&doc));
    }
}
