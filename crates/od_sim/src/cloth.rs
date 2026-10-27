//! Builds an XPBD [`crate::xpbd::World`] from a garment: each pattern
//! meshed by [`crate::mesher`] (outline plus interior particles about the
//! particle distance apart), structural and bending constraints from that
//! mesh, and stitch constraints from [`od_core::document::SewingLine`]s
//! and [`od_core::document::Tack`]s. The rest of the pipeline (pins,
//! draping it over the avatar every frame) is layered on top in
//! [`crate::xpbd::World`] and [`crate::collision`].

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use od_core::document::{
    Document, FlatPoint, Pattern, PatternId, SeamEdge, TackTarget, TrimKind, TOLERANCE,
};
use od_core::geometry::{self, Vec2};
use od_render::mesh::Mesh;
use od_render::pattern::Placement;

use crate::mesher::{self, PatternMesh};
use crate::xpbd::{DistanceConstraint, Particle, SurfacePoint, SurfaceStitch};

/// Particle spacing for a pattern that doesn't set its own, in mm.
pub const DEFAULT_PARTICLE_DISTANCE: f32 = 20.0;

/// Tuning for the constraints [`build`] doesn't take from a pattern's
/// [`od_core::Fabric`]: the seam, which stitches two sides that might be
/// cut from different fabrics, so it has no single one to read from; and
/// how finely patterns are meshed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothConfig {
    pub stitch_compliance: f32,
    /// Floor under a particle's mass, in kg, so an unreasonably light
    /// fabric or a very finely subdivided pattern can't zero it out and
    /// make the solver divide by zero.
    pub min_particle_mass: f32,
    /// Spacing between particles, in mm, for a pattern without its own
    /// [`Pattern::particle_distance`].
    pub particle_distance: f32,
    /// Most interior particles one pattern may get, however fine its
    /// particle distance: keeps a huge piece from stalling the solver.
    pub max_interior_particles: usize,
}

impl Default for ClothConfig {
    fn default() -> Self {
        Self {
            stitch_compliance: 1e-7,
            min_particle_mass: 1e-6,
            particle_distance: DEFAULT_PARTICLE_DISTANCE,
            max_interior_particles: 4000,
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
    /// Each pattern's mesh, in its own mm coordinates, with indices local
    /// to its range (add the range's start to index `particles`). Faces
    /// turn draped positions back into a surface; vertex positions map a
    /// draped point back to the flat pattern.
    pub pattern_meshes: Vec<(PatternId, PatternMesh)>,
    /// Each placed [`od_core::document::Trim`], as the kind to draw and
    /// where on the cloth it rides.
    pub trims: Vec<(TrimKind, SurfacePoint)>,
    /// Each placed [`od_core::document::Topstitch`], resampled to about
    /// the particle distance, as points on the cloth in order: draw a line
    /// through their current draped positions.
    pub topstitches: Vec<Vec<SurfacePoint>>,
    /// Each tack the scene could place, by its index in `doc.tacks`: the
    /// two points it holds together, the second either on the cloth or at
    /// a fixed anchor particle on the avatar.
    pub tacks: Vec<(usize, SurfaceStitch)>,
}

fn to_glam(v: Vec2) -> glam::Vec2 {
    glam::Vec2::new(v.x, v.y)
}

/// Where a pattern's mesh landed in the scene.
struct PatternLayout<'a> {
    pattern: &'a Pattern,
    offset: usize,
    mesh: PatternMesh,
    /// The mesh's outline vertices with their outline parameters, as local
    /// vertex index and flat point.
    boundary: Vec<(usize, FlatPoint)>,
}

/// The cloth point at flat position `at` (mm, in the pattern's space) of
/// a pattern meshed as `mesh` whose particles start at `offset`.
pub fn surface_point(mesh: &PatternMesh, offset: usize, at: Vec2) -> Option<SurfacePoint> {
    let (face, weights) = mesh.locate(at)?;
    let [a, b, c] = mesh.faces[face];
    Some(SurfacePoint {
        particles: [a, b, c].map(|v| offset + v as usize),
        weights,
    })
}

impl PatternLayout<'_> {
    fn surface_point(&self, at: Vec2) -> Option<SurfacePoint> {
        surface_point(&self.mesh, self.offset, at)
    }
}

/// A pattern meshed with just its flattened outline, the way every
/// pattern was before interior particles: for outlines the mesher can't
/// handle.
fn outline_mesh(flat: &[FlatPoint]) -> PatternMesh {
    let poly: Vec<Vec2> = flat.iter().map(|f| f.pos).collect();
    PatternMesh {
        vertices: flat
            .iter()
            .map(|f| mesher::MeshVertex {
                pos: f.pos,
                param: Some(f.param),
            })
            .collect(),
        faces: geometry::triangulate(&poly),
        boundary: (0..flat.len() as u32).collect(),
    }
}

/// The particle spacing `pattern` simulates at, in mm.
pub fn particle_distance(pattern: &Pattern, config: &ClothConfig) -> f32 {
    pattern
        .particle_distance
        .unwrap_or(config.particle_distance)
        .max(1.0)
}

/// Mesh `pattern` at its particle distance, or `config`'s default.
pub fn mesh_for(pattern: &Pattern, config: &ClothConfig) -> PatternMesh {
    let flat = pattern.flatten(TOLERANCE);
    let lines: Vec<Vec<Vec2>> = pattern
        .internal_lines
        .iter()
        .map(|l| l.points.clone())
        .collect();
    mesher::mesh_pattern(
        &flat,
        pattern.outline.len(),
        &lines,
        particle_distance(pattern, config),
        config.max_interior_particles,
    )
    .unwrap_or_else(|| outline_mesh(&flat))
}

/// Outline vertices (local index and position) whose outline parameter
/// falls in `[start, end]`, travel-ordered, allowing for `end` wrapping
/// past the outline's length the way [`Pattern::segment_at`] produces.
fn points_in_range(
    boundary: &[(usize, FlatPoint)],
    outline_len: usize,
    start: f32,
    end: f32,
) -> Vec<(usize, Vec2)> {
    let nf = outline_len as f32;
    let mut matches: Vec<(f32, usize, Vec2)> = Vec::new();
    for &(i, f) in boundary {
        for shift in [-nf, 0.0, nf] {
            let p = f.param + shift;
            if p >= start - 1e-4 && p <= end + 1e-4 {
                matches.push((p, i, f.pos));
            }
        }
    }
    matches.sort_by(|a, b| a.0.total_cmp(&b.0));
    matches.dedup_by_key(|&mut (_, i, _)| i);
    matches.into_iter().map(|(_, i, pos)| (i, pos)).collect()
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
    let mut pairs: Vec<(u32, u32)> = opposite
        .into_values()
        .filter_map(|opps| match opps.as_slice() {
            [p, q] if p != q => Some((*p, *q)),
            _ => None,
        })
        .collect();
    // HashMap order varies from run to run; keep the solver's stable.
    pairs.sort_unstable();
    pairs
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
        let mut points = points_in_range(&layout.boundary, outline_len, edge.start, edge.end);
        // A backwards edge is sewn from its end to its start, so its
        // particles pair up with the other side's in that order.
        if !edge.forward {
            points.reverse();
        }
        for (i, pos) in points {
            chain.push((layout.offset + i, pos));
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

/// An open polyline with extra points so no segment is longer than `spacing`.
fn resample(points: &[Vec2], spacing: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    for w in points.windows(2) {
        out.push(w[0]);
        let steps = (w[0].distance(w[1]) / spacing).ceil() as usize;
        for k in 1..steps {
            out.push(w[0].lerp(w[1], k as f32 / steps as f32));
        }
    }
    out.extend(points.last());
    out
}

/// Where a tack's avatar end is in the world, on `avatar` as it is now. A
/// surface point follows the body when it's resized; a fixed point (as
/// format 9 saved them) stays put. `None` for a pattern target, or a
/// surface point the mesh doesn't have.
pub fn avatar_point(target: &TackTarget, avatar: Option<&Mesh>) -> Option<glam::Vec3> {
    match *target {
        TackTarget::Pattern(_) => None,
        TackTarget::Avatar(at) => Some(glam::Vec3::from_array(at)),
        TackTarget::AvatarSurface { triangle, weights } => {
            let mesh = avatar?;
            let base = triangle as usize * 3;
            let corners = mesh.indices.get(base..base + 3)?;
            let mut p = glam::Vec3::ZERO;
            for (&v, w) in corners.iter().zip(weights) {
                p += *mesh.positions.get(v as usize)? * w;
            }
            Some(p)
        }
    }
}

/// The stitch for one tack, or `None` if a pattern it holds isn't
/// simulated. An avatar end becomes a fixed (massless) particle pushed onto
/// `particles`: nothing moves it, so the stitch pulls the cloth to it.
pub fn tack_stitch(
    tack: &od_core::document::Tack,
    locate: impl Fn(PatternId, Vec2) -> Option<SurfacePoint>,
    avatar: Option<&Mesh>,
    particles: &mut Vec<Particle>,
    compliance: f32,
) -> Option<SurfaceStitch> {
    let a = locate(tack.a.pattern, tack.a.at)?;
    let b = match &tack.b {
        TackTarget::Pattern(b) => locate(b.pattern, b.at)?,
        target => {
            particles.push(Particle::pinned(avatar_point(target, avatar)?));
            SurfacePoint::at(particles.len() - 1)
        }
    };
    Some(SurfaceStitch { a, b, compliance })
}

/// Build particles and constraints for every pattern in `placements`,
/// stitched together by `doc`'s sewing lines and tacks. Patterns in `doc`
/// not listed in `placements` are left out of the simulation. `avatar` is
/// the body as it is now, for tacks held to its surface.
pub fn build(
    doc: &Document,
    placements: &[(PatternId, Placement)],
    avatar: Option<&Mesh>,
    config: &ClothConfig,
) -> ClothScene {
    let mut particles = Vec::new();
    let mut structural_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut structural = Vec::new();
    let mut bending = Vec::new();
    let mut pattern_ranges = Vec::new();
    let mut layouts: HashMap<PatternId, PatternLayout> = HashMap::new();
    let mut order = Vec::new();

    for (id, placement) in placements {
        let Some(pattern) = doc.pattern(*id) else {
            continue;
        };
        let mesh = mesh_for(pattern, config);
        let flat: Vec<glam::Vec2> = mesh.vertices.iter().map(|v| to_glam(v.pos)).collect();
        // Centred on the outline's bounds, which interior points don't
        // widen, so the piece lands where the flat preview shows it.
        let world = placement.place(&flat);

        // Each particle carries the fabric of the surface around it, so a
        // finer or coarser mesh weighs the same overall.
        let offset = particles.len();
        for (area, &p) in mesh.vertex_areas().iter().zip(&world) {
            let mass = (area * 1e-6 * pattern.fabric.mass_per_area).max(config.min_particle_mass);
            particles.push(Particle::new(p, 1.0 / mass));
        }
        pattern_ranges.push((*id, offset..particles.len()));

        for &[a, b, c] in &mesh.faces {
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
        for (p, q) in bending_pairs(&mesh.faces) {
            let (ga, gb) = (offset + p as usize, offset + q as usize);
            let rest_length = world[p as usize].distance(world[q as usize]);
            bending.push(DistanceConstraint::new(
                ga.min(gb),
                ga.max(gb),
                rest_length,
                pattern.fabric.bending_compliance,
            ));
        }

        let boundary = mesh
            .boundary
            .iter()
            .filter_map(|&v| {
                let vertex = mesh.vertices[v as usize];
                Some((
                    v as usize,
                    FlatPoint {
                        pos: vertex.pos,
                        param: vertex.param?,
                    },
                ))
            })
            .collect();
        order.push(*id);
        layouts.insert(
            *id,
            PatternLayout {
                pattern,
                offset,
                mesh,
                boundary,
            },
        );
    }

    let mut trims = Vec::new();
    let mut topstitches = Vec::new();
    for id in &order {
        let layout = &layouts[id];
        let spacing = particle_distance(layout.pattern, config);
        for trim in &layout.pattern.trims {
            if let Some(at) = layout.surface_point(trim.pos) {
                trims.push((trim.kind, at));
            }
        }
        for line in &layout.pattern.topstitches {
            topstitches.push(
                resample(&line.points, spacing)
                    .into_iter()
                    .filter_map(|p| layout.surface_point(p))
                    .collect(),
            );
        }
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
    let stitches = stitch_pairs
        .into_iter()
        .map(|(a, b)| DistanceConstraint::new(a, b, 0.0, config.stitch_compliance))
        .collect();

    let locate = |pattern: PatternId, at: Vec2| layouts.get(&pattern)?.surface_point(at);
    let mut tacks = Vec::new();
    for (i, tack) in doc.tacks.iter().enumerate() {
        if let Some(stitch) = tack_stitch(
            tack,
            locate,
            avatar,
            &mut particles,
            config.stitch_compliance,
        ) {
            tacks.push((i, stitch));
        }
    }

    let pattern_meshes = order
        .into_iter()
        .filter_map(|id| layouts.remove(&id).map(|l| (id, l.mesh)))
        .collect();
    ClothScene {
        particles,
        structural,
        bending,
        stitches,
        pattern_ranges,
        pattern_meshes,
        trims,
        topstitches,
        tacks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpbd::{SurfacePoint, World};
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

    /// Meshed with just the outline, as before interior particles: for
    /// tests that count or index particles by corner.
    fn coarse() -> ClothConfig {
        ClothConfig {
            particle_distance: 1000.0,
            ..ClothConfig::default()
        }
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
        let scene = build(&doc, &placements, None, &coarse());
        assert_eq!(scene.particles.len(), 4);
        assert_eq!(scene.pattern_meshes.len(), 1);
        assert_eq!(
            scene.pattern_meshes[0].1.faces.len(),
            2,
            "a quad is 2 triangles"
        );
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
        let scene = build(&doc, &placements, None, &coarse());

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
        let scene = build(&doc, &placements, None, &ClothConfig::default());
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
        let scene = build(&doc, &placements, None, &ClothConfig::default());
        let world = |flat: Vec2| {
            placements[0].1.place(&[
                glam::Vec2::new(0.0, 0.0),
                glam::Vec2::new(200.0, 300.0),
                glam::Vec2::new(flat.x, flat.y),
            ])[2]
        };

        // Each sits exactly where it was placed, not at the nearest corner.
        assert_eq!(scene.trims.len(), 1);
        assert_eq!(scene.trims[0].0, TrimKind::Button);
        let at = scene.trims[0].1.position(&scene.particles);
        assert!(at.distance(world(Vec2::new(195.0, 5.0))) < 1e-4, "{at}");

        // The 190 mm line resampled to the 20 mm particle distance.
        assert_eq!(scene.topstitches.len(), 1);
        let line = &scene.topstitches[0];
        assert_eq!(line.len(), 11);
        let end = line[10].position(&scene.particles);
        assert!(end.distance(world(Vec2::new(195.0, 2.0))) < 1e-4, "{end}");
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
        let scene = build(&doc, &placements, None, &ClothConfig::default());
        assert!(scene.stitches.is_empty(), "no seams");
        assert_eq!(scene.tacks.len(), 1);
        let (index, stitch) = scene.tacks[0];
        assert_eq!(index, 0);
        let (front, back) = (
            scene.pattern_ranges[0].1.clone(),
            scene.pattern_ranges[1].1.clone(),
        );
        assert!(stitch.a.particles.iter().all(|i| front.contains(i)));
        assert!(stitch.b.particles.iter().all(|i| back.contains(i)));
    }

    #[test]
    fn avatar_tack_stitches_the_cloth_to_a_fixed_anchor() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "front", 200.0, 300.0));
        let anchor = [0.0, 1.2, 0.05];
        doc.tacks.push(Tack {
            id: TackId(1),
            // Inside the pattern: held right there.
            a: TackPoint {
                pattern: Pid(1),
                at: Vec2::new(195.0, 150.0),
            },
            b: TackTarget::Avatar(anchor),
        });
        let placements = placements_for(&doc);
        let scene = build(&doc, &placements, None, &ClothConfig::default());
        let cloth = scene.pattern_ranges[0].1.clone();
        assert_eq!(scene.particles.len(), cloth.end + 1, "one anchor particle");
        let fixed = &scene.particles[cloth.end];
        assert_eq!(fixed.inv_mass, 0.0);
        assert_eq!(fixed.position.to_array(), anchor);
        let stitch = scene.tacks[0].1;
        assert_eq!(stitch.b, SurfacePoint::at(cloth.end));
        assert!(stitch.a.particles.iter().all(|i| cloth.contains(i)));

        // Stepping pulls the tacked cloth towards the anchor; the anchor stays.
        let mut world = World::new(scene.particles.clone());
        world.distance_constraints = scene.structural.clone();
        world.surface_stitches = vec![stitch];
        let gap = |world: &World| {
            stitch
                .a
                .position(&world.particles)
                .distance(glam::Vec3::from_array(anchor))
        };
        let start = gap(&world);
        for _ in 0..30 {
            world.step(1.0 / 60.0, 8, &[]);
        }
        let end = gap(&world);
        assert!(end < start * 0.5, "{start} -> {end}");
        assert_eq!(world.particles[cloth.end].position.to_array(), anchor);
    }

    #[test]
    fn a_pattern_gets_interior_particles_at_its_particle_distance() {
        let mut doc = Document::new("test");
        let mut panel = rectangle(1, "panel", 200.0, 300.0);
        doc.patterns.push(panel.clone());
        let placements = placements_for(&doc);
        let fine = build(&doc, &placements, None, &ClothConfig::default());
        assert!(fine.particles.len() > 150, "{}", fine.particles.len());
        // The whole piece weighs what its fabric says, however it's meshed.
        let mass: f32 = fine.particles.iter().map(|p| 1.0 / p.inv_mass).sum();
        let expected = 0.2 * 0.3 * panel.fabric.mass_per_area;
        assert!(
            (mass - expected).abs() < expected * 0.01,
            "{mass} vs {expected}"
        );

        // A pattern's own particle distance overrides the default.
        panel.particle_distance = Some(40.0);
        doc.patterns[0] = panel;
        let coarser = build(&doc, &placements, None, &ClothConfig::default());
        assert!(coarser.particles.len() < fine.particles.len() / 2);
    }

    #[test]
    fn avatar_surface_tacks_anchor_on_the_avatar_as_it_is_now() {
        use od_render::mesh::Mesh;
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, "front", 200.0, 300.0));
        doc.tacks.push(Tack {
            id: TackId(1),
            a: TackPoint {
                pattern: Pid(1),
                at: Vec2::new(100.0, 150.0),
            },
            b: TackTarget::AvatarSurface {
                triangle: 0,
                weights: [0.5, 0.5, 0.0],
            },
        });
        let placements = placements_for(&doc);
        let mut body = Mesh {
            positions: vec![
                glam::Vec3::new(0.0, 1.0, 0.1),
                glam::Vec3::new(0.2, 1.0, 0.1),
                glam::Vec3::new(0.0, 1.2, 0.1),
            ],
            normals: vec![glam::Vec3::Z; 3],
            indices: vec![0, 1, 2],
        };
        let anchor_on = |body: &Mesh| {
            let scene = build(&doc, &placements, Some(body), &ClothConfig::default());
            scene.particles.last().unwrap().position
        };
        let [i, j] = [body.indices[0], body.indices[1]].map(|v| v as usize);
        let midpoint = (body.positions[i] + body.positions[j]) * 0.5;
        assert!(anchor_on(&body).distance(midpoint) < 1e-6);

        // A bigger body moves the anchor with it.
        body.scale_translate(2.0, glam::Vec3::ZERO);
        assert!(anchor_on(&body).distance(midpoint * 2.0) < 1e-6);

        // Without the avatar mesh there's nothing to anchor to.
        let scene = build(&doc, &placements, None, &ClothConfig::default());
        assert!(scene.tacks.is_empty());
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
            build(&doc, &placements, None, &ClothConfig::default())
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
        let scene = build(&doc, &placements, None, &config);
        assert_eq!(scene.particles[0].inv_mass, 1.0 / 0.002);
    }
}
