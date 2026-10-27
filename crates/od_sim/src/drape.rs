//! Ties a document's patterns to the solver and back: build once from a
//! [`Document`], step it every frame, read the draped shape back out as
//! the same [`PlacedPattern`]s the flat ring layout uses, so a renderer
//! can show either without caring which.

use std::ops::Range;

use glam::Vec3;
use od_core::document::{Document, PatternId};
use od_render::pattern::Placement;
use od_render::view::PlacedPattern;

use crate::cloth::{self, ClothConfig};
use crate::collision::Collider;
use crate::mesh::draped_mesh;
use crate::xpbd::{Pin, World};

/// A garment mid-simulation: the XPBD world plus enough of `ClothScene`'s
/// layout to turn particle positions back into meshes per pattern.
pub struct Drape {
    pub world: World,
    pub colliders: Vec<Collider>,
    pattern_ranges: Vec<(PatternId, Range<usize>)>,
    pattern_faces: Vec<(PatternId, Vec<[u32; 3]>)>,
    /// `world.distance_constraints[..structural_count]` are the
    /// structural (stretch) ones, in that fixed prefix — see
    /// [`Self::strain`].
    structural_count: usize,
}

impl Drape {
    /// Build particles and constraints for `doc` at `placements` (its
    /// patterns' starting position and orientation, e.g. from
    /// [`od_render::pattern::RingLayout`]) and wrap them ready to step.
    /// Has no colliders yet; add them (e.g. the avatar) before stepping.
    pub fn new(
        doc: &Document,
        placements: &[(PatternId, Placement)],
        config: &ClothConfig,
    ) -> Self {
        let scene = cloth::build(doc, placements, config);
        let structural_count = scene.structural.len();
        let mut world = World::new(scene.particles);
        world.distance_constraints = scene.structural;
        world.distance_constraints.extend(scene.bending);
        world.distance_constraints.extend(scene.stitches);
        Self {
            world,
            colliders: Vec::new(),
            pattern_ranges: scene.pattern_ranges,
            pattern_faces: scene.pattern_faces,
            structural_count,
        }
    }

    pub fn step(&mut self, dt: f32, substeps: u32) {
        self.world.step(dt, substeps, &self.colliders);
    }

    /// Pin the particle of `pattern` closest to `point` where it currently
    /// is, or unpin it if it already has a pin. Used by a "click to pin"
    /// tool: `point` is the 3D position the click landed on. Returns
    /// whether a pin now exists there (`false` if `pattern` isn't in this
    /// scene, or was just unpinned).
    pub fn toggle_pin_near(&mut self, pattern: PatternId, point: Vec3) -> bool {
        let Some(range) = self
            .pattern_ranges
            .iter()
            .find(|(id, _)| *id == pattern)
            .map(|(_, range)| range.clone())
        else {
            return false;
        };
        let Some(particle) = self.world.particles[range.clone()]
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.position
                    .distance(point)
                    .total_cmp(&b.position.distance(point))
            })
            .map(|(i, _)| range.start + i)
        else {
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

    /// Where each pin currently holds its particle, for drawing markers.
    pub fn pin_positions(&self) -> Vec<Vec3> {
        self.world.pins.iter().map(|p| p.target).collect()
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
            .filter(|(id, _)| {
                self.pattern_faces
                    .iter()
                    .any(|(pid, f)| pid == id && !f.is_empty())
            })
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
    /// had faces to build a mesh from.
    pub fn placed_patterns(&self) -> Vec<PlacedPattern<PatternId>> {
        self.pattern_ranges
            .iter()
            .filter_map(|(id, range)| {
                let positions: Vec<Vec3> = self.world.particles[range.clone()]
                    .iter()
                    .map(|p| p.position)
                    .collect();
                let faces = &self.pattern_faces.iter().find(|(pid, _)| pid == id)?.1;
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
                Some(PlacedPattern {
                    key: *id,
                    mesh,
                    outline: positions,
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
        let mut drape = Drape::new(&doc, &placements, &ClothConfig::default());
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
        let mut drape = Drape::new(&doc, &placements, &ClothConfig::default());
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
    fn toggle_pin_on_an_unknown_pattern_does_nothing() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, &ClothConfig::default());
        assert!(!drape.toggle_pin_near(PatternId(99), Vec3::ZERO));
        assert!(drape.pin_positions().is_empty());
    }

    #[test]
    fn strain_is_zero_at_rest() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let drape = Drape::new(&doc, &placements, &ClothConfig::default());
        assert!(drape.strain().iter().all(|&s| s < 1e-4), "starts relaxed");
    }

    #[test]
    fn strain_rises_when_a_structural_edge_is_stretched() {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 200.0, 300.0));
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, &ClothConfig::default());
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
        let drape = Drape::new(&doc, &placements, &ClothConfig::default());
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
            }],
            b: vec![SeamEdge {
                pattern: PatternId(2),
                start: 3.0,
                end: 4.0,
            }],
        });
        let placements = placements_for(&doc, 1.5);
        let mut drape = Drape::new(&doc, &placements, &ClothConfig::default());
        for _ in 0..300 {
            drape.step(1.0 / 60.0, 8);
        }
        let placed = drape.placed_patterns();
        assert_eq!(placed.len(), 2);
        for p in &placed {
            assert!(!p.mesh.positions.is_empty());
        }
    }
}
