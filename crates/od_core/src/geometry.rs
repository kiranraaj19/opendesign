//! Small 2D geometry helpers for pattern outlines. Units are millimetres.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }

    /// Z component of the 3D cross product: positive when `o` turns
    /// counter-clockwise from `self` (in y-up axes).
    pub fn cross(self, o: Vec2) -> f32 {
        self.x * o.y - self.y * o.x
    }

    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn distance(self, o: Vec2) -> f32 {
        (o - self).length()
    }

    /// Unit vector, or zero for a zero vector.
    pub fn normalized(self) -> Vec2 {
        let len = self.length();
        if len > 0.0 {
            self * (1.0 / len)
        } else {
            Vec2::ZERO
        }
    }

    /// Rotated a quarter turn: (x, y) → (-y, x).
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    pub fn lerp(self, o: Vec2, t: f32) -> Vec2 {
        self + (o - self) * t
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Vec2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// Axis-aligned bounds as `(min, max)`, or `None` for an empty outline.
pub fn bounds(points: &[Vec2]) -> Option<(Vec2, Vec2)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(min, max), p| {
        (
            Vec2::new(min.x.min(p.x), min.y.min(p.y)),
            Vec2::new(max.x.max(p.x), max.y.max(p.y)),
        )
    }))
}

/// Shoelace sum: positive when the points run counter-clockwise in y-up
/// axes, which is clockwise on screen (y down).
pub fn signed_area(points: &[Vec2]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut twice = 0.0;
    for (i, a) in points.iter().enumerate() {
        twice += a.cross(points[(i + 1) % points.len()]);
    }
    twice * 0.5
}

/// Unsigned polygon area.
pub fn area(points: &[Vec2]) -> f32 {
    signed_area(points).abs()
}

/// Total outline length, including the closing edge.
pub fn perimeter(points: &[Vec2]) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }
    points
        .iter()
        .enumerate()
        .map(|(i, a)| a.distance(points[(i + 1) % points.len()]))
        .sum()
}

/// Even-odd point-in-polygon test.
pub fn contains(points: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    let n = points.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (points[i], points[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Outline of an axis-aligned rectangle from two opposite corners.
pub fn rectangle(a: Vec2, b: Vec2) -> Vec<Vec2> {
    let (min, max) = (
        Vec2::new(a.x.min(b.x), a.y.min(b.y)),
        Vec2::new(a.x.max(b.x), a.y.max(b.y)),
    );
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

/// Closest point to `p` on the segment `a`–`b`, and its parameter `t` in 0..=1.
pub fn closest_on_segment(a: Vec2, b: Vec2, p: Vec2) -> (Vec2, f32) {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t, t)
}

/// Where segments `a`–`b` and `c`–`d` cross, if they do (touching counts).
pub fn segment_intersection(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    let (r, s) = (b - a, d - c);
    let denom = r.cross(s);
    if denom.abs() < 1e-9 {
        return None; // parallel or degenerate
    }
    let t = (c - a).cross(s) / denom;
    let u = (c - a).cross(r) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

/// Point at arc length `distance` along a closed outline, wrapping around,
/// with the unit direction of travel there.
pub fn point_at_distance(points: &[Vec2], distance: f32) -> Option<(Vec2, Vec2)> {
    let total = perimeter(points);
    if total <= 0.0 {
        return None;
    }
    let mut left = distance.rem_euclid(total);
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        let len = a.distance(b);
        if left <= len && len > 0.0 {
            return Some((a.lerp(b, left / len), (b - a).normalized()));
        }
        left -= len;
    }
    let (a, b) = (points[points.len() - 1], points[0]);
    Some((b, (b - a).normalized()))
}

/// Arc length along a closed outline to the point on it closest to `p`,
/// with that point.
pub fn distance_along(points: &[Vec2], p: Vec2) -> Option<(f32, Vec2)> {
    let mut best: Option<(f32, f32, Vec2)> = None; // (gap, distance along, point)
    let mut walked = 0.0;
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        let (q, t) = closest_on_segment(a, b, p);
        let gap = q.distance(p);
        let len = a.distance(b);
        if best.is_none_or(|(g, _, _)| gap < g) {
            best = Some((gap, walked + len * t, q));
        }
        walked += len;
    }
    best.map(|(_, d, q)| (d, q))
}

/// Split a simple polygon (convex or not) into triangles, as index triples
/// into `points`. Triangles cover the polygon exactly, with no overlap.
pub fn triangulate(points: &[Vec2]) -> Vec<[u32; 3]> {
    if points.len() < 3 {
        return Vec::new();
    }
    let flat: Vec<f64> = points
        .iter()
        .flat_map(|p| [f64::from(p.x), f64::from(p.y)])
        .collect();
    let indices = earcutr::earcut(&flat, &[], 2).unwrap_or_default();
    indices
        .chunks_exact(3)
        .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
        .collect()
}

/// How far a convex corner's miter may reach, in widths, before it is
/// bevelled off instead.
const MITER_LIMIT: f32 = 3.0;

/// Outline grown outwards by `width` (a seam allowance). Straight edges move
/// out in parallel; convex corners are mitered (bevelled when very sharp) and
/// concave corners meet where the offset edges cross. Loops that tight
/// concave curves would fold into are cut away.
pub fn offset_outline(points: &[Vec2], width: f32) -> Vec<Vec2> {
    let pts = dedup_closed(points);
    let n = pts.len();
    if n < 3 || width <= 0.0 {
        return Vec::new();
    }
    // With y-up winding, the outward normal of a counter-clockwise outline
    // is the edge direction turned clockwise.
    let ccw = signed_area(&pts) > 0.0;
    let outward = |d: Vec2| if ccw { -d.perp() } else { d.perp() };

    let mut raw = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (prev, cur, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let d0 = (cur - prev).normalized();
        let d1 = (next - cur).normalized();
        let (n0, n1) = (outward(d0), outward(d1));
        let turn = d0.cross(d1);
        let convex = if ccw { turn > 0.0 } else { turn < 0.0 };
        let bisector = (n0 + n1).normalized();
        let cos_half = bisector.dot(n0);
        if turn.abs() < 1e-6 || cos_half <= 1e-6 {
            // Straight through (or a full reversal): just push out.
            raw.push(cur + n0 * width);
            if cos_half <= 1e-6 {
                raw.push(cur + n1 * width);
            }
        } else if convex && 1.0 / cos_half > MITER_LIMIT {
            raw.push(cur + n0 * width);
            raw.push(cur + n1 * width);
        } else {
            // Miter: where the two offset edges meet.
            raw.push(cur + bisector * (width / cos_half));
        }
    }
    remove_loops(raw)
}

/// Drop consecutive duplicates, including last-equals-first.
fn dedup_closed(points: &[Vec2]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|q| q.distance(p) > 1e-4) {
            out.push(p);
        }
    }
    while out.len() > 1 && out[0].distance(out[out.len() - 1]) <= 1e-4 {
        out.pop();
    }
    out
}

/// Cut out self-intersection loops: when two non-adjacent edges cross, the
/// shorter way round between them is a fold and is replaced by the crossing.
fn remove_loops(mut pts: Vec<Vec2>) -> Vec<Vec2> {
    'restart: loop {
        let n = pts.len();
        if n < 4 {
            return pts;
        }
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            // Only look half way round, so the cut is always the short side.
            for k in 2..=n / 2 {
                let j = (i + k) % n;
                if (j + 1) % n == i {
                    continue; // adjacent through the wrap
                }
                let (c, d) = (pts[j], pts[(j + 1) % n]);
                if let Some(x) = segment_intersection(a, b, c, d) {
                    // Replace points i+1 ..= j with the crossing.
                    let mut out = Vec::with_capacity(n);
                    out.push(x);
                    let mut m = (j + 1) % n;
                    while m != (i + 1) % n {
                        out.push(pts[m]);
                        m = (m + 1) % n;
                    }
                    pts = out;
                    continue 'restart;
                }
            }
        }
        return pts;
    }
}

/// True when no two non-adjacent edges of the closed outline cross.
pub fn is_simple(points: &[Vec2]) -> bool {
    let n = points.len();
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (a, b) = (points[i], points[(i + 1) % n]);
            let (c, d) = (points[j], points[(j + 1) % n]);
            if segment_intersection(a, b, c, d).is_some() {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2> {
        rectangle(Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0))
    }

    /// An L: 100 × 100 with the top-right 50 × 50 quarter removed.
    fn l_shape() -> Vec<Vec2> {
        [
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 50.0),
            (100.0, 50.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ]
        .map(|(x, y)| Vec2::new(x, y))
        .to_vec()
    }

    fn assert_close(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn rectangle_measurements() {
        let sq = square();
        assert_eq!(area(&sq), 5000.0);
        assert_eq!(perimeter(&sq), 300.0);
        assert_eq!(
            bounds(&sq),
            Some((Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)))
        );
    }

    #[test]
    fn rectangle_normalises_corners() {
        assert_eq!(
            rectangle(Vec2::new(10.0, 10.0), Vec2::new(0.0, 0.0)),
            rectangle(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0))
        );
    }

    #[test]
    fn point_in_polygon() {
        let sq = square();
        assert!(contains(&sq, Vec2::new(50.0, 25.0)));
        assert!(!contains(&sq, Vec2::new(150.0, 25.0)));
        assert!(!contains(&[], Vec2::ZERO));
    }

    fn triangle_area(pts: &[Vec2], t: [u32; 3]) -> f32 {
        area(&t.map(|i| pts[i as usize]))
    }

    /// Triangles must add up to the polygon's area and each lie inside it.
    fn assert_exact_cover(pts: &[Vec2]) {
        let tris = triangulate(pts);
        assert_eq!(tris.len(), pts.len() - 2, "n-2 triangles for n points");
        let sum: f32 = tris.iter().map(|&t| triangle_area(pts, t)).sum();
        assert_close(sum, area(pts), area(pts) * 1e-4);
        for t in tris {
            let [a, b, c] = t.map(|i| pts[i as usize]);
            let centroid = (a + b + c) * (1.0 / 3.0);
            assert!(contains(pts, centroid), "triangle {t:?} lies outside");
        }
    }

    #[test]
    fn triangulates_concave_polygons_exactly() {
        assert_exact_cover(&square());
        assert_exact_cover(&l_shape());
        // A deep U, like a neckline: concave on both sides of the dip.
        let u = [
            (0.0, 0.0),
            (30.0, 0.0),
            (40.0, 60.0),
            (60.0, 60.0),
            (70.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ]
        .map(|(x, y)| Vec2::new(x, y));
        assert_exact_cover(&u);
        // The same outline wound the other way.
        let mut reversed = u.to_vec();
        reversed.reverse();
        assert_exact_cover(&reversed);
    }

    #[test]
    fn triangulate_degenerate_input_is_empty() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[Vec2::ZERO, Vec2::new(1.0, 0.0)]).is_empty());
    }

    #[test]
    fn offset_of_rectangle_grows_every_side() {
        for winding in [false, true] {
            let mut sq = square();
            if winding {
                sq.reverse();
            }
            let out = offset_outline(&sq, 10.0);
            assert_eq!(out.len(), 4);
            let (min, max) = bounds(&out).unwrap();
            assert_close(min.x, -10.0, 1e-3);
            assert_close(min.y, -10.0, 1e-3);
            assert_close(max.x, 110.0, 1e-3);
            assert_close(max.y, 60.0, 1e-3);
            assert_close(area(&out), 120.0 * 70.0, 0.1);
        }
    }

    #[test]
    fn offset_of_concave_corner_meets_cleanly() {
        let out = offset_outline(&l_shape(), 10.0);
        // The inner corner (50, 50) moves diagonally into the notch.
        assert!(out.iter().any(|p| p.distance(Vec2::new(60.0, 40.0)) < 1e-3));
        assert!(is_simple(&out));
        // Every corner is a right angle, so the miters are exact: a 120 mm
        // square minus the 50 mm notch that remains.
        assert_close(area(&out), 120.0 * 120.0 - 50.0 * 50.0, 0.5);
        for p in l_shape() {
            assert!(contains(&out, p + Vec2::new(0.01, 0.01)) || contains(&out, p));
        }
    }

    #[test]
    fn offset_cuts_away_loops_in_tight_concave_curves() {
        // A narrow slot 10 mm wide: a 20 mm allowance would fold over itself.
        let slot = [
            (0.0, 0.0),
            (45.0, 0.0),
            (45.0, 80.0),
            (55.0, 80.0),
            (55.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ]
        .map(|(x, y)| Vec2::new(x, y));
        let out = offset_outline(&slot, 20.0);
        assert!(is_simple(&out), "offset should not self-intersect: {out:?}");
        // Every original point stays inside the allowance.
        for p in slot {
            let probe = p.lerp(Vec2::new(50.0, 50.0), 0.001);
            assert!(contains(&out, probe), "{p:?} outside");
        }
    }

    #[test]
    fn walking_along_an_outline() {
        let sq = square();
        let (p, dir) = point_at_distance(&sq, 120.0).unwrap();
        assert_eq!(p, Vec2::new(100.0, 20.0));
        assert_eq!(dir, Vec2::new(0.0, 1.0));
        // Wraps past the start.
        assert_eq!(
            point_at_distance(&sq, 310.0).unwrap().0,
            Vec2::new(10.0, 0.0)
        );

        let (d, q) = distance_along(&sq, Vec2::new(103.0, 20.0)).unwrap();
        assert_close(d, 120.0, 1e-4);
        assert_eq!(q, Vec2::new(100.0, 20.0));
    }

    #[test]
    fn segments_cross_or_not() {
        let o = Vec2::ZERO;
        let x = segment_intersection(
            o,
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
            Vec2::new(10.0, 0.0),
        );
        assert_eq!(x, Some(Vec2::new(5.0, 5.0)));
        assert!(segment_intersection(
            o,
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0)
        )
        .is_none());
    }
}
