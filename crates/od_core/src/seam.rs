//! Seam geometry on top of [`SeamEdge`] ranges: which way an edge is sewn,
//! finding outline parameters near a point, pairing points along the two
//! sides of a seam, and keeping edges on the same stretch of outline when
//! points are inserted or deleted.
//!
//! Outline parameters are as in [`crate::document::FlatPoint::param`]:
//! `i + f` is a fraction `f` of the way from outline point `i` to `i + 1`.

use crate::document::{Document, Pattern, PatternId, SeamEdge, SewingLine, TOLERANCE};
use crate::geometry::{self, Vec2};

/// Which side of a [`SewingLine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// Distance from parameter `a` forwards to `b` around an outline of `n`
/// points, in `0..n`.
pub(crate) fn ahead(a: f32, b: f32, n: f32) -> f32 {
    (b - a).rem_euclid(n)
}

/// Is `p` strictly between `a` and `b`, going forwards from `a`?
fn strictly_between(p: f32, a: f32, b: f32, n: f32) -> bool {
    let d = ahead(a, p, n);
    d > 0.0 && d < ahead(a, b, n)
}

impl SeamEdge {
    /// The same stretch sewn the other way.
    pub fn reversed(self) -> Self {
        Self {
            forward: !self.forward,
            ..self
        }
    }

    /// Outline parameter where sewing starts.
    pub fn first(&self) -> f32 {
        if self.forward {
            self.start
        } else {
            self.end
        }
    }

    /// Outline parameter where sewing ends.
    pub fn last(&self) -> f32 {
        if self.forward {
            self.end
        } else {
            self.start
        }
    }
}

impl Pattern {
    /// The edge's stretch of outline as an open polyline, in sewing order.
    pub fn edge_points(&self, edge: &SeamEdge) -> Vec<Vec2> {
        let mut points = self.segment_points(edge.start, edge.end, TOLERANCE);
        if !edge.forward {
            points.reverse();
        }
        points
    }

    /// Outline points from parameter `from` forwards to `to_unwrapped`
    /// (which may exceed the outline's length), with their parameters.
    fn run(&self, from: f32, to_unwrapped: f32) -> Vec<(f32, Vec2)> {
        let mut out = vec![(from, self.pos_at_param(from, TOLERANCE))];
        out.extend(
            self.unwrapped_flat(TOLERANCE)
                .into_iter()
                .filter(|&(p, _)| p > from + 1e-4 && p < to_unwrapped - 1e-4),
        );
        out.push((to_unwrapped, self.pos_at_param(to_unwrapped, TOLERANCE)));
        out
    }

    /// Nearest place to `p` on the run from `from` to `to_unwrapped`, as an
    /// outline parameter in `0..n` and the point there.
    fn nearest_on_run(&self, p: Vec2, from: f32, to_unwrapped: f32) -> (f32, Vec2) {
        let n = self.outline.len() as f32;
        let run = self.run(from, to_unwrapped);
        let mut best = (f32::MAX, from, run[0].1);
        for w in run.windows(2) {
            let (q, t) = geometry::closest_on_segment(w[0].1, w[1].1, p);
            let gap = q.distance(p);
            if gap < best.0 {
                best = (gap, w[0].0 + (w[1].0 - w[0].0) * t, q);
            }
        }
        (best.1.rem_euclid(n), best.2)
    }

    /// Outline parameter of the point on the outline nearest `p`, and that
    /// point.
    pub fn nearest_param(&self, p: Vec2) -> (f32, Vec2) {
        self.nearest_on_run(p, 0.0, self.outline.len() as f32)
    }

    /// Outline parameter nearest `p`, looking only forwards from `from` to
    /// `to` (both in `0..n`).
    pub fn nearest_param_between(&self, p: Vec2, from: f32, to: f32) -> f32 {
        let n = self.outline.len() as f32;
        self.nearest_on_run(p, from, from + ahead(from, to, n)).0
    }

    /// An edge sewn from parameter `from` to `to` the shorter way round the
    /// outline.
    pub fn shorter_edge(&self, from: f32, to: f32) -> SeamEdge {
        let n = self.outline.len() as f32;
        let fwd = ahead(from, to, n);
        let forward = SeamEdge {
            pattern: self.id,
            start: from,
            end: from + fwd,
            forward: true,
        };
        let backward = SeamEdge {
            pattern: self.id,
            start: to,
            end: to + (n - fwd),
            forward: false,
        };
        let len = |e: &SeamEdge| self.segment_length(e.start, e.end, TOLERANCE);
        if len(&forward) <= len(&backward) {
            forward
        } else {
            backward
        }
    }

    /// An edge from `from` to `to` the way an existing edge already runs:
    /// forwards around the outline if it is sewn forwards. Used when one
    /// end of an edge is dragged.
    pub fn edge_between(&self, from: f32, to: f32, forward: bool) -> SeamEdge {
        let n = self.outline.len() as f32;
        let (start, end) = if forward { (from, to) } else { (to, from) };
        let start = start.rem_euclid(n);
        SeamEdge {
            pattern: self.id,
            start,
            end: start + ahead(start, end, n),
            forward,
        }
    }
}

impl SewingLine {
    pub fn side(&self, side: Side) -> &[SeamEdge] {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }

    pub fn side_mut(&mut self, side: Side) -> &mut Vec<SeamEdge> {
        match side {
            Side::A => &mut self.a,
            Side::B => &mut self.b,
        }
    }

    pub fn edges(&self) -> impl Iterator<Item = &SeamEdge> {
        self.a.iter().chain(&self.b)
    }

    pub fn touches(&self, pattern: PatternId) -> bool {
        self.edges().any(|e| e.pattern == pattern)
    }
}

impl Document {
    /// The point a fraction `t` (0..=1) of the way along one side of a
    /// seam, by sewn length, with the pattern it lies on. Matching `t` on
    /// both sides gives points sewn together.
    pub fn side_point(&self, line: &SewingLine, side: Side, t: f32) -> Option<(PatternId, Vec2)> {
        let runs: Vec<(PatternId, Vec<Vec2>)> = line
            .side(side)
            .iter()
            .filter_map(|e| self.pattern(e.pattern).map(|p| (p.id, p.edge_points(e))))
            .collect();
        let length = |pts: &[Vec2]| pts.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
        let total: f32 = runs.iter().map(|(_, pts)| length(pts)).sum();
        let mut left = t.clamp(0.0, 1.0) * total;
        for (id, pts) in &runs {
            for w in pts.windows(2) {
                let len = w[0].distance(w[1]);
                if left <= len && len > 0.0 {
                    return Some((*id, w[0].lerp(w[1], left / len)));
                }
                left -= len;
            }
        }
        runs.last()
            .and_then(|(id, pts)| pts.last().map(|p| (*id, *p)))
    }
}

/// Where parameter `t` (in `0..n`) moves after a point was inserted before
/// `index`. Points at or after `index` move up by one; a parameter on the
/// edge that was split is placed again by position.
pub(crate) fn param_after_insert(before: &Pattern, after: &Pattern, index: usize, t: f32) -> f32 {
    let nb = before.outline.len() as f32;
    let na = after.outline.len() as f32;
    let k = index as f32;
    let prev = (k - 1.0).rem_euclid(nb);
    if strictly_between(t, prev, k.rem_euclid(nb), nb) {
        // The split edge runs from the old point before `index` to the old
        // point at `index`, which is now one further on.
        let from = if prev < k { prev } else { prev + 1.0 };
        let pos = before.pos_at_param(t, TOLERANCE);
        return after.nearest_param_between(pos, from, (k + 1.0).rem_euclid(na));
    }
    if t >= k {
        t + 1.0
    } else {
        t
    }
}

/// Where parameter `t` (in `0..n`) moves after the point at `index` was
/// removed. Parameters on the two edges that met there are placed again on
/// the edge that replaces them; later points move down by one.
pub(crate) fn param_after_delete(before: &Pattern, after: &Pattern, index: usize, t: f32) -> f32 {
    let nb = before.outline.len() as f32;
    let na = after.outline.len() as f32;
    let k = index as f32;
    let prev = (k - 1.0).rem_euclid(nb);
    let next = (k + 1.0).rem_euclid(nb);
    if t == k || strictly_between(t, prev, next, nb) {
        let pos = before.pos_at_param(t, TOLERANCE);
        let from = (k - 1.0).rem_euclid(na);
        return after.nearest_param_between(pos, from, k.rem_euclid(na));
    }
    if t > k {
        t - 1.0
    } else {
        t
    }
}

/// Move an edge's ends with `remap` (a parameter in `0..n` before, to one
/// after), keeping its direction. `None` if the edge shrank to nothing.
pub(crate) fn remap_edge(
    edge: &SeamEdge,
    before: &Pattern,
    after: &Pattern,
    remap: &impl Fn(&Pattern, &Pattern, f32) -> f32,
) -> Option<SeamEdge> {
    let nb = before.outline.len() as f32;
    let na = after.outline.len() as f32;
    let whole = (edge.end - edge.start - nb).abs() < 1e-4;
    let start = remap(before, after, edge.start.rem_euclid(nb));
    let end = remap(before, after, edge.end.rem_euclid(nb));
    let span = if whole { na } else { ahead(start, end, na) };
    (span > 1e-4).then_some(SeamEdge {
        start,
        end: start + span,
        ..*edge
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::OutlinePoint;
    use crate::geometry::rectangle;

    /// A 100 × 50 rectangle: points (0,0) (100,0) (100,50) (0,50).
    fn rect() -> Pattern {
        let outline = rectangle(Vec2::ZERO, Vec2::new(100.0, 50.0))
            .into_iter()
            .map(OutlinePoint::corner)
            .collect();
        Pattern::new(PatternId(1), "Panel", outline)
    }

    fn edge(start: f32, end: f32, forward: bool) -> SeamEdge {
        SeamEdge {
            pattern: PatternId(1),
            start,
            end,
            forward,
        }
    }

    #[test]
    fn edge_points_follow_the_sewing_direction() {
        let p = rect();
        let fwd = p.edge_points(&edge(1.0, 2.0, true));
        assert_eq!(fwd, [Vec2::new(100.0, 0.0), Vec2::new(100.0, 50.0)]);
        let back = p.edge_points(&edge(1.0, 2.0, false));
        assert_eq!(back, [Vec2::new(100.0, 50.0), Vec2::new(100.0, 0.0)]);
        // Wrapping past point 0.
        let wrap = p.edge_points(&edge(3.5, 4.5, true));
        assert_eq!(
            wrap,
            [Vec2::new(0.0, 25.0), Vec2::ZERO, Vec2::new(50.0, 0.0)]
        );
    }

    #[test]
    fn nearest_params() {
        let p = rect();
        let (t, q) = p.nearest_param(Vec2::new(-5.0, 10.0));
        assert!((t - 3.8).abs() < 1e-4, "{t}");
        assert_eq!(q, Vec2::new(0.0, 10.0));
        let t = p.nearest_param_between(Vec2::new(60.0, 50.0), 3.0, 0.0);
        assert!((t - 3.0).abs() < 1e-4, "{t}");
    }

    #[test]
    fn shorter_edge_goes_either_way_round() {
        let p = rect();
        let e = p.shorter_edge(0.5, 1.5);
        assert_eq!((e.start, e.end, e.forward), (0.5, 1.5, true));
        // From the middle of the bottom edge back past point 0.
        let e = p.shorter_edge(0.5, 3.5);
        assert_eq!((e.start, e.end, e.forward), (3.5, 4.5, false));
        assert_eq!(e.first(), 4.5);
        assert_eq!(p.edge_points(&e)[0], Vec2::new(50.0, 0.0));
    }

    #[test]
    fn edge_between_keeps_direction() {
        let p = rect();
        let e = p.edge_between(1.5, 2.0, true);
        assert_eq!((e.start, e.end), (1.5, 2.0));
        let e = p.edge_between(2.0, 1.5, false);
        assert_eq!((e.start, e.end, e.forward), (1.5, 2.0, false));
        assert_eq!(e.first(), 2.0);
    }

    #[test]
    fn params_follow_inserted_and_deleted_points() {
        let before = rect();
        let mut after = before.clone();
        after
            .outline
            .insert(1, OutlinePoint::corner(Vec2::new(50.0, 0.0)));
        assert_eq!(param_after_insert(&before, &after, 1, 1.0), 2.0);
        assert_eq!(param_after_insert(&before, &after, 1, 0.0), 0.0);
        let t = param_after_insert(&before, &after, 1, 0.75);
        assert!((t - 1.5).abs() < 1e-3, "{t}");
        // And back.
        let t = param_after_delete(&after, &before, 1, 1.5);
        assert!((t - 0.75).abs() < 1e-3, "{t}");
        assert_eq!(param_after_delete(&after, &before, 1, 2.0), 1.0);
    }

    #[test]
    fn remapped_edges_keep_direction_and_wrap() {
        let before = rect();
        let mut after = before.clone();
        after
            .outline
            .insert(0, OutlinePoint::corner(Vec2::new(0.0, 25.0)));
        let shift = |b: &Pattern, a: &Pattern, t| param_after_insert(b, a, 0, t);
        // Right side, sewn bottom to top.
        let e = remap_edge(&edge(1.0, 2.0, false), &before, &after, &shift).unwrap();
        assert_eq!((e.start, e.end, e.forward), (2.0, 3.0, false));
        // The whole outline stays the whole outline.
        let e = remap_edge(&edge(0.0, 4.0, true), &before, &after, &shift).unwrap();
        assert_eq!(e.end - e.start, 5.0);
    }
}
