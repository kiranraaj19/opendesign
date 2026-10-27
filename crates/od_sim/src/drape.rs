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
        let mut world = World::new(scene.particles);
        world.distance_constraints = scene.structural;
        world.distance_constraints.extend(scene.bending);
        world.distance_constraints.extend(scene.stitches);
        Self {
            world,
            colliders: Vec::new(),
            pattern_ranges: scene.pattern_ranges,
            pattern_faces: scene.pattern_faces,
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
