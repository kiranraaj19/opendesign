//! Builds an XPBD [`crate::xpbd::World`] from a garment: particles for each
//! pattern's flattened outline, structural constraints from its
//! triangulated mesh, and stitch constraints from
//! [`od_core::document::SewingLine`]s and [`Tack`]s. This is the "sewn
//! edges" input the Phase 6 roadmap note describes — the rest of the
//! pipeline (bending, pins, draping it over the avatar every frame) is
//! layered on top in [`crate::xpbd::World`] and [`crate::collision`].

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use od_core::document::{
    Document, FlatPoint, Pattern, PatternId, SeamEdge, TackTarget, TrimKind, TOLERANCE,
};
use od_core::geometry::{self, Vec2};
use od_render::pattern::Placement;

use crate::xpbd::{DistanceConstraint, Particle};

/// Tuning for the constraints [`build`] doesn't take from a pattern's
/// [`od_core::Fabric`]: just the seam, which stitches two sides that might
/// be cut from different fabrics, so it has no single one to read from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothConfig {
    pub stitch_compliance: f32,
    /// Floor under a particle's mass, in kg, so an unreasonably light
    /// fabric or a very finely subdivided pattern can't zero it out and
    /// make the solver divide by zero.
    pub min_particle_mass: f32,
}

impl Default for ClothConfig {
    fn default() -> Self {
        Self {
            stitch_compliance: 1e-7,
            min_particle_mass: 1e-6,
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
    /// Each placed [`od_core::document::Trim`], as the kind to draw and the
    /// global particle index nearest its flattened position — the same
    /// "snap to the nearest particle" idea [`Tack`]s use, since the only
    /// particles a trim can ride along with are the outline's. Draw it at
    /// that particle's current draped position.
    pub trims: Vec<(TrimKind, usize)>,
    /// Each placed [`od_core::document::Topstitch`], as the global
    /// particle index nearest each of its points, in order — draw a line
    /// through their current draped positions.
    pub topstitches: Vec<Vec<usize>>,
    /// Each [`Tack`], as the two particles it stitches: the cloth particle
    /// nearest its pattern point, and the one nearest its other pattern
    /// point or a fixed anchor particle on the avatar. Draw a thread
    /// between their current positions.
    pub tacks: Vec<(usize, usize)>,
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
        let mut points = points_in_range(&layout.flat, outline_len, edge.start, edge.end);
        // A backwards edge is sewn from its end to its start, so its
        // particles pair up with the other side's in that order.
        if !edge.forward {
            points.reverse();
        }
        for i in points {
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

/// Local index (into `flat`) closest to `target`, used the same way as
/// [`nearest_particle`] but before a pattern's [`PatternLayout`] is built
/// (trims and topstitches are mapped to particles while a pattern's
/// particles are still being laid out, not after).
fn nearest_flat_index(flat: &[FlatPoint], target: Vec2) -> usize {
    flat.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.pos.distance(target).total_cmp(&b.pos.distance(target)))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Build particles and constraints for every pattern in `placements`,
/// stitched together by `doc`'s sewing lines and tacks. Patterns in `doc`
/// not listed in `placements` are left out of the simulation.
pub fn build(
    doc: &Document,
    placements: &[(PatternId, Placement)],
    config: &ClothConfig,
) -> ClothScene {
    let mut particles = Vec::new();
    let mut structural_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut structural = Vec::new();
    let mut bending = Vec::new();
    let mut pattern_ranges = Vec::new();
    let mut pattern_faces = Vec::new();
    let mut trims = Vec::new();
    let mut topstitches = Vec::new();
    let mut layouts: HashMap<PatternId, PatternLayout> = HashMap::new();

    for (id, placement) in placements {
        let Some(pattern) = doc.pattern(*id) else {
            continue;
        };
        let flat = pattern.flatten(TOLERANCE);
        let poly_2d: Vec<Vec2> = flat.iter().map(|f| f.pos).collect();
        let glam_poly: Vec<glam::Vec2> = poly_2d.iter().copied().map(to_glam).collect();
        let world = placement.place(&glam_poly);

        // Spread the fabric's mass per square metre over however many
        // particles this pattern flattened to, so a bigger or more finely
        // subdivided piece doesn't end up lighter or heavier overall.
        let area_m2 = pattern.area() * 1e-6;
        let per_particle_mass = (area_m2 * pattern.fabric.mass_per_area
            / world.len().max(1) as f32)
            .max(config.min_particle_mass);
        let inv_mass = 1.0 / per_particle_mass;

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
                        pattern.fabric.stretch_compliance,
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
                pattern.fabric.bending_compliance,
            ));
        }
        pattern_faces.push((*id, faces));

        for trim in &pattern.trims {
            trims.push((trim.kind, offset + nearest_flat_index(&flat, trim.pos)));
        }
        for line in &pattern.topstitches {
            topstitches.push(
                line.points
                    .iter()
                    .map(|&p| offset + nearest_flat_index(&flat, p))
                    .collect(),
            );
        }

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

    // Particles only run along outlines, so a tacked point inside a
    // pattern is held by the outline particle nearest it.
    let mut tacks = Vec::new();
    for tack in &doc.tacks {
        let Some(la) = layouts.get(&tack.a.pattern) else {
            continue;
        };
        let ia = nearest_particle(la, tack.a.at);
        let ib = match tack.b {
            TackTarget::Pattern(b) => match layouts.get(&b.pattern) {
                Some(lb) => nearest_particle(lb, b.at),
                None => continue,
            },
            // A fixed particle on the avatar: it has no mass, so nothing
            // moves it and the stitch pulls the cloth to it.
            TackTarget::Avatar(at) => {
                particles.push(Particle::pinned(glam::Vec3::from_array(at)));
                particles.len() - 1
            }
        };
        if ia != ib {
            stitch_pairs.insert((ia.min(ib), ia.max(ib)));
            tacks.push((ia, ib));
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
        trims,
        topstitches,
        tacks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpbd::World;
    use od_core::document::{
        OutlinePoint, PatternId as Pid, SewingLine, SewingLineId, Tack, TackId, TackPoint,
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
        // Sew the front's right edge (corners 1->2, top down) to the back's
        // left edge (corners 3->0 run bottom up, so sewn backwards).
        doc.sewing_lines.push(SewingLine {
            id: SewingLineId(1),
            a: vec![SeamEdge {
                pattern: Pid(1),
                start: 1.0,
                end: 2.0,
                forward: true,
            }],
            b: vec![SeamEdge {
                pattern: Pid(2),
                start: 3.0,
                end: 4.0,
                forward: false,
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
            // Untwisted: both patterns hang at the same height on the ring,
            // so stitched particles start level with each other.
            let (ya, yb) = (
                scene.particles[s.a].position.y,
                scene.particles[s.b].position.y,
            );
            assert!((ya - yb).abs() < 0.02, "stitch {s:?} joins {ya} to {yb}");
        }
    }

    #[test]
    fn trims_and_topstitches_map_to_the_nearest_particle() {
        use od_core::document::{Topstitch, Trim};
        let mut doc = Document::new("test");
        let mut panel = rectangle(1, "front", 200.0, 300.0);
        panel.trims.push(Trim {
            kind: TrimKind::Button,
            // Near corner 1 (200.0, 0.0).
            pos: Vec2::new(195.0, 5.0),
        });
        panel.topstitches.push(Topstitch {
            // Near corners 0 and 1.
            points: vec![Vec2::new(5.0, 2.0), Vec2::new(195.0, 2.0)],
        });
        doc.patterns.push(panel);
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());

        assert_eq!(scene.trims.len(), 1);
        assert_eq!(scene.trims[0].0, TrimKind::Button);
        assert_eq!(scene.trims[0].1, 1, "closest particle is corner 1");

        assert_eq!(scene.topstitches.len(), 1);
        assert_eq!(scene.topstitches[0], vec![0, 1]);
    }

    #[test]
    fn tack_stitches_two_exact_points() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "front", 200.0, 300.0));
        doc.patterns.push(rectangle(2, "back", 200.0, 300.0));
        doc.tacks.push(Tack {
            id: TackId(1),
            a: TackPoint {
                pattern: Pid(1),
                at: Vec2::ZERO,
            },
            b: TackTarget::Pattern(TackPoint {
                pattern: Pid(2),
                at: Vec2::ZERO,
            }),
        });
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());
        assert_eq!(scene.stitches.len(), 1);
        assert_eq!(scene.stitches[0].rest_length, 0.0);
        assert_eq!(scene.tacks.len(), 1);
    }

    #[test]
    fn avatar_tack_stitches_the_cloth_to_a_fixed_anchor() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "front", 200.0, 300.0));
        let anchor = [0.0, 1.2, 0.05];
        doc.tacks.push(Tack {
            id: TackId(1),
            // Inside the pattern: held by the nearest outline particle.
            a: TackPoint {
                pattern: Pid(1),
                at: Vec2::new(195.0, 150.0),
            },
            b: TackTarget::Avatar(anchor),
        });
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, &ClothConfig::default());
        let cloth = scene.pattern_ranges[0].1.clone();
        assert_eq!(scene.particles.len(), cloth.end + 1, "one anchor particle");
        let fixed = &scene.particles[cloth.end];
        assert_eq!(fixed.inv_mass, 0.0);
        assert_eq!(fixed.position.to_array(), anchor);
        assert_eq!(scene.tacks, [(scene.tacks[0].0, cloth.end)]);
        assert!(cloth.contains(&scene.tacks[0].0));

        // Stepping pulls the tacked cloth towards the anchor; the anchor stays.
        let mut world = World::new(scene.particles.clone());
        world.distance_constraints = scene.structural.clone();
        world.distance_constraints.extend(scene.stitches.clone());
        let held = scene.tacks[0].0;
        let start = world.particles[held]
            .position
            .distance(glam::Vec3::from_array(anchor));
        for _ in 0..30 {
            world.step(1.0 / 60.0, 8, &[]);
        }
        let end = world.particles[held]
            .position
            .distance(glam::Vec3::from_array(anchor));
        assert!(end < start * 0.5, "{start} -> {end}");
        assert_eq!(world.particles[cloth.end].position.to_array(), anchor);
    }

    #[test]
    fn a_heavier_stiffer_fabric_gets_more_mass_and_less_compliance() {
        use od_core::document::Fabric;

        let scene_for = |fabric: Fabric| {
            let mut doc = Document::new("test");
            let mut panel = rectangle(1, "panel", 200.0, 300.0);
            panel.fabric = fabric;
            doc.patterns.push(panel);
            let placements = placements_for(&doc);
            build(&doc, &placements, &ClothConfig::default())
        };

        let silk = scene_for(Fabric::silk());
        let denim = scene_for(Fabric::denim());

        // Denim is both heavier per square metre and stiffer (lower
        // compliance) than silk, so its particles should end up heavier
        // and its structural constraints less compliant.
        assert!(
            denim.particles[0].inv_mass < silk.particles[0].inv_mass,
            "denim should be heavier: {:?} vs {:?}",
            denim.particles[0].inv_mass,
            silk.particles[0].inv_mass
        );
        assert!(denim.structural[0].compliance < silk.structural[0].compliance);
        assert!(denim.bending[0].compliance < silk.bending[0].compliance);
    }

    #[test]
    fn particle_mass_is_never_below_the_configured_floor() {
        use od_core::document::Fabric;
        let mut doc = Document::new("test");
        let mut panel = rectangle(1, "panel", 200.0, 300.0);
        panel.fabric = Fabric {
            mass_per_area: 0.0,
            ..Fabric::cotton()
        };
        doc.patterns.push(panel);
        let placements = placements_for(&doc);
        let config = ClothConfig {
            min_particle_mass: 0.002,
            ..ClothConfig::default()
        };
        let scene = build(&doc, &placements, &config);
        assert_eq!(scene.particles[0].inv_mass, 1.0 / 0.002);
    }
}
