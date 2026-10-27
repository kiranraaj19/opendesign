//! Builds an XPBD [`crate::xpbd::World`] from a garment: particles for each
//! pattern's flattened outline, structural constraints from its
//! triangulated mesh, and stitch constraints from
//! [`od_core::document::SewingLine`]s and [`Tack`]s. This is the "sewn
//! edges" input the Phase 6 roadmap note describes — the rest of the
//! pipeline (bending, pins, draping it over the avatar every frame) is
//! layered on top in [`crate::xpbd::World`] and [`crate::collision`].

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use od_core::document::{Document, FlatPoint, Pattern, PatternId, SeamEdge, TOLERANCE};
use od_core::geometry::{self, Vec2};
use od_render::pattern::Placement;

use crate::xpbd::{DistanceConstraint, Particle};

/// Tuning for how stiff the built constraints are. Compliance is XPBD's
/// inverse stiffness (m/N); `0.0` is rigid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothConfig {
    /// Mass of one particle, in kg. Uniform for now; a future pass could
    /// weight it by the Voronoi area the particle covers.
    pub particle_mass: f32,
    pub structural_compliance: f32,
    /// Compliance of the cross-edge constraint that resists folding at
    /// each interior triangle edge (see [`bending_pairs`]). Real cloth
    /// bends far more easily than it stretches, so this is softer than
    /// `structural_compliance`.
    pub bending_compliance: f32,
    pub stitch_compliance: f32,
}

impl Default for ClothConfig {
    fn default() -> Self {
        Self {
            particle_mass: 0.01,
            structural_compliance: 1e-7,
            bending_compliance: 1e-5,
            stitch_compliance: 1e-7,
        }
    }
}

/// A garment ready to simulate: particles and the constraints between
/// them, plus where each pattern's particles landed so a caller can read
/// draped positions back out.
#[derive(Debug, Clone)]
pub struct ClothScene {
    pub particles: Vec<Particle>,
    pub structural: Vec<DistanceConstraint>,
    pub bending: Vec<DistanceConstraint>,
    pub stitches: Vec<DistanceConstraint>,
    pub pattern_ranges: Vec<(PatternId, Range<usize>)>,
    /// Each pattern's triangulated faces, as indices local to its own
    /// range (subtract the range's start before indexing `particles`
    /// directly). What a renderer needs to turn draped particle positions
    /// back into a surface.
    pub pattern_faces: Vec<(PatternId, Vec<[u32; 3]>)>,
}

fn to_glam(v: Vec2) -> glam::Vec2 {
    glam::Vec2::new(v.x, v.y)
}

struct PatternLayout<'a> {
    pattern: &'a Pattern,
    offset: usize,
    flat: Vec<FlatPoint>,
}

/// Local indices (into `flat`) whose outline parameter falls in
/// `[start, end]`, travel-ordered, allowing for `end` wrapping past the
/// outline's length the way [`Pattern::segment_at`] produces.
fn points_in_range(flat: &[FlatPoint], outline_len: usize, start: f32, end: f32) -> Vec<usize> {
    let nf = outline_len as f32;
    let mut matches: Vec<(f32, usize)> = Vec::new();
    for (i, f) in flat.iter().enumerate() {
        for shift in [-nf, 0.0, nf] {
            let p = f.param + shift;
            if p >= start - 1e-4 && p <= end + 1e-4 {
                matches.push((p, i));
            }
        }
    }
    matches.sort_by(|a, b| a.0.total_cmp(&b.0));
    matches.dedup_by_key(|&mut (_, i)| i);
    matches.into_iter().map(|(_, i)| i).collect()
}

/// For each edge shared by exactly two of `faces` (i.e. not on the
/// boundary), the pair of vertices opposite it in those two triangles —
/// the vertices a bending constraint should span to resist folding along
/// that edge. Local indices, into whatever `faces` indexes.
fn bending_pairs(faces: &[[u32; 3]]) -> Vec<(u32, u32)> {
    let mut opposite: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for &[a, b, c] in faces {
        for (x, y, opp) in [(a, b, c), (b, c, a), (c, a, b)] {
            opposite.entry((x.min(y), x.max(y))).or_default().push(opp);
        }
    }
    opposite
        .into_values()
        .filter_map(|opps| match opps.as_slice() {
            [p, q] if p != q => Some((*p, *q)),
            _ => None,
        })
        .collect()
}

/// One side of a seam, as the particles it touches in travel order along
/// with each one's fraction of the way along the side's combined length.
fn side_chain(
    layouts: &HashMap<PatternId, PatternLayout>,
    group: &[SeamEdge],
) -> Vec<(usize, f32)> {
    let mut chain: Vec<(usize, Vec2)> = Vec::new();
    for edge in group {
        let Some(layout) = layouts.get(&edge.pattern) else {
            continue;
        };
        let outline_len = layout.pattern.outline.len();
        for i in points_in_range(&layout.flat, outline_len, edge.start, edge.end) {
            chain.push((layout.offset + i, layout.flat[i].pos));
        }
    }
    let mut cumulative = 0.0;
    let mut out = Vec::with_capacity(chain.len());
    for (i, &(idx, pos)) in chain.iter().enumerate() {
        if i > 0 {
            cumulative += chain[i - 1].1.distance(pos);
        }
        out.push((idx, cumulative));
    }
    let total = out.last().map_or(0.0, |&(_, c)| c);
    if total > 0.0 {
        for (_, frac) in &mut out {
            *frac /= total;
        }
    }
    out
}

/// The particle in `chain` closest to fraction `frac` along it. `chain`
/// must be sorted by fraction, ascending.
fn nearest_by_fraction(chain: &[(usize, f32)], frac: f32) -> Option<usize> {
    if chain.is_empty() {
        return None;
    }
    let i = chain.partition_point(|&(_, f)| f < frac);
    if i == 0 {
        return Some(chain[0].0);
    }
    if i >= chain.len() {
        return Some(chain[chain.len() - 1].0);
    }
    let (ia, fa) = chain[i - 1];
    let (ib, fb) = chain[i];
    Some(if (frac - fa).abs() <= (fb - frac).abs() {
        ia
    } else {
        ib
    })
}

/// The particle whose flattened position is closest to `target`, used to
/// map a [`Tack`]'s exact outline point (which may be a curve control
/// point not actually on the flattened outline) onto the nearest particle
/// that is.
fn nearest_particle(layout: &PatternLayout, target: Vec2) -> usize {
    layout
        .flat
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.pos.distance(target).total_cmp(&b.pos.distance(target)))
        .map(|(i, _)| layout.offset + i)
        .unwrap_or(layout.offset)
}

/// Build particles and constraints for every pattern in `placements`,
/// stitched together by `doc`'s sewing lines and tacks. Patterns in `doc`
/// not listed in `placements` are left out of the simulation.
pub fn build(
    doc: &Document,
    placements: &[(PatternId, Placement)],
    config: &ClothConfig,
) -> ClothScene {
    let inv_mass = 1.0 / config.particle_mass;

    let mut particles = Vec::new();
    let mut structural_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut structural = Vec::new();
    let mut bending = Vec::new();
    let mut pattern_ranges = Vec::new();
    let mut pattern_faces = Vec::new();
    let mut layouts: HashMap<PatternId, PatternLayout> = HashMap::new();

    for (id, placement) in placements {
        let Some(pattern) = doc.pattern(*id) else {
            continue;
        };
        let flat = pattern.flatten(TOLERANCE);
        let poly_2d: Vec<Vec2> = flat.iter().map(|f| f.pos).collect();
        let glam_poly: Vec<glam::Vec2> = poly_2d.iter().copied().map(to_glam).collect();
        let world = placement.place(&glam_poly);

        let offset = particles.len();
        particles.extend(world.iter().map(|&p| Particle::new(p, inv_mass)));
        pattern_ranges.push((*id, offset..particles.len()));

        let faces = geometry::triangulate(&poly_2d);
        for &[a, b, c] in &faces {
            for (local_a, local_b) in [(a, b), (b, c), (c, a)] {
                let (ga, gb) = (offset + local_a as usize, offset + local_b as usize);
                let key = (ga.min(gb), ga.max(gb));
                if structural_edges.insert(key) {
                    let rest_length = world[local_a as usize].distance(world[local_b as usize]);
                    structural.push(DistanceConstraint::new(
                        key.0,
                        key.1,
                        rest_length,
                        config.structural_compliance,
                    ));
                }
            }
        }
        for (p, q) in bending_pairs(&faces) {
            let (ga, gb) = (offset + p as usize, offset + q as usize);
            let rest_length = world[p as usize].distance(world[q as usize]);
            bending.push(DistanceConstraint::new(
                ga.min(gb),
                ga.max(gb),
                rest_length,
                config.bending_compliance,
            ));
        }
        pattern_faces.push((*id, faces));

        layouts.insert(
            *id,
            PatternLayout {
                pattern,
                offset,
                flat,
            },
        );
    }

    let mut stitch_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for seam in &doc.sewing_lines {
        let a_chain = side_chain(&layouts, &seam.a);
        let b_chain = side_chain(&layouts, &seam.b);
        for &(ia, fa) in &a_chain {
            if let Some(ib) = nearest_by_fraction(&b_chain, fa) {
                stitch_pairs.insert((ia.min(ib), ia.max(ib)));
            }
        }
        for &(ib, fb) in &b_chain {
            if let Some(ia) = nearest_by_fraction(&a_chain, fb) {
                stitch_pairs.insert((ia.min(ib), ia.max(ib)));
            }
        }
    }
    stitch_pairs.retain(|&(a, b)| a != b);

    for tack in &doc.tacks {
        if let (Some(la), Some(lb)) = (layouts.get(&tack.a.pattern), layouts.get(&tack.b.pattern)) {
            let pa = la.pattern.outline[tack.a.index].pos;
            let pb = lb.pattern.outline[tack.b.index].pos;
            let ia = nearest_particle(la, pa);
            let ib = nearest_particle(lb, pb);
            if ia != ib {
                stitch_pairs.insert((ia.min(ib), ia.max(ib)));
            }
        }
    }

    let stitches = stitch_pairs
        .into_iter()
        .map(|(a, b)| DistanceConstraint::new(a, b, 0.0, config.stitch_compliance))
        .collect();

    ClothScene {
        particles,
        structural,
        bending,
        stitches,
        pattern_ranges,
        pattern_faces,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpbd::World;
    use od_core::document::{
        OutlinePoint, PatternId as Pid, SeamPoint, SewingLine, SewingLineId, Tack, TackId,
    };
    use od_render::pattern::RingLayout;

    fn rectangle(id: u64, name: &str, w: f32, h: f32) -> Pattern {
        Pattern::new(
            Pid(id),
            name,
            vec![
                OutlinePoint::corner(Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(Vec2::new(w, 0.0)),
                OutlinePoint::corner(Vec2::new(w, h)),
                OutlinePoint::corner(Vec2::new(0.0, h)),
            ],
        )
    }

    fn placements_for(doc: &Document) -> Vec<(PatternId, Placement)> {
        let ring = RingLayout {
            radius: 0.5,
            height: 1.2,
        };
        doc.patterns
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, ring.placement(i, doc.patterns.len())))
            .collect()
    }

    #[test]
    fn one_pattern_builds_a_triangulated_mesh() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "panel", 200.0, 300.0));
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());
        assert_eq!(scene.particles.len(), 4);
        assert_eq!(scene.pattern_faces.len(), 1);
        assert_eq!(scene.pattern_faces[0].1.len(), 2, "a quad is 2 triangles");
        // A quad triangulates into 2 triangles sharing a diagonal: 5 unique edges.
        assert_eq!(scene.structural.len(), 5);
        // One bending constraint, across the diagonal, between the quad's
        // other two corners.
        assert_eq!(scene.bending.len(), 1);
        assert!(scene.stitches.is_empty());
    }

    #[test]
    fn bending_constraint_resists_folding() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "panel", 300.0, 300.0));
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());

        // Pin one edge (corners 0, 1) and let gravity fold the rest down;
        // measure how far the opposite corner (3) sags after settling.
        let sag = |include_bending: bool| -> f32 {
            let mut world = World::new(scene.particles.clone());
            world.distance_constraints = scene.structural.clone();
            if include_bending {
                world.distance_constraints.extend(scene.bending.clone());
            }
            world.particles[0].inv_mass = 0.0;
            world.particles[1].inv_mass = 0.0;
            for _ in 0..180 {
                world.step(1.0 / 60.0, 8, &[]);
            }
            scene.particles[3].position.y - world.particles[3].position.y
        };

        let with_bending = sag(true);
        let without_bending = sag(false);
        assert!(
            with_bending < without_bending,
            "bending should resist folding: {with_bending} vs {without_bending}"
        );
    }

    #[test]
    fn bending_pairs_span_the_quads_other_diagonal() {
        // Earcut splits a convex quad along one diagonal into two
        // triangles; the two corners NOT on that diagonal are the pair a
        // bending constraint should connect.
        let pairs = bending_pairs(&[[0, 1, 2], [0, 2, 3]]);
        assert_eq!(pairs, vec![(1, 3)]);
    }

    #[test]
    fn sewing_line_stitches_two_patterns_together() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "front", 200.0, 300.0));
        doc.patterns.push(rectangle(2, "back", 200.0, 300.0));
        // Sew the front's right edge (corners 1->2) to the back's left edge (corners 3->0).
        doc.sewing_lines.push(SewingLine {
            id: SewingLineId(1),
            a: vec![SeamEdge {
                pattern: Pid(1),
                start: 1.0,
                end: 2.0,
            }],
            b: vec![SeamEdge {
                pattern: Pid(2),
                start: 3.0,
                end: 4.0,
            }],
        });
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());
        assert!(!scene.stitches.is_empty());
        // Every stitch constraint should cross between the two patterns' ranges.
        let (front_range, back_range) = (
            scene.pattern_ranges[0].1.clone(),
            scene.pattern_ranges[1].1.clone(),
        );
        for s in &scene.stitches {
            let in_front = front_range.contains(&s.a) || front_range.contains(&s.b);
            let in_back = back_range.contains(&s.a) || back_range.contains(&s.b);
            assert!(in_front && in_back, "stitch {s:?} should cross patterns");
            assert_eq!(s.rest_length, 0.0);
        }
    }

    #[test]
    fn tack_stitches_two_exact_points() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "front", 200.0, 300.0));
        doc.patterns.push(rectangle(2, "back", 200.0, 300.0));
        doc.tacks.push(Tack {
            id: TackId(1),
            a: SeamPoint {
                pattern: Pid(1),
                index: 0,
            },
            b: SeamPoint {
                pattern: Pid(2),
                index: 0,
            },
        });
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());
        assert_eq!(scene.stitches.len(), 1);
        assert_eq!(scene.stitches[0].rest_length, 0.0);
    }
}
