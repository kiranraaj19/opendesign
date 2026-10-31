//! Ties a document's patterns to the solver and back: build once from a
//! [`Document`], step it every frame, read the draped shape back out as
//! the same [`PlacedPattern`]s the flat ring layout uses, so a renderer
//! can show either without caring which.

use std::ops::Range;

use glam::Vec3;
use od_core::document::{Document, PatternId, SewingLine, SewingLineId, Tack, TrimKind};
use od_core::geometry::Vec2;
use od_render::mesh::Mesh;
use od_render::pattern::Placement;
use od_render::view::PlacedPattern;

use crate::cloth::{self, ClothConfig};
use crate::collision::{closest_on_triangle, Collider};
use crate::details::{EdgeKind, SceneDetails};
use crate::mesh::draped_mesh;
use crate::mesher::PatternMesh;
use crate::self_collision::{SelfCollision, DEFAULT_THICKNESS};
use crate::xpbd::{Pin, SurfacePoint, World};

/// A garment mid-simulation: the XPBD world plus enough of `ClothScene`'s
/// layout to turn particle positions back into meshes per pattern.
pub struct Drape {
    pub world: World,
    pub colliders: Vec<Collider>,
    pattern_ranges: Vec<(PatternId, Range<usize>)>,
    pattern_meshes: Vec<(PatternId, PatternMesh)>,
    /// `world.distance_constraints[..structural_count]` are the
    /// structural (stretch) ones, in that fixed prefix — see
    /// [`Self::strain`].
    structural_count: usize,
    trims: Vec<(TrimKind, SurfacePoint)>,
    topstitches: Vec<Vec<SurfacePoint>>,
    /// Zipper tapes along the cloth's edges, to draw, and the zippers'
    /// stitches.
    details: SceneDetails,
    /// `world.distance_constraints[zip_base..]` are the closed zippers'
    /// stitches, rebuilt when one opens or closes.
    zip_base: usize,
    /// The tacks the stitches in `world.surface_stitches` were built from,
    /// so a change to the document's tacks can be spotted and synced.
    tacks: Vec<Tack>,
    /// For each of `world.surface_stitches`, the index in `tacks` of the
    /// tack it holds.
    tack_indices: Vec<usize>,
    /// Particles from here on are tack anchors on the avatar, rebuilt
    /// whenever the tacks are.
    anchor_start: usize,
    stitch_compliance: f32,
    /// Particles joined by seams, which cloth-to-cloth collision leaves
    /// out so it doesn't push apart the panels a seam joins.
    seam_particles: Vec<usize>,
    /// The particle being pulled by the mouse, held by a pin in
    /// `world.pins` that [`Self::move_grab`] moves.
    grabbed: Option<usize>,
}

impl Drape {
    /// Build particles and constraints for `doc` at `placements` (its
    /// patterns' starting position and orientation, e.g. from
    /// [`od_render::pattern::RingLayout`]) and wrap them ready to step.
    /// `avatar` is the body tacks to its surface hold to. Has no
    /// colliders yet; add them (e.g. the avatar) before stepping.
    pub fn new(
        doc: &Document,
        placements: &[(PatternId, Placement)],
        avatar: Option<&Mesh>,
        config: &ClothConfig,
    ) -> Self {
        let scene = cloth::build(doc, placements, avatar, config);
        let structural_count = scene.structural.len();
        let anchor_start = scene
            .pattern_ranges
            .iter()
            .map(|(_, r)| r.end)
            .max()
            .unwrap_or(0);
        let seam_particles = scene
            .stitches
            .iter()
            .chain(scene.details.zipper_stitches.iter().map(|(_, c)| c))
            .flat_map(|c| [c.a, c.b])
            .collect();
        let triangles = scene
            .pattern_ranges
            .iter()
            .zip(&scene.pattern_meshes)
            .flat_map(|((_, range), (_, mesh))| {
                mesh.faces
                    .iter()
                    .map(move |f| f.map(|v| range.start + v as usize))
            })
            .collect();
        let mut world = World::new(scene.particles);
        world.distance_constraints = scene.structural;
        world.distance_constraints.extend(scene.bending);
        world.distance_constraints.extend(scene.stitches);
        world
            .distance_constraints
            .extend(scene.details.constraints.iter().copied());
        let zip_base = world.distance_constraints.len();
        world
            .distance_constraints
            .extend(scene.details.closed_stitches());
        let (tack_indices, stitches) = scene.tacks.into_iter().unzip();
        world.surface_stitches = stitches;
        world.self_collision = Some(SelfCollision::new(triangles, Vec::new(), DEFAULT_THICKNESS));
        let mut drape = Self {
            world,
            colliders: Vec::new(),
            pattern_ranges: scene.pattern_ranges,
            pattern_meshes: scene.pattern_meshes,
            structural_count,
            trims: scene.trims,
            topstitches: scene.topstitches,
            details: scene.details,
            zip_base,
            tacks: doc.tacks.clone(),
            tack_indices,
            anchor_start,
            stitch_compliance: config.stitch_compliance,
            seam_particles,
            grabbed: None,
        };
        drape.update_collision_skip();
        drape
    }

    /// Particles cloth-to-cloth collision leaves out: seam and tack ends,
    /// and the avatar anchors.
    fn update_collision_skip(&mut self) {
        let mut skip = vec![false; self.world.particles.len()];
        for &i in &self.seam_particles {
            skip[i] = true;
        }
        for s in &self.world.surface_stitches {
            for side in [s.a, s.b] {
                for (&i, w) in side.particles.iter().zip(side.weights) {
                    if w > 0.0 {
                        skip[i] = true;
                    }
                }
            }
        }
        for flag in skip.iter_mut().skip(self.anchor_start) {
            *flag = true;
        }
        if let Some(collision) = &mut self.world.self_collision {
            collision.set_skip(skip);
        }
    }

    /// Whether cloth-to-cloth collision is on.
    pub fn cloth_collision(&self) -> bool {
        self.world
            .self_collision
            .as_ref()
            .is_some_and(|c| c.enabled)
    }

    /// Turn cloth-to-cloth collision on or off.
    pub fn set_cloth_collision(&mut self, on: bool) {
        if let Some(collision) = &mut self.world.self_collision {
            collision.enabled = on;
        }
    }

    pub fn step(&mut self, dt: f32, substeps: u32) {
        self.world.step(dt, substeps, &self.colliders);
    }

    fn range_of(&self, pattern: PatternId) -> Option<Range<usize>> {
        self.pattern_ranges
            .iter()
            .find(|(id, _)| *id == pattern)
            .map(|(_, range)| range.clone())
    }

    fn mesh_of(&self, pattern: PatternId) -> Option<&PatternMesh> {
        self.pattern_meshes
            .iter()
            .find(|(id, _)| *id == pattern)
            .map(|(_, mesh)| mesh)
    }

    /// Whether the stitches already hold exactly `tacks`.
    pub fn has_tacks(&self, tacks: &[Tack]) -> bool {
        self.tacks == tacks
    }

    /// Rebuild the tack stitches from `tacks`, without restarting the
    /// simulation: for a tack placed, deleted or undone while draping, or
    /// an avatar resized under avatar tacks (their anchors move with it).
    /// Tacks on patterns this drape doesn't simulate are skipped.
    pub fn sync_tacks(&mut self, tacks: &[Tack], avatar: Option<&Mesh>) {
        self.world.particles.truncate(self.anchor_start);
        self.world.pins.retain(|p| p.particle < self.anchor_start);
        let mut stitches = Vec::new();
        let mut indices = Vec::new();
        let ranges = &self.pattern_ranges;
        let meshes = &self.pattern_meshes;
        let locate = |pattern: PatternId, at: Vec2| {
            let offset = ranges.iter().find(|(id, _)| *id == pattern)?.1.start;
            let mesh = &meshes.iter().find(|(id, _)| *id == pattern)?.1;
            cloth::surface_point(mesh, offset, at)
        };
        for (i, tack) in tacks.iter().enumerate() {
            if let Some(stitch) = cloth::tack_stitch(
                tack,
                locate,
                avatar,
                &mut self.world.particles,
                self.stitch_compliance,
            ) {
                stitches.push(stitch);
                indices.push(i);
            }
        }
        self.world.surface_stitches = stitches;
        self.tack_indices = indices;
        self.tacks = tacks.to_vec();
        self.update_collision_skip();
    }

    /// The flat point (mm, in `pattern`'s own space) under a world point on
    /// its draped surface: for placing a tack on the draped cloth.
    pub fn flat_point(&self, pattern: PatternId, point: Vec3) -> Option<Vec2> {
        let offset = self.range_of(pattern)?.start;
        let mesh = self.mesh_of(pattern)?;
        let pos = |v: u32| self.world.particles[offset + v as usize].position;
        let (face, q) = mesh
            .faces
            .iter()
            .map(|&[a, b, c]| (pos(a), pos(b), pos(c)))
            .enumerate()
            .map(|(i, (a, b, c))| (i, closest_on_triangle(a, b, c, point)))
            .min_by(|(_, p), (_, q)| p.distance(point).total_cmp(&q.distance(point)))?;
        let [a, b, c] = mesh.faces[face];
        let weights = od_render::pick::triangle_weights([pos(a), pos(b), pos(c)], q);
        Some(
            [a, b, c]
                .iter()
                .zip(weights)
                .fold(Vec2::ZERO, |acc, (&v, w)| {
                    acc + mesh.vertices[v as usize].pos * w
                }),
        )
    }

    /// Pin the particle of `pattern` closest to `point` where it currently
    /// is, or unpin it if it already has a pin. Used by a "click to pin"
    /// tool: `point` is the 3D position the click landed on. Returns
    /// whether a pin now exists there (`false` if `pattern` isn't in this
    /// scene, or was just unpinned).
    pub fn toggle_pin_near(&mut self, pattern: PatternId, point: Vec3) -> bool {
        let Some(particle) = self.nearest_particle(pattern, point) else {
            return false;
        };
        if let Some(i) = self.world.pins.iter().position(|p| p.particle == particle) {
            self.world.pins.remove(i);
            false
        } else {
            let target = self.world.particles[particle].position;
            self.world.pins.push(Pin { particle, target });
            true
        }
    }

    /// The particle of `pattern` closest to `point`.
    fn nearest_particle(&self, pattern: PatternId, point: Vec3) -> Option<usize> {
        let range = self.range_of(pattern)?;
        self.world.particles[range.clone()]
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.position
                    .distance(point)
                    .total_cmp(&b.position.distance(point))
            })
            .map(|(i, _)| range.start + i)
    }

    /// Start pulling the particle of `pattern` nearest `point` (where a
    /// drag started on the cloth). Returns whether one was grabbed; a
    /// pinned particle stays pinned instead.
    pub fn grab(&mut self, pattern: PatternId, point: Vec3) -> bool {
        self.release();
        let Some(particle) = self.nearest_particle(pattern, point) else {
            return false;
        };
        if self.world.pins.iter().any(|p| p.particle == particle) {
            return false;
        }
        let target = self.world.particles[particle].position;
        self.world.pins.push(Pin { particle, target });
        self.grabbed = Some(particle);
        true
    }

    /// Pull the grabbed particle towards `target`.
    pub fn move_grab(&mut self, target: Vec3) {
        if let Some(particle) = self.grabbed {
            if let Some(pin) = self.world.pins.iter_mut().find(|p| p.particle == particle) {
                pin.target = target;
            }
        }
    }

    /// Let go of the grabbed particle.
    pub fn release(&mut self) {
        if let Some(particle) = self.grabbed.take() {
            self.world.pins.retain(|p| p.particle != particle);
        }
    }

    /// Where the grabbed particle is being pulled to, if anything is.
    pub fn grab_target(&self) -> Option<Vec3> {
        let particle = self.grabbed?;
        self.world
            .pins
            .iter()
            .find(|p| p.particle == particle)
            .map(|p| p.target)
    }

    /// Where each pin currently holds its particle, for drawing markers.
    /// The grab isn't a pin.
    pub fn pin_positions(&self) -> Vec<Vec3> {
        self.world
            .pins
            .iter()
            .filter(|p| Some(p.particle) != self.grabbed)
            .map(|p| p.target)
            .collect()
    }

    /// Where each placed trim (button, snap) currently sits, for drawing
    /// markers at its draped position.
    pub fn trim_positions(&self) -> Vec<(TrimKind, Vec3)> {
        self.trims
            .iter()
            .map(|&(kind, at)| (kind, at.position(&self.world.particles)))
            .collect()
    }

    /// Where each tack's two ends currently sit, for drawing its thread,
    /// with the tack's index in the document's tacks.
    pub fn tack_positions(&self) -> Vec<(usize, Vec3, Vec3)> {
        self.tack_indices
            .iter()
            .zip(&self.world.surface_stitches)
            .map(|(&i, s)| {
                (
                    i,
                    s.a.position(&self.world.particles),
                    s.b.position(&self.world.particles),
                )
            })
            .collect()
    }

    /// Where each placed topstitch line's points currently sit, in order,
    /// for drawing a line through its draped position.
    pub fn topstitch_positions(&self) -> Vec<Vec<Vec3>> {
        self.topstitches
            .iter()
            .map(|line| {
                line.iter()
                    .map(|at| at.position(&self.world.particles))
                    .collect()
            })
            .collect()
    }

    /// Open or close zippers to match the document's seams, without
    /// restarting the simulation: an opened zipper lets go of its sides,
    /// a closed one pulls them back together.
    pub fn sync_zippers(&mut self, lines: &[SewingLine]) {
        let built = |id: &SewingLineId| {
            self.details
                .zipper_stitches
                .iter()
                .any(|(seam, _)| seam == id)
        };
        let mut now: Vec<SewingLineId> = lines
            .iter()
            .filter(|l| l.details.zipper.is_some_and(|z| z.closed))
            .map(|l| l.id)
            .filter(built)
            .collect();
        let mut was = self.details.closed_zippers();
        was.sort();
        now.sort();
        if was == now {
            return;
        }
        self.details.set_closed(&now);
        self.world.distance_constraints.truncate(self.zip_base);
        let closed: Vec<_> = self.details.closed_stitches().collect();
        self.world.distance_constraints.extend(closed);
    }

    /// Where each zipper tape currently runs, in order along its edge.
    pub fn edge_detail_positions(&self) -> Vec<(EdgeKind, Vec<Vec3>)> {
        self.details
            .edges
            .iter()
            .map(|d| {
                let points = d
                    .particles
                    .iter()
                    .map(|&i| self.world.particles[i].position)
                    .collect();
                (d.kind, points)
            })
            .collect()
    }

    /// How stretched the fabric is at each particle: the largest fraction
    /// by which any structural constraint touching it currently exceeds
    /// its rest length (`0.0` = relaxed or compressed, `0.5` = stretched
    /// to 150% of rest length). Bending and stitch constraints aren't
    /// stretch in this sense, so they're left out.
    pub fn strain(&self) -> Vec<f32> {
        let mut strain = vec![0.0_f32; self.world.particles.len()];
        for c in &self.world.distance_constraints[..self.structural_count] {
            if c.rest_length <= 1e-6 {
                continue;
            }
            let dist = self.world.particles[c.a]
                .position
                .distance(self.world.particles[c.b].position);
            let s = (dist / c.rest_length - 1.0).max(0.0);
            strain[c.a] = strain[c.a].max(s);
            strain[c.b] = strain[c.b].max(s);
        }
        strain
    }

    /// Per-pattern vertex colors from [`Self::strain`], via `color_for`,
    /// matching [`Self::placed_patterns`]'s double-sided mesh layout
    /// (every pattern's vertices, front half then an identical back
    /// half). Patterns with no faces are left out, as in
    /// `placed_patterns`.
    pub fn strain_colors(
        &self,
        color_for: impl Fn(f32) -> [f32; 4],
    ) -> Vec<(PatternId, Vec<[f32; 4]>)> {
        let strain = self.strain();
        self.pattern_ranges
            .iter()
            .filter(|(id, _)| self.mesh_of(*id).is_some_and(|m| !m.faces.is_empty()))
            .map(|(id, range)| {
                let front: Vec<[f32; 4]> = strain[range.clone()]
                    .iter()
                    .map(|&s| color_for(s))
                    .collect();
                let mut colors = front.clone();
                colors.extend(front);
                (*id, colors)
            })
            .collect()
    }

    /// The current draped shape, one [`PlacedPattern`] per pattern that
    /// had faces to build a mesh from. Its outline is the pattern's
    /// outline particles, in order.
    pub fn placed_patterns(&self) -> Vec<PlacedPattern<PatternId>> {
        self.pattern_ranges
            .iter()
            .filter_map(|(id, range)| {
                let positions: Vec<Vec3> = self.world.particles[range.clone()]
                    .iter()
                    .map(|p| p.position)
                    .collect();
                let pattern_mesh = self.mesh_of(*id)?;
                let faces = &pattern_mesh.faces;
                let mesh = draped_mesh(&positions, faces)?;
                let normal = faces
                    .first()
                    .map(|&[a, b, c]| {
                        let (pa, pb, pc) = (
                            positions[a as usize],
                            positions[b as usize],
                            positions[c as usize],
                        );
                        (pb - pa).cross(pc - pa).normalize_or_zero()
                    })
                    .unwrap_or(Vec3::Y);
                let outline = pattern_mesh
                    .boundary
                    .iter()
                    .map(|&v| positions[v as usize])
                    .collect();
                Some(PlacedPattern {
                    key: *id,
                    mesh,
                    outline,
                    normal,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use od_core::document::{OutlinePoint, SeamEdge, SewingLine, SewingLineId};
    use od_core::geometry::Vec2;
    use od_render::pattern::RingLayout;

    fn rectangle(id: u64, w: f32, h: f32) -> od_core::document::Pattern {
        od_core::document::Pattern::new(
            PatternId(id),
            "panel",
            vec![
                OutlinePoint::corner(Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(Vec2::new(w, 0.0)),
                OutlinePoint::corner(Vec2::new(w, h)),
                OutlinePoint::corner(Vec2::new(0.0, h)),
            ],
        )
    }

    /// Meshed with just the outline: for tests that index particles by corner.
    fn coarse() -> ClothConfig {
        ClothConfig {
            particle_distance: 1000.0,
            ..ClothConfig::default()
        }
    }

    fn placements_for(doc: &Document, height: f32) -> Vec<(PatternId, Placement)> {
        let ring = RingLayout {
            radius: 0.5,
            height,
        };
        doc.patterns
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, ring.placement(i, doc.patterns.len())))
            .collect()
    }

    #[test]
    fn falling_cloth_is_stopped_by_the_ground() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        drape.colliders.push(Collider::ground(0.0));
        for _ in 0..300 {
            drape.step(1.0 / 60.0, 8);
        }
        for p in &drape.world.particles {
            assert!(p.position.y >= -1e-2, "particle fell through: {p:?}");
        }
    }

    #[test]
    fn pin_toggle_holds_a_particle_and_unpinning_releases_it() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        let near_corner = drape.world.particles[0].position;

        assert!(drape.toggle_pin_near(PatternId(1), near_corner));
        assert_eq!(drape.pin_positions().len(), 1);

        for _ in 0..120 {
            drape.step(1.0 / 60.0, 8);
        }
        let held = drape.world.particles[0].position;
        assert!(
            (held - near_corner).length() < 1e-4,
            "pinned particle moved: {held:?}"
        );

        // Toggling again at the same spot unpins it.
        assert!(!drape.toggle_pin_near(PatternId(1), near_corner));
        assert!(drape.pin_positions().is_empty());
        for _ in 0..120 {
            drape.step(1.0 / 60.0, 8);
        }
        assert!(
            drape.world.particles[0].position.y < held.y - 0.1,
            "unpinned particle should fall"
        );
    }

    #[test]
    fn trim_and_topstitch_positions_follow_the_drape() {
        use od_core::document::{Topstitch, Trim};
        let mut doc = Document::new("test");
        let mut panel = rectangle(1, 200.0, 300.0);
        panel.trims.push(Trim {
            kind: TrimKind::Snap,
            pos: Vec2::new(0.0, 0.0),
        });
        panel.topstitches.push(Topstitch {
            points: vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)],
        });
        doc.patterns.push(panel);
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());

        let trims = drape.trim_positions();
        assert_eq!(trims.len(), 1);
        assert_eq!(trims[0].0, TrimKind::Snap);
        assert!(trims[0].1.distance(drape.world.particles[0].position) < 1e-5);

        // Resampled to the 20 mm particle distance.
        let lines = drape.topstitch_positions();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 11);

        for _ in 0..60 {
            drape.step(1.0 / 60.0, 8);
        }
        // Positions track the particles they were anchored to as the
        // cloth moves, not the pattern's flat starting position.
        let trim = drape.trim_positions()[0].1;
        assert!(trim.distance(drape.world.particles[0].position) < 1e-5);
    }

    #[test]
    fn toggle_pin_on_an_unknown_pattern_does_nothing() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        assert!(!drape.toggle_pin_near(PatternId(99), Vec3::ZERO));
        assert!(drape.pin_positions().is_empty());
    }

    #[test]
    fn strain_is_zero_at_rest() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        assert!(drape.strain().iter().all(|&s| s < 1e-4), "starts relaxed");
    }

    #[test]
    fn strain_rises_when_a_structural_edge_is_stretched() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &coarse());
        // Pull corner 1 far from its neighbours without giving the solver
        // a chance to correct it, as if caught mid-overstretch.
        drape.world.particles[1].position += Vec3::new(5.0, 0.0, 0.0);
        let strain = drape.strain();
        assert!(
            strain[1] > 0.5,
            "corner 1 should read as heavily stretched: {strain:?}"
        );
        assert!(
            strain[3] < 1e-4,
            "the untouched opposite corner stays relaxed: {strain:?}"
        );
    }

    #[test]
    fn strain_colors_cover_every_particle_on_both_mesh_sides() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let drape = Drape::new(&doc, &placements, None, &coarse());
        let colors = drape.strain_colors(|s| [s, s, s, 1.0]);
        assert_eq!(colors.len(), 1);
        assert_eq!(colors[0].0, PatternId(1));
        assert_eq!(
            colors[0].1.len(),
            8,
            "4 particles, doubled for both mesh sides"
        );
    }

    #[test]
    fn sewn_patterns_pull_together_while_draping() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        doc.patterns.push(rectangle(2, 200.0, 300.0));
        doc.sewing_lines.push(SewingLine {
            id: SewingLineId(1),
            a: vec![SeamEdge {
                pattern: PatternId(1),
                start: 1.0,
                end: 2.0,
                forward: true,
            }],
            b: vec![SeamEdge {
                pattern: PatternId(2),
                start: 3.0,
                end: 4.0,
                forward: true,
            }],
            details: Default::default(),
        });
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        for _ in 0..300 {
            drape.step(1.0 / 60.0, 8);
        }
        let placed = drape.placed_patterns();
        assert_eq!(placed.len(), 2);
        for p in &placed {
            assert!(!p.mesh.positions.is_empty());
        }
    }

    #[test]
    fn the_draped_outline_is_the_outline_particles_only() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        let placed = drape.placed_patterns();
        // 1000 mm of outline at 20 mm: 50 points, out of many more particles.
        assert_eq!(placed[0].outline.len(), 50);
        assert!(drape.world.particles.len() > 150);
        assert_eq!(placed[0].outline[0], drape.world.particles[0].position);
        // The mesh covers every particle, front and back.
        assert_eq!(
            placed[0].mesh.positions.len(),
            2 * drape.world.particles.len()
        );
    }

    #[test]
    fn a_draped_point_maps_back_to_its_flat_position() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        for _ in 0..30 {
            drape.step(1.0 / 60.0, 8);
        }
        // A particle's draped position maps back to where it was cut.
        let mesh = drape.mesh_of(PatternId(1)).unwrap().clone();
        let v = mesh.vertices.len() / 2;
        let flat = drape
            .flat_point(PatternId(1), drape.world.particles[v].position)
            .unwrap();
        assert!(flat.distance(mesh.vertices[v].pos) < 1e-2, "{flat:?}");
        assert!(drape.flat_point(PatternId(9), Vec3::ZERO).is_none());
    }

    #[test]
    fn tacks_sync_into_a_running_drape() {
        use od_core::document::{Tack, TackId, TackPoint, TackTarget};
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        let cloth = drape.world.particles.len();
        assert!(drape.has_tacks(&doc.tacks));

        let tack = Tack {
            id: TackId(1),
            a: TackPoint {
                pattern: PatternId(1),
                at: Vec2::new(100.0, 150.0),
            },
            b: TackTarget::Avatar([0.0, 2.0, 0.0]),
        };
        doc.tacks.push(tack);
        assert!(!drape.has_tacks(&doc.tacks));
        drape.sync_tacks(&doc.tacks, None);
        assert!(drape.has_tacks(&doc.tacks));
        assert_eq!(drape.world.particles.len(), cloth + 1, "one anchor");
        assert_eq!(drape.tack_positions().len(), 1);
        assert_eq!(drape.tack_positions()[0].2, Vec3::new(0.0, 2.0, 0.0));

        // Held up by the tack, the middle of the piece doesn't fall.
        for _ in 0..120 {
            drape.step(1.0 / 60.0, 8);
        }
        let (_, held, anchor) = drape.tack_positions()[0];
        assert!(held.distance(anchor) < 0.02, "{held} vs {anchor}");

        // Removing it drops the anchor and the stitch.
        doc.tacks.clear();
        drape.sync_tacks(&doc.tacks, None);
        assert_eq!(drape.world.particles.len(), cloth);
        assert!(drape.tack_positions().is_empty());
    }

    /// Two panels lying flat, one dropped onto the other, as placements.
    fn stacked(doc: &Document) -> Vec<(PatternId, Placement)> {
        doc.patterns
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.id,
                    Placement {
                        center: Vec3::new(0.0, 0.02 + 0.03 * i as f32, 0.0),
                        right: Vec3::X,
                        up: Vec3::Z,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn cloth_collision_keeps_stacked_panels_apart() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 200.0));
        doc.patterns.push(rectangle(2, 200.0, 200.0));
        let placements = stacked(&doc);
        // How far the top panel ends up above the bottom one, at the middle.
        let gap = |collide: bool| {
            let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
            drape.set_cloth_collision(collide);
            assert_eq!(drape.cloth_collision(), collide);
            // The bottom panel rests on the floor; the top falls onto it.
            drape.colliders.push(Collider::ground(0.0));
            for _ in 0..90 {
                drape.step(1.0 / 60.0, 8);
            }
            let middle = |id| {
                let range = drape.range_of(PatternId(id)).unwrap();
                let mesh = drape.mesh_of(PatternId(id)).unwrap();
                let (v, _) = mesh
                    .vertices
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let centre = Vec2::new(100.0, 100.0);
                        a.pos.distance(centre).total_cmp(&b.pos.distance(centre))
                    })
                    .unwrap();
                drape.world.particles[range.start + v].position.y
            };
            middle(2) - middle(1)
        };
        let apart = gap(true);
        assert!(apart > 0.002, "the top panel rests on the bottom: {apart}");
        let through = gap(false);
        assert!(
            through.abs() < 0.001,
            "without collision they meet: {through}"
        );
    }

    /// A T-shirt's worth of cloth stays fast enough to drape live. Timing
    /// only means something in an optimised build, so debug runs skip it.
    #[test]
    #[cfg_attr(debug_assertions, ignore)]
    fn a_garment_sized_drape_steps_in_real_time() {
        let mut doc = Document::new("test");
        for id in 1..=4 {
            doc.patterns.push(rectangle(id, 500.0, 700.0));
        }
        let placements = placements_for(&doc, 1.2);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        drape.colliders.push(Collider::ground(0.0));
        let particles = drape.world.particles.len();
        assert!(particles > 3000, "{particles}");
        drape.step(1.0 / 60.0, 8);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            drape.step(1.0 / 60.0, 8);
        }
        let per_frame = start.elapsed() / 10;
        assert!(
            per_frame < std::time::Duration::from_millis(50),
            "{particles} particles took {per_frame:?} per frame"
        );
    }

    #[test]
    fn a_grabbed_point_follows_its_target_until_let_go() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 200.0));
        let placements = placements_for(&doc, 1.0);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        let range = drape.range_of(PatternId(1)).unwrap();
        let corner = drape.world.particles[range.start].position;
        assert!(drape.grab(PatternId(1), corner));
        assert!(drape.pin_positions().is_empty(), "a grab isn't a pin");
        let target = corner + Vec3::new(0.0, 0.3, 0.1);
        drape.move_grab(target);
        assert_eq!(drape.grab_target(), Some(target));
        for _ in 0..30 {
            drape.step(1.0 / 60.0, 8);
        }
        let grabbed = drape.world.particles[range.start].position;
        assert!(grabbed.distance(target) < 1e-4, "{grabbed} vs {target}");

        drape.release();
        assert!(drape.world.pins.is_empty());
        assert_eq!(drape.grab_target(), None);
        for _ in 0..30 {
            drape.step(1.0 / 60.0, 8);
        }
        let fallen = drape.world.particles[range.start].position;
        assert!(fallen.y < grabbed.y - 0.01, "let go, it falls: {fallen}");

        // A pinned point stays pinned rather than being grabbed.
        assert!(drape.toggle_pin_near(PatternId(1), fallen));
        assert!(!drape.grab(PatternId(1), fallen));
        assert_eq!(drape.pin_positions().len(), 1);
    }
}
