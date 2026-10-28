//! Changing which pieces there are: cutting one in two along an internal
//! line, and merging two into one along the seam between them. Seams,
//! tacks and markings move to whichever new piece they lie on.

use crate::command::Command;
use crate::document::{
    rotate_point, Document, Grading, Notch, OutlinePoint, Pattern, PatternId, SeamEdge, SewingLine,
    SewingLineId, Tack, TOLERANCE,
};
use crate::edit::split_outline;
use crate::geometry::{self, Vec2};
use crate::seam::ahead;

/// How close (mm) a point must be to an outline to count as on it.
const ON_OUTLINE: f32 = 0.5;

/// Everything a cut or merge can change.
#[derive(Debug, Clone, PartialEq)]
struct State {
    patterns: Vec<Pattern>,
    sewing_lines: Vec<SewingLine>,
    tacks: Vec<Tack>,
}

impl State {
    fn of(doc: &Document) -> Self {
        Self {
            patterns: doc.patterns.clone(),
            sewing_lines: doc.sewing_lines.clone(),
            tacks: doc.tacks.clone(),
        }
    }

    fn put(self, doc: &mut Document) {
        doc.patterns = self.patterns;
        doc.sewing_lines = self.sewing_lines;
        doc.tacks = self.tacks;
    }
}

/// A command that works out a new [`State`] the first time it runs (ids
/// and all), then swaps between that and the state before.
struct Restructure {
    label: String,
    before: Option<State>,
    after: Option<State>,
}

impl Restructure {
    fn apply(&mut self, doc: &mut Document, make: impl FnOnce(&mut Document) -> Option<State>) {
        let before = State::of(doc);
        let after = match self.after.take() {
            Some(after) => Some(after),
            None => make(doc),
        };
        if let Some(after) = after {
            self.after = Some(after.clone());
            after.put(doc);
            self.before = Some(before);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            before.put(doc);
        }
    }
}

/// Cut a pattern in two along one of its internal lines, which must cross
/// the outline at least twice (the first two crossings along it are
/// used). The piece keeps its id for one half; the other half is new. With
/// `sew`, the two cut edges are sewn back together so the garment drapes
/// as before.
pub struct CutPattern {
    id: PatternId,
    line: usize,
    sew: bool,
    inner: Restructure,
}

impl CutPattern {
    pub fn new(id: PatternId, line: usize, sew: bool) -> Self {
        Self {
            id,
            line,
            sew,
            inner: Restructure {
                label: if sew {
                    "Cut and sew pattern".into()
                } else {
                    "Cut pattern".into()
                },
                before: None,
                after: None,
            },
        }
    }

    /// The new half's id, once the cut has been made.
    pub fn new_id(&self) -> Option<PatternId> {
        let after = self.inner.after.as_ref()?;
        let before = self.inner.before.as_ref()?;
        after
            .patterns
            .iter()
            .map(|p| p.id)
            .find(|id| before.patterns.iter().all(|p| p.id != *id))
    }
}

impl Command for CutPattern {
    fn label(&self) -> String {
        self.inner.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        let (id, line, sew) = (self.id, self.line, self.sew);
        self.inner.apply(doc, |doc| cut(doc, id, line, sew));
    }

    fn undo(&mut self, doc: &mut Document) {
        self.inner.undo(doc);
    }
}

/// Merge the two patterns a seam sews together into one, along that seam.
/// The seam must have one edge on each side, on different patterns. The
/// second pattern is turned (and flipped, if it would land on top of the
/// first) so its edge lies along the first's, and joins it there.
pub struct MergePatterns {
    seam: SewingLineId,
    inner: Restructure,
}

impl MergePatterns {
    pub fn new(seam: SewingLineId) -> Self {
        Self {
            seam,
            inner: Restructure {
                label: "Merge patterns".into(),
                before: None,
                after: None,
            },
        }
    }
}

impl Command for MergePatterns {
    fn label(&self) -> String {
        self.inner.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        let seam = self.seam;
        self.inner.apply(doc, |doc| merge(doc, seam));
    }

    fn undo(&mut self, doc: &mut Document) {
        self.inner.undo(doc);
    }
}

/// Can `seam` be merged along (see [`MergePatterns`])?
pub fn can_merge(doc: &Document, seam: SewingLineId) -> bool {
    let mut probe = doc.clone();
    merge(&mut probe, seam).is_some()
}

/// Can `pattern` be cut along its internal line `line` (see [`CutPattern`])?
pub fn can_cut(doc: &Document, id: PatternId, line: usize) -> bool {
    let mut probe = doc.clone();
    cut(&mut probe, id, line, false).is_some()
}

/// Is `p` on `pattern`'s outline?
fn on_outline(pattern: &Pattern, p: Vec2) -> bool {
    pattern.nearest_param(p).1.distance(p) <= ON_OUTLINE
}

/// The point half way along an open polyline.
fn halfway(points: &[Vec2]) -> Vec2 {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total * 0.5;
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if left <= len && len > 0.0 {
            return w[0].lerp(w[1], left / len);
        }
        left -= len;
    }
    points.first().copied().unwrap_or(Vec2::ZERO)
}

/// An edge of `target` sewn from `first` to `last` by way of `mid`, all
/// three on its outline. `None` if any of them is off it.
fn place_edge(target: &Pattern, first: Vec2, mid: Vec2, last: Vec2) -> Option<SeamEdge> {
    if ![first, mid, last].iter().all(|&p| on_outline(target, p)) {
        return None;
    }
    let n = target.outline.len() as f32;
    let (tf, tm, tl) = (
        target.nearest_param(first).0,
        target.nearest_param(mid).0,
        target.nearest_param(last).0,
    );
    let edge = if ahead(tf, tm, n) < ahead(tf, tl, n) {
        SeamEdge {
            pattern: target.id,
            start: tf,
            end: tf + ahead(tf, tl, n),
            forward: true,
        }
    } else {
        SeamEdge {
            pattern: target.id,
            start: tl,
            end: tl + ahead(tl, tf, n),
            forward: false,
        }
    };
    (edge.end - edge.start > 1e-4).then_some(edge)
}

/// `pattern`'s points from index `from` forwards to `to`, both included.
fn run(outline: &[OutlinePoint], from: usize, to: usize) -> Vec<OutlinePoint> {
    let n = outline.len();
    let count = (to + n - from) % n + 1;
    (0..count).map(|k| outline[(from + k) % n]).collect()
}

/// `pattern` with on-outline corner points at each of `points`, and their
/// indices in the new outline.
fn with_corners<const N: usize>(
    pattern: &Pattern,
    points: [Vec2; N],
) -> Option<(Pattern, [usize; N])> {
    let mut work = pattern.clone();
    for p in points {
        let (outline, _) = split_outline(&work.outline, work.nearest_param(p).0)?;
        work.outline = outline;
    }
    let mut indices = [0; N];
    for (slot, p) in indices.iter_mut().zip(points) {
        *slot = work
            .outline
            .iter()
            .enumerate()
            .filter(|(_, q)| q.kind == crate::document::PointKind::Corner)
            .min_by(|a, b| a.1.pos.distance(p).total_cmp(&b.1.pos.distance(p)))?
            .0;
    }
    Some((work, indices))
}

/// `notches` (on `from`'s outline) moved through `f` onto `to`'s outline,
/// leaving out any that land off it.
fn notches_onto(
    from: &Pattern,
    notches: &[Notch],
    f: impl Fn(Vec2) -> Vec2,
    to: &Pattern,
) -> Vec<Notch> {
    let (old, new) = (from.polyline(TOLERANCE), to.polyline(TOLERANCE));
    notches
        .iter()
        .filter_map(|n| {
            let (pos, _) = geometry::point_at_distance(&old, n.at)?;
            let (at, q) = geometry::distance_along(&new, f(pos))?;
            (q.distance(f(pos)) <= ON_OUTLINE).then_some(Notch { at })
        })
        .collect()
}

fn cut(doc: &mut Document, id: PatternId, line: usize, sew: bool) -> Option<State> {
    let p = doc.pattern(id)?.clone();
    let points = p.internal_lines.get(line)?.points.clone();
    let hits = p.crossings(&points);
    let (c1, c2) = (*hits.first()?, *hits.get(1)?);
    // The cut runs along the line from the first crossing to the second.
    let mut walked = 0.0;
    let mut path = Vec::new();
    for w in points.windows(2) {
        walked += w[0].distance(w[1]);
        if walked > c1.along + 1e-3 && walked < c2.along - 1e-3 {
            path.push(w[1]);
        }
    }

    let (split, [i1, i2]) = with_corners(&p, [c1.pos, c2.pos])?;
    if i1 == i2 {
        return None;
    }
    let corners =
        |pts: &mut dyn Iterator<Item = Vec2>| pts.map(OutlinePoint::corner).collect::<Vec<_>>();
    let mut a_outline = run(&split.outline, i1, i2);
    let a_cut = a_outline.len() - 1;
    a_outline.extend(corners(&mut path.iter().rev().copied()));
    let mut b_outline = run(&split.outline, i2, i1);
    let b_cut = b_outline.len() - 1;
    b_outline.extend(corners(&mut path.iter().copied()));
    if a_outline.len() < 3 || b_outline.len() < 3 {
        return None;
    }

    let blank = Pattern {
        notches: Vec::new(),
        internal_lines: Vec::new(),
        darts: Vec::new(),
        topstitches: Vec::new(),
        trims: Vec::new(),
        grading: Grading::default(),
        ..p.clone()
    };
    let mut a = Pattern {
        outline: a_outline,
        name: format!("{} A", p.name),
        ..blank.clone()
    };
    let mut b = Pattern {
        id: doc.allocate_id(),
        outline: b_outline,
        name: format!("{} B", p.name),
        arrangement: None,
        ..blank
    };
    if geometry::area(&a.polyline(TOLERANCE)) < 1.0 || geometry::area(&b.polyline(TOLERANCE)) < 1.0
    {
        return None;
    }
    // Markings go to the half they lie in.
    let (a_shape, b_shape) = (a.clone(), b.clone());
    let in_a = |q: Vec2| a_shape.contains(q) || !b_shape.contains(q);
    let half = |q: Vec2| if in_a(q) { 0 } else { 1 };
    let mut halves = [&mut a, &mut b];
    for (_, l) in p
        .internal_lines
        .iter()
        .enumerate()
        .filter(|&(k, _)| k != line)
    {
        halves[half(halfway(&l.points))]
            .internal_lines
            .push(l.clone());
    }
    for d in &p.darts {
        halves[half(d.start.lerp(d.end, 0.5))].darts.push(*d);
    }
    for t in &p.topstitches {
        halves[half(halfway(&t.points))].topstitches.push(t.clone());
    }
    for t in &p.trims {
        halves[half(t.pos)].trims.push(*t);
    }
    // A notch goes to the first half whose outline it lands on.
    let poly = p.polyline(TOLERANCE);
    for n in &p.notches {
        let Some((pos, _)) = geometry::point_at_distance(&poly, n.at) else {
            continue;
        };
        for piece in halves.iter_mut() {
            let outline = piece.polyline(TOLERANCE);
            if let Some((at, q)) = geometry::distance_along(&outline, pos) {
                if q.distance(pos) <= ON_OUTLINE {
                    piece.notches.push(Notch { at });
                    break;
                }
            }
        }
    }

    // Seams on the cut piece: onto whichever half each edge is on, split
    // in two where an edge runs across the cut.
    let mut sewing_lines = Vec::new();
    'lines: for line in &doc.sewing_lines {
        let mut moved = line.clone();
        for group in [&mut moved.a, &mut moved.b] {
            let mut out = Vec::new();
            for e in group.iter() {
                if e.pattern != id {
                    out.push(*e);
                    continue;
                }
                let pts = p.edge_points(e);
                let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else {
                    continue 'lines;
                };
                let mid = halfway(&pts);
                let whole = [&a, &b]
                    .into_iter()
                    .find_map(|piece| place_edge(piece, first, mid, last));
                if let Some(edge) = whole {
                    out.push(edge);
                    continue;
                }
                // Runs across the cut: split it where it meets a crossing.
                let on_edge = |c: Vec2| {
                    pts.windows(2)
                        .any(|w| geometry::closest_on_segment(w[0], w[1], c).0.distance(c) < 1e-2)
                };
                let Some(at) = [c1.pos, c2.pos].into_iter().find(|&c| on_edge(c)) else {
                    continue 'lines;
                };
                let k = pts
                    .windows(2)
                    .position(|w| {
                        geometry::closest_on_segment(w[0], w[1], at).0.distance(at) < 1e-2
                    })
                    .unwrap_or(0);
                let mut before: Vec<Vec2> = pts[..=k].to_vec();
                before.push(at);
                let mut after = vec![at];
                after.extend_from_slice(&pts[k + 1..]);
                let halves = [(&before, first, at), (&after, at, last)].map(|(run, f, l)| {
                    [&a, &b]
                        .into_iter()
                        .find_map(|piece| place_edge(piece, f, halfway(run), l))
                });
                match halves {
                    [Some(x), Some(y)] if x.pattern != y.pattern => out.extend([x, y]),
                    _ => continue 'lines,
                }
            }
            *group = out;
        }
        if !moved.a.is_empty() && !moved.b.is_empty() {
            sewing_lines.push(moved);
        }
    }
    if sew {
        let n = |piece: &Pattern| piece.outline.len() as f32;
        sewing_lines.push(SewingLine {
            id: doc.allocate_sewing_id(),
            // From the second crossing to the first on both halves.
            a: vec![SeamEdge {
                pattern: a.id,
                start: a_cut as f32,
                end: n(&a),
                forward: true,
            }],
            b: vec![SeamEdge {
                pattern: b.id,
                start: b_cut as f32,
                end: n(&b),
                forward: false,
            }],
        });
    }

    let mut tacks = doc.tacks.clone();
    for tack in &mut tacks {
        for point in tack.points_mut().filter(|t| t.pattern == id) {
            if !in_a(point.at) {
                point.pattern = b.id;
            }
        }
    }

    let mut patterns = doc.patterns.clone();
    let index = doc.index_of(id)?;
    patterns[index] = a;
    patterns.insert(index + 1, b);
    Some(State {
        patterns,
        sewing_lines,
        tacks,
    })
}

fn merge(doc: &mut Document, seam: SewingLineId) -> Option<State> {
    let line = doc.sewing_line(seam)?.clone();
    let ([ea], [eb]) = (line.a.as_slice(), line.b.as_slice()) else {
        return None;
    };
    if ea.pattern == eb.pattern {
        return None;
    }
    let (pa, pb) = (
        doc.pattern(ea.pattern)?.clone(),
        doc.pattern(eb.pattern)?.clone(),
    );
    if pa.locked || pb.locked {
        return None;
    }
    let ends = |p: &Pattern, e: &SeamEdge| {
        let pts = p.edge_points(e);
        Some((*pts.first()?, halfway(&pts), *pts.last()?))
    };
    let (a_first, _, a_last) = ends(&pa, ea)?;
    let (b_first, b_mid, b_last) = ends(&pb, eb)?;
    let (da, db) = (a_last - a_first, b_last - b_first);
    if da.length() < 1e-3 || db.length() < 1e-3 {
        return None;
    }

    // Turn B so its edge lies along A's, sewing start on sewing start; if
    // that puts it on top of A, flip it over the edge.
    let degrees = (da.y.atan2(da.x) - db.y.atan2(db.x)).to_degrees();
    let shift = a_first - b_first;
    let turn = move |q: Vec2| rotate_point(q, degrees, b_first) + shift;
    let u = da * (1.0 / da.length());
    let side = |q: Vec2| da.cross(q - a_first);
    let reflect = move |q: Vec2| {
        let r = q - a_first;
        a_first + u * (r.dot(u) * 2.0) - r
    };
    let flip = side(pa.center()) * side(turn(pb.center())) > 0.0;
    let place = move |q: Vec2| if flip { reflect(turn(q)) } else { turn(q) };

    let (a_start, a_end) = if ea.forward {
        (a_first, a_last)
    } else {
        (a_last, a_first)
    };
    let (sa, [i_s, i_e]) = with_corners(&pa, [a_start, a_end])?;
    let (sb, [j_first, j_last]) = with_corners(&pb, [b_first, b_last])?;
    let mut tb = sb.clone();
    tb.map_points(place);
    let mut index_of = [j_first, j_last];
    let area = |p: &Pattern| geometry::signed_area(&p.polyline(TOLERANCE));
    if area(&tb) * area(&sa) < 0.0 {
        // Keep both outlines going the same way round.
        tb.outline.reverse();
        let n = tb.outline.len();
        index_of = index_of.map(|j| n - 1 - j);
    }
    // B's points where A's edge starts and ends.
    let (j_s, j_e) = if ea.forward {
        (index_of[0], index_of[1])
    } else {
        (index_of[1], index_of[0])
    };
    // B's shared edge must be the run from j_e to j_s, so the rest of B
    // runs from j_s to j_e.
    let nb = tb.outline.len() as f32;
    let tm = tb.nearest_param(place(b_mid)).0;
    if ahead(j_e as f32, tm, nb) >= ahead(j_e as f32, j_s as f32, nb) {
        return None;
    }
    let mut outline = run(&sa.outline, i_e, i_s);
    let rest = run(&tb.outline, j_s, j_e);
    outline.extend_from_slice(&rest[1..rest.len() - 1]);
    if outline.len() < 3 {
        return None;
    }

    let mut merged = Pattern {
        outline,
        grading: Grading::default(),
        ..pa.clone()
    };
    merged
        .internal_lines
        .extend(tb.internal_lines.iter().cloned());
    merged.darts.extend(tb.darts.iter().copied());
    merged.topstitches.extend(tb.topstitches.iter().cloned());
    merged.trims.extend(tb.trims.iter().copied());
    let mut notches = notches_onto(&pa, &pa.notches, |q| q, &merged);
    notches.extend(notches_onto(&pb, &pb.notches, place, &merged));
    merged.notches = notches;

    let mut sewing_lines = Vec::new();
    'lines: for other in doc.sewing_lines.iter().filter(|l| l.id != seam) {
        let mut moved = other.clone();
        for e in moved.a.iter_mut().chain(moved.b.iter_mut()) {
            let (source, f): (&Pattern, &dyn Fn(Vec2) -> Vec2) = if e.pattern == pa.id {
                (&pa, &|q: Vec2| q)
            } else if e.pattern == pb.id {
                (&pb, &place)
            } else {
                continue;
            };
            let Some((first, mid, last)) = ends(source, e) else {
                continue 'lines;
            };
            match place_edge(&merged, f(first), f(mid), f(last)) {
                Some(edge) => *e = edge,
                None => continue 'lines,
            }
        }
        sewing_lines.push(moved);
    }

    let mut tacks = doc.tacks.clone();
    for tack in &mut tacks {
        for point in tack.points_mut().filter(|t| t.pattern == pb.id) {
            point.pattern = pa.id;
            point.at = place(point.at);
        }
    }

    let mut patterns = doc.patterns.clone();
    let index = doc.index_of(pa.id)?;
    patterns[index] = merged;
    patterns.retain(|p| p.id != pb.id);
    Some(State {
        patterns,
        sewing_lines,
        tacks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, AddSewingLine};
    use crate::document::{InternalLine, TackPoint, TackTarget};

    fn add_rect(doc: &mut Document, name: &str, min: Vec2, max: Vec2) -> PatternId {
        let mut add = AddPattern::new(name, geometry::rectangle(min, max));
        add.apply(doc);
        add.id().unwrap()
    }

    fn edge(pattern: PatternId, start: f32, end: f32, forward: bool) -> SeamEdge {
        SeamEdge {
            pattern,
            start,
            end,
            forward,
        }
    }

    fn sew(doc: &mut Document, a: SeamEdge, b: SeamEdge) -> SewingLineId {
        let mut add = AddSewingLine::new(vec![a], vec![b]);
        add.apply(doc);
        add.id().unwrap()
    }

    fn positions(p: &Pattern) -> Vec<Vec2> {
        p.positions().collect()
    }

    #[test]
    fn cutting_splits_a_piece_and_sews_the_halves() {
        let mut doc = Document::default();
        let id = add_rect(&mut doc, "Front", Vec2::ZERO, Vec2::new(200.0, 100.0));
        // A horizontal line right across, half way down.
        doc.pattern_mut(id)
            .unwrap()
            .internal_lines
            .push(InternalLine::new(vec![
                Vec2::new(-10.0, 50.0),
                Vec2::new(210.0, 50.0),
            ]));
        // A trim in the bottom half, and a tack on it.
        doc.pattern_mut(id)
            .unwrap()
            .trims
            .push(crate::document::Trim {
                kind: crate::document::TrimKind::Button,
                pos: Vec2::new(100.0, 80.0),
            });
        let other = add_rect(
            &mut doc,
            "Back",
            Vec2::new(300.0, 0.0),
            Vec2::new(500.0, 100.0),
        );
        let tack = doc.allocate_tack_id();
        doc.tacks.push(Tack {
            id: tack,
            a: TackPoint {
                pattern: id,
                at: Vec2::new(100.0, 80.0),
            },
            b: TackTarget::Pattern(TackPoint {
                pattern: other,
                at: Vec2::new(400.0, 50.0),
            }),
        });
        // The right side, sewn to the back's left side: it runs across
        // the cut, so it ends up as two edges.
        sew(
            &mut doc,
            edge(id, 1.0, 2.0, true),
            edge(other, 3.0, 4.0, false),
        );
        let before = doc.clone();

        let mut cut = CutPattern::new(id, 0, true);
        cut.apply(&mut doc);
        let b_id = cut.new_id().unwrap();
        assert_eq!(doc.patterns.len(), 3);
        let a = doc.pattern(id).unwrap();
        let b = doc.pattern(b_id).unwrap();
        assert!((a.area() - 10_000.0).abs() < 1.0, "{}", a.area());
        assert!((b.area() - 10_000.0).abs() < 1.0, "{}", b.area());
        assert!(a.internal_lines.is_empty() && b.internal_lines.is_empty());
        // The top half is A; the trim and tack were in the bottom half.
        assert!(a.contains(Vec2::new(100.0, 25.0)));
        assert_eq!(b.trims.len(), 1);
        assert_eq!(doc.tacks[0].a.pattern, b_id);

        assert_eq!(doc.sewing_lines.len(), 2);
        let side = &doc.sewing_lines[0];
        assert_eq!(side.a.len(), 2, "split at the cut: {side:?}");
        assert_eq!(side.length_mismatch(&doc), Some(0.0));
        let pts = |e: &SeamEdge| doc.pattern(e.pattern).unwrap().edge_points(e);
        // Still sewn from the top right corner down, across both halves.
        assert_eq!(pts(&side.a[0])[0], Vec2::new(200.0, 0.0));
        assert_eq!(pts(&side.a[1]).last(), Some(&Vec2::new(200.0, 100.0)));
        // The cut edges sewn together, untwisted: both start at the same end.
        let joined = &doc.sewing_lines[1];
        assert_eq!(pts(&joined.a[0])[0], pts(&joined.b[0])[0]);
        assert_eq!(joined.length_mismatch(&doc), Some(0.0));

        cut.undo(&mut doc);
        assert_eq!(doc.patterns, before.patterns);
        assert_eq!(doc.sewing_lines, before.sewing_lines);
        assert_eq!(doc.tacks, before.tacks);
        // Redo makes the same halves, ids and all.
        cut.apply(&mut doc);
        assert!(doc.pattern(b_id).is_some());
    }

    #[test]
    fn a_line_that_does_not_cross_twice_cannot_cut() {
        let mut doc = Document::default();
        let id = add_rect(&mut doc, "Front", Vec2::ZERO, Vec2::new(200.0, 100.0));
        doc.pattern_mut(id)
            .unwrap()
            .internal_lines
            .push(InternalLine::new(vec![
                Vec2::new(10.0, 50.0),
                Vec2::new(150.0, 50.0),
            ]));
        assert!(!can_cut(&doc, id, 0));
        let before = doc.clone();
        CutPattern::new(id, 0, false).apply(&mut doc);
        assert_eq!(doc, before);
    }

    #[test]
    fn merging_joins_two_pieces_along_their_seam() {
        let mut doc = Document::default();
        let front = add_rect(&mut doc, "Front", Vec2::ZERO, Vec2::new(100.0, 200.0));
        // The back is somewhere else, turned a quarter turn.
        let back = add_rect(
            &mut doc,
            "Back",
            Vec2::new(300.0, 0.0),
            Vec2::new(500.0, 100.0),
        );
        // Front's right side (100,0)→(100,200) sewn to the back's top edge
        // (300,0)→(500,0).
        let seam = sew(
            &mut doc,
            edge(front, 1.0, 2.0, true),
            edge(back, 0.0, 1.0, true),
        );
        // Another seam on the back's bottom edge, to a third piece.
        let third = add_rect(
            &mut doc,
            "Third",
            Vec2::new(0.0, 300.0),
            Vec2::new(200.0, 400.0),
        );
        sew(
            &mut doc,
            edge(back, 2.0, 3.0, true),
            edge(third, 0.0, 1.0, false),
        );
        assert!(can_merge(&doc, seam));
        let before = doc.clone();

        let mut merge = MergePatterns::new(seam);
        merge.apply(&mut doc);
        assert_eq!(doc.patterns.len(), 2);
        let merged = doc.pattern(front).unwrap();
        assert!(doc.pattern(back).is_none());
        // 100 × 200 plus 200 × 100, side by side.
        assert!((merged.area() - 40_000.0).abs() < 1.0, "{}", merged.area());
        assert_eq!(merged.outline.len(), 6, "{:?}", positions(merged));
        assert!(merged.contains(Vec2::new(150.0, 100.0)));
        // The other seam now sews the merged piece, the same length.
        assert_eq!(doc.sewing_lines.len(), 1);
        let rest = &doc.sewing_lines[0];
        assert_eq!(rest.a[0].pattern, front);
        assert_eq!(rest.length_mismatch(&doc), Some(0.0));

        merge.undo(&mut doc);
        assert_eq!(doc.patterns, before.patterns);
        assert_eq!(doc.sewing_lines, before.sewing_lines);
    }

    #[test]
    fn merging_flips_a_piece_that_would_land_on_top() {
        let mut doc = Document::default();
        let left = add_rect(&mut doc, "Left", Vec2::ZERO, Vec2::new(100.0, 100.0));
        let right = add_rect(
            &mut doc,
            "Right",
            Vec2::new(300.0, 0.0),
            Vec2::new(400.0, 100.0),
        );
        // Left's right side sewn top-down to Right's right side top-down:
        // laid along each other unflipped, Right would cover Left.
        let seam = sew(
            &mut doc,
            edge(left, 1.0, 2.0, true),
            edge(right, 1.0, 2.0, true),
        );
        MergePatterns::new(seam).apply(&mut doc);
        let merged = doc.pattern(left).unwrap();
        assert!((merged.area() - 20_000.0).abs() < 1.0, "{}", merged.area());
        assert!(merged.contains(Vec2::new(150.0, 50.0)));
        assert!(geometry::is_simple(&merged.polyline(TOLERANCE)));
    }
}
