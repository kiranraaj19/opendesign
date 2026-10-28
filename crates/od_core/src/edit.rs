//! Outline editing that changes more than one point at a time: splitting
//! an outline exactly at a parameter (curves included), rounding a corner,
//! setting an edge's length, and finding where a line crosses an outline.
//!
//! These work out a new outline; [`crate::command::SetOutline`] puts it in
//! the document, keeping seams and notches on the same stretch of outline.

use crate::document::{OutlinePoint, Pattern, PointKind, TOLERANCE};
use crate::geometry::{self, Vec2};

/// Closer than this (in outline parameter) to an existing on-curve point,
/// a split uses that point instead of adding one.
const PARAM_EPS: f32 = 1e-3;

/// `outline` with an on-curve corner point at outline parameter `t` (see
/// [`crate::document::FlatPoint::param`]), and that point's index. The
/// outline's shape is unchanged: a straight edge gets a point on it, and a
/// curve is split in two exactly (de Casteljau), with the curve points on
/// either side moved to keep the same curve. If `t` is already at a corner
/// point, the outline comes back as it was. `None` for fewer than three
/// points.
pub fn split_outline(outline: &[OutlinePoint], t: f32) -> Option<(Vec<OutlinePoint>, usize)> {
    let n = outline.len();
    if n < 3 {
        return None;
    }
    let t = t.rem_euclid(n as f32);
    let i = (t.floor() as usize).min(n - 1);
    let f = t - i as f32;
    let j = (i + 1) % n;
    let (ki, kj) = (outline[i].kind, outline[j].kind);
    if ki == PointKind::Corner && f < PARAM_EPS {
        return Some((outline.to_vec(), i));
    }
    if kj == PointKind::Corner && f > 1.0 - PARAM_EPS {
        return Some((outline.to_vec(), j));
    }
    // The curve point controlling the stretch `t` is on, and how far along
    // that quadratic `t` is (0 at its start, 1 at its end).
    let (c, u) = match (ki, kj) {
        (PointKind::Corner, PointKind::Corner) => {
            let mut out = outline.to_vec();
            let pos = outline[i].pos.lerp(outline[j].pos, f);
            out.insert(i + 1, OutlinePoint::corner(pos));
            return Some((out, i + 1));
        }
        (PointKind::Corner, PointKind::Curve) => (j, 0.5 * f),
        (PointKind::Curve, PointKind::Corner) => (i, 0.5 + 0.5 * f),
        (PointKind::Curve, PointKind::Curve) if f < 0.5 => (i, 0.5 + f),
        (PointKind::Curve, PointKind::Curve) => (j, f - 0.5),
    };
    let prev = outline[(c + n - 1) % n];
    let next = outline[(c + 1) % n];
    let ctrl = outline[c].pos;
    // Between two curve points the outline passes through their midpoint;
    // that point becomes a real corner so the split can't move it.
    let a_implied = prev.kind == PointKind::Curve;
    let b_implied = next.kind == PointKind::Curve;
    let a = if a_implied {
        prev.pos.lerp(ctrl, 0.5)
    } else {
        prev.pos
    };
    let b = if b_implied {
        next.pos.lerp(ctrl, 0.5)
    } else {
        next.pos
    };
    if !(PARAM_EPS..=1.0 - PARAM_EPS).contains(&u) {
        // At one end of the quadratic: that end becomes (or already is) the
        // corner.
        let at_start = u < PARAM_EPS;
        let implied = if at_start { a_implied } else { b_implied };
        if !implied {
            let index = if at_start {
                (c + n - 1) % n
            } else {
                (c + 1) % n
            };
            return Some((outline.to_vec(), index));
        }
        let (pos, at) = if at_start { (a, c) } else { (b, c + 1) };
        let mut out = outline.to_vec();
        out.insert(at, OutlinePoint::corner(pos));
        return Some((out, at));
    }
    let mut out = Vec::with_capacity(n + 4);
    out.extend_from_slice(&outline[..c]);
    if a_implied {
        out.push(OutlinePoint::corner(a));
    }
    let c1 = a.lerp(ctrl, u);
    let c2 = ctrl.lerp(b, u);
    out.push(OutlinePoint::curve(c1));
    let index = out.len();
    out.push(OutlinePoint::corner(c1.lerp(c2, u)));
    out.push(OutlinePoint::curve(c2));
    if b_implied {
        out.push(OutlinePoint::corner(b));
    }
    out.extend_from_slice(&outline[c + 1..]);
    Some((out, index))
}

/// Where an open polyline crosses a closed outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    /// Outline parameter of the crossing.
    pub param: f32,
    pub pos: Vec2,
    /// Distance along the polyline to the crossing, in mm.
    pub along: f32,
}

impl Pattern {
    /// Every place `line` crosses this pattern's outline, in order along
    /// the line.
    pub fn crossings(&self, line: &[Vec2]) -> Vec<Crossing> {
        let flat = self.flatten(TOLERANCE);
        let n = flat.len();
        let count = self.outline.len() as f32;
        let mut out: Vec<Crossing> = Vec::new();
        let mut walked = 0.0;
        for w in line.windows(2) {
            for k in 0..n {
                let (a, b) = (flat[k], flat[(k + 1) % n]);
                let Some(pos) = geometry::segment_intersection(w[0], w[1], a.pos, b.pos) else {
                    continue;
                };
                let mut end = b.param;
                if end <= a.param {
                    end += count;
                }
                let along_chord = a.pos.distance(b.pos);
                let s = if along_chord > 0.0 {
                    a.pos.distance(pos) / along_chord
                } else {
                    0.0
                };
                out.push(Crossing {
                    param: (a.param + (end - a.param) * s).rem_euclid(count),
                    pos,
                    along: walked + w[0].distance(pos),
                });
            }
            walked += w[0].distance(w[1]);
        }
        out.sort_by(|a, b| a.along.total_cmp(&b.along));
        // A line through a flattened point hits both chords meeting there.
        out.dedup_by(|a, b| a.pos.distance(b.pos) < 1e-3);
        out
    }
}

/// `pattern`'s outline with a corner point wherever `line` crosses it.
/// `None` if it doesn't cross, or every crossing is already a corner.
pub fn points_at_crossings(pattern: &Pattern, line: &[Vec2]) -> Option<Vec<OutlinePoint>> {
    let mut work = pattern.clone();
    let mut changed = false;
    for crossing in pattern.crossings(line) {
        // Earlier splits renumber the outline: find the crossing again.
        let (t, _) = work.nearest_param(crossing.pos);
        let (outline, _) = split_outline(&work.outline, t)?;
        if outline.len() != work.outline.len() {
            changed = true;
            work.outline = outline;
        }
    }
    changed.then_some(work.outline)
}

/// `outline` with corner `index` rounded off: the corner replaced by a
/// curve that leaves each neighbouring edge `distance` mm from the corner.
/// The distance is kept under half of either edge. `None` if `index` isn't
/// a corner or the corner is degenerate.
pub fn round_corner(
    outline: &[OutlinePoint],
    index: usize,
    distance: f32,
) -> Option<Vec<OutlinePoint>> {
    let n = outline.len();
    let corner = *outline.get(index).filter(|p| p.kind == PointKind::Corner)?;
    if n < 3 || distance <= 0.0 {
        return None;
    }
    let prev = outline[(index + n - 1) % n].pos;
    let next = outline[(index + 1) % n].pos;
    let (to_prev, to_next) = (prev - corner.pos, next - corner.pos);
    let (lp, ln) = (to_prev.length(), to_next.length());
    if lp < 1e-3 || ln < 1e-3 {
        return None;
    }
    let d = distance.min(lp * 0.5).min(ln * 0.5);
    let mut out = outline.to_vec();
    out.splice(
        index..=index,
        [
            OutlinePoint::corner(corner.pos + to_prev * (d / lp)),
            OutlinePoint::curve(corner.pos),
            OutlinePoint::corner(corner.pos + to_next * (d / ln)),
        ],
    );
    Some(out)
}

/// `pattern`'s outline with the corner-to-corner edge starting at corner
/// `index` made `length` mm long: the edge is scaled about its start, so
/// its end point moves along the same direction and a curved edge keeps
/// its shape. `None` if `index` isn't a corner or the edge has no length.
pub fn with_edge_length(pattern: &Pattern, index: usize, length: f32) -> Option<Vec<OutlinePoint>> {
    let n = pattern.outline.len();
    let start = *pattern
        .outline
        .get(index)
        .filter(|p| p.kind == PointKind::Corner)?;
    let (s, e) = pattern.segment_at(index as f32);
    if s as usize != index || length <= 0.0 {
        return None;
    }
    let current = pattern.segment_length(s, e, TOLERANCE);
    if current < 1e-3 {
        return None;
    }
    let k = length / current;
    let mut out = pattern.outline.clone();
    for i in s as usize + 1..=e as usize {
        let p = &mut out[i % n];
        p.pos = start.pos + (p.pos - start.pos) * k;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PatternId;

    fn pattern(outline: Vec<OutlinePoint>) -> Pattern {
        Pattern::new(PatternId(1), "Test", outline)
    }

    fn rect() -> Vec<OutlinePoint> {
        geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 50.0))
            .into_iter()
            .map(OutlinePoint::corner)
            .collect()
    }

    /// Every point of `b`'s outline lies on `a`'s, and the other way round.
    fn same_shape(a: &Pattern, b: &Pattern) {
        for (x, y) in [(a, b), (b, a)] {
            let poly = y.polyline(0.05);
            for p in x.polyline(0.05) {
                let gap = (0..poly.len())
                    .map(|k| {
                        let (q, _) =
                            geometry::closest_on_segment(poly[k], poly[(k + 1) % poly.len()], p);
                        q.distance(p)
                    })
                    .fold(f32::MAX, f32::min);
                assert!(gap < 0.1, "{p:?} is {gap} mm off");
            }
        }
    }

    #[test]
    fn split_a_straight_edge() {
        let (out, i) = split_outline(&rect(), 1.5).unwrap();
        assert_eq!(i, 2);
        assert_eq!(out.len(), 5);
        assert_eq!(out[2], OutlinePoint::corner(Vec2::new(100.0, 25.0)));
        // At an existing corner nothing changes.
        let (out, i) = split_outline(&rect(), 2.0).unwrap();
        assert_eq!((out.len(), i), (4, 2));
        // The closing edge.
        let (out, i) = split_outline(&rect(), 3.5).unwrap();
        assert_eq!(i, 4);
        assert_eq!(out[4].pos, Vec2::new(0.0, 25.0));
    }

    #[test]
    fn split_a_curve_keeps_its_shape() {
        let before = pattern(vec![
            OutlinePoint::corner(Vec2::new(0.0, 0.0)),
            OutlinePoint::curve(Vec2::new(50.0, -40.0)),
            OutlinePoint::corner(Vec2::new(100.0, 0.0)),
            OutlinePoint::corner(Vec2::new(100.0, 50.0)),
            OutlinePoint::curve(Vec2::new(50.0, 90.0)),
            OutlinePoint::curve(Vec2::new(0.0, 60.0)),
        ]);
        for t in [0.3, 0.9, 1.2, 4.2, 4.7, 5.3] {
            let (out, i) = split_outline(&before.outline, t).unwrap();
            let after = pattern(out);
            same_shape(&before, &after);
            assert_eq!(after.outline[i].kind, PointKind::Corner);
            let want = before.pos_at_param(t, 0.05);
            assert!(
                after.outline[i].pos.distance(want) < 0.5,
                "t {t}: {:?} vs {want:?}",
                after.outline[i].pos
            );
        }
    }

    #[test]
    fn split_at_an_implied_midpoint_makes_it_a_corner() {
        let before = pattern(vec![
            OutlinePoint::corner(Vec2::new(0.0, 0.0)),
            OutlinePoint::curve(Vec2::new(0.0, 100.0)),
            OutlinePoint::curve(Vec2::new(100.0, 100.0)),
            OutlinePoint::corner(Vec2::new(100.0, 0.0)),
        ]);
        let (out, i) = split_outline(&before.outline, 1.5).unwrap();
        assert_eq!(out.len(), 5);
        assert_eq!(out[i], OutlinePoint::corner(Vec2::new(50.0, 100.0)));
        same_shape(&before, &pattern(out));
    }

    #[test]
    fn crossings_in_order_along_the_line() {
        let p = pattern(rect());
        let c = p.crossings(&[Vec2::new(30.0, -10.0), Vec2::new(30.0, 60.0)]);
        assert_eq!(c.len(), 2);
        assert!((c[0].param - 0.3).abs() < 1e-4, "{c:?}");
        assert_eq!(c[0].pos, Vec2::new(30.0, 0.0));
        assert!((c[1].param - 2.7).abs() < 1e-4, "{c:?}");
        assert!((c[0].along - 10.0).abs() < 1e-4);
    }

    #[test]
    fn crossing_points_are_added_as_corners() {
        let p = pattern(rect());
        let out =
            points_at_crossings(&p, &[Vec2::new(30.0, -10.0), Vec2::new(30.0, 60.0)]).unwrap();
        assert_eq!(out.len(), 6);
        for want in [Vec2::new(30.0, 0.0), Vec2::new(30.0, 50.0)] {
            assert!(out.iter().any(|q| q.pos.distance(want) < 1e-3), "{out:?}");
        }
        // Through existing corners there is nothing to add.
        let diagonal = [Vec2::new(-10.0, -5.0), Vec2::new(110.0, 55.0)];
        assert_eq!(points_at_crossings(&p, &diagonal), None);
    }

    #[test]
    fn rounding_a_corner_cuts_it_with_a_curve() {
        let out = round_corner(&rect(), 1, 10.0).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(out[1], OutlinePoint::corner(Vec2::new(90.0, 0.0)));
        assert_eq!(out[2], OutlinePoint::curve(Vec2::new(100.0, 0.0)));
        assert_eq!(out[3], OutlinePoint::corner(Vec2::new(100.0, 10.0)));
        // Never more than half an edge.
        let out = round_corner(&rect(), 1, 500.0).unwrap();
        assert_eq!(out[3].pos, Vec2::new(100.0, 25.0));
        assert_eq!(round_corner(&out, 2, 5.0), None, "not a corner");
        let rounded = pattern(round_corner(&rect(), 1, 10.0).unwrap());
        assert!(rounded.area() < pattern(rect()).area());
    }

    #[test]
    fn edge_length_moves_the_end_along_the_edge() {
        let p = pattern(rect());
        let out = with_edge_length(&p, 1, 80.0).unwrap();
        assert_eq!(out[2].pos, Vec2::new(100.0, 80.0));
        assert_eq!(out[1].pos, Vec2::new(100.0, 0.0));
        // The closing edge moves point 0.
        let out = with_edge_length(&p, 3, 25.0).unwrap();
        assert_eq!(out[0].pos, Vec2::new(0.0, 25.0));
        // A curved edge is scaled, so its length comes out exact.
        let curved = pattern(vec![
            OutlinePoint::corner(Vec2::new(0.0, 0.0)),
            OutlinePoint::curve(Vec2::new(50.0, -40.0)),
            OutlinePoint::corner(Vec2::new(100.0, 0.0)),
            OutlinePoint::corner(Vec2::new(50.0, 80.0)),
        ]);
        let after = pattern(with_edge_length(&curved, 0, 150.0).unwrap());
        let len = after.segment_length(0.0, 2.0, TOLERANCE);
        assert!((len - 150.0).abs() < 0.5, "{len}");
        assert_eq!(with_edge_length(&curved, 1, 10.0), None);
    }
}
