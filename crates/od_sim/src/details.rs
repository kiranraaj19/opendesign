//! Garment details in the simulation: a zipper's tapes and binding or
//! piping, which stiffen and weigh down the edges they are sewn to, and (when it is open) leave
//! its two sides unstitched; and buttonhole slits, carried on the cloth
//! for drawing. A fastened button needs nothing here: its tack holds it.
//! A puckered seam has its longer side drawn in, the way a gathering
//! thread would, and seeded to ripple out of the cloth's plane.
//!
//! A tape is extra constraints along a run of outline particles: stiff
//! ones between neighbours, so the edge hardly stretches, and between
//! particles a few apart, so it resists bending more than the cloth.

use od_core::document::{PatternId, SewingLineId};
use std::collections::HashMap;

use glam::Vec3;
use od_core::{BindingKind, Pucker, Zipper};

use crate::xpbd::{DistanceConstraint, Particle, SurfacePoint};

/// How a strip sewn along an edge changes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tape {
    /// Mass per metre of edge, in kg.
    pub mass_per_m: f32,
    /// Compliance between neighbouring particles along the edge.
    pub stretch_compliance: f32,
    /// Compliance between particles two to [`TAPE_SPAN`] apart along the
    /// edge, which resist it bending.
    pub bending_compliance: f32,
}

impl Tape {
    /// One side of a dress zipper: a woven tape with plastic teeth.
    pub const ZIPPER: Tape = Tape {
        mass_per_m: 0.02,
        stretch_compliance: 1e-9,
        bending_compliance: 1e-8,
    };

    /// A bias binding folded over the edge: light, and only a little
    /// stiffer than the cloth.
    pub const BINDING: Tape = Tape {
        mass_per_m: 0.01,
        stretch_compliance: 2e-8,
        bending_compliance: 1e-6,
    };

    /// Piping: a covered cord, firm against bending.
    pub const PIPING: Tape = Tape {
        mass_per_m: 0.012,
        stretch_compliance: 5e-9,
        bending_compliance: 5e-8,
    };

    pub fn for_binding(kind: BindingKind) -> Tape {
        match kind {
            BindingKind::Binding => Self::BINDING,
            BindingKind::Piping => Self::PIPING,
        }
    }
}

/// The most neighbours along an edge a tape's bending constraints span.
const TAPE_SPAN: usize = 4;

/// What a run of edge particles in [`SceneDetails::edges`] is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeKind {
    /// One side of a zipper seam: `side` 0 for its side A, 1 for B.
    Zipper {
        seam: SewingLineId,
        side: usize,
        closed: bool,
    },
    /// Binding or piping: `pattern`'s binding number `index`.
    Binding {
        pattern: PatternId,
        index: usize,
        kind: BindingKind,
    },
}

/// A detail along an edge, as the particles it runs through in order.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeDetail {
    pub kind: EdgeKind,
    pub particles: Vec<usize>,
}

/// The constraints details add to a scene, and where they are drawn.
#[derive(Debug, Clone, Default)]
pub struct SceneDetails {
    pub constraints: Vec<DistanceConstraint>,
    pub edges: Vec<EdgeDetail>,
    /// Every zipper's stitches, open or closed, by seam: only a closed
    /// zipper's hold (see [`SceneDetails::closed_stitches`]).
    pub zipper_stitches: Vec<(SewingLineId, DistanceConstraint)>,
    /// Each buttonhole's slit, as its two ends on the cloth.
    pub holes: Vec<[SurfacePoint; 2]>,
    /// Each gathered side of a puckered seam (its seam, 0 for side A or 1
    /// for B) and how far it is drawn in: its sewn length over its own.
    pub gathers: Vec<(SewingLineId, usize, f32)>,
}

/// How far a puckered seam's gathered particles start out of the cloth's
/// plane, alternately either way, in metres: enough to set it rippling
/// rather than squashing flat.
const PUCKER_NUDGE: f32 = 0.002;

impl SceneDetails {
    /// The stitches of the zippers that are closed.
    pub fn closed_stitches(&self) -> impl Iterator<Item = DistanceConstraint> + '_ {
        self.zipper_stitches
            .iter()
            .filter(|(seam, _)| self.is_closed(*seam))
            .map(|&(_, c)| c)
    }

    fn is_closed(&self, seam: SewingLineId) -> bool {
        self.edges
            .iter()
            .any(|e| matches!(e.kind, EdgeKind::Zipper { seam: s, closed: true, .. } if s == seam))
    }

    /// Open or close the zippers: those on `closed` seams close, the rest
    /// open.
    pub fn set_closed(&mut self, closed: &[SewingLineId]) {
        for edge in &mut self.edges {
            if let EdgeKind::Zipper {
                seam, closed: c, ..
            } = &mut edge.kind
            {
                *c = closed.contains(seam);
            }
        }
    }

    /// The seams whose zippers are closed, in order.
    pub fn closed_zippers(&self) -> Vec<SewingLineId> {
        let mut out: Vec<SewingLineId> = self
            .edges
            .iter()
            .filter_map(|e| match e.kind {
                EdgeKind::Zipper {
                    seam, closed: true, ..
                } => Some(seam),
                _ => None,
            })
            .collect();
        out.dedup();
        out
    }

    /// Sew `tape` along the particles of `chain` (in order along the
    /// edge): heavier, and stiff against stretching and bending at the
    /// lengths the particles are apart now.
    pub fn tape(&mut self, chain: &[usize], particles: &mut [Particle], tape: Tape) {
        let at = |i: usize| particles[chain[i]].position;
        let mut extra = vec![0.0; chain.len()];
        for i in 1..chain.len() {
            let length = at(i - 1).distance(at(i));
            extra[i - 1] += length * 0.5 * tape.mass_per_m;
            extra[i] += length * 0.5 * tape.mass_per_m;
            self.constraints.push(DistanceConstraint::new(
                chain[i - 1],
                chain[i],
                length,
                tape.stretch_compliance,
            ));
        }
        // Spans across two to four neighbours: a longer span shortens
        // more as the edge bends, so these resist bending far better
        // than neighbours alone.
        for span in 2..=TAPE_SPAN {
            for i in span..chain.len() {
                self.constraints.push(DistanceConstraint::new(
                    chain[i - span],
                    chain[i],
                    at(i - span).distance(at(i)),
                    tape.bending_compliance,
                ));
            }
        }
        for (&p, extra) in chain.iter().zip(extra) {
            let particle = &mut particles[p];
            if particle.inv_mass > 0.0 && extra > 0.0 {
                particle.inv_mass = 1.0 / (1.0 / particle.inv_mass + extra);
            }
        }
    }

    /// Binding or piping along `chain`, `pattern`'s binding `index`.
    pub fn binding(
        &mut self,
        pattern: PatternId,
        index: usize,
        kind: BindingKind,
        chain: &[usize],
        particles: &mut [Particle],
    ) {
        self.tape(chain, particles, Tape::for_binding(kind));
        self.edges.push(EdgeDetail {
            kind: EdgeKind::Binding {
                pattern,
                index,
                kind,
            },
            particles: chain.to_vec(),
        });
    }

    /// Gather puckered seam `seam`, whose sides run through `sides`'
    /// particles (in order): a side longer than the seam's sewn length
    /// (see [`Pucker::sewn_length`]) has the structural constraints
    /// between its neighbouring particles shortened to match, as if a
    /// gathering thread drew it in, and its particles nudged alternately
    /// either side of the cloth (along `normal_of` each) so it ripples.
    pub fn pucker(
        &mut self,
        seam: SewingLineId,
        pucker: Pucker,
        sides: [&[usize]; 2],
        structural: &mut [DistanceConstraint],
        particles: &mut [Particle],
        normal_of: impl Fn(usize) -> Option<Vec3>,
    ) {
        let by_pair: HashMap<(usize, usize), usize> = structural
            .iter()
            .enumerate()
            .map(|(i, c)| ((c.a.min(c.b), c.a.max(c.b)), i))
            .collect();
        let length = |chain: &[usize]| -> f32 {
            chain
                .windows(2)
                .map(|w| particles[w[0]].position.distance(particles[w[1]].position))
                .sum()
        };
        let lengths = sides.map(length);
        let sewn = pucker.sewn_length(lengths[0], lengths[1]);
        for (side, chain) in sides.into_iter().enumerate() {
            if lengths[side] <= 0.0 || sewn >= lengths[side] * 0.999 {
                continue;
            }
            let ratio = sewn / lengths[side];
            for w in chain.windows(2) {
                if let Some(&i) = by_pair.get(&(w[0].min(w[1]), w[0].max(w[1]))) {
                    structural[i].rest_length *= ratio;
                }
            }
            for (k, &p) in chain.iter().enumerate().skip(1) {
                if k + 1 == chain.len() {
                    break;
                }
                if let Some(normal) = normal_of(p) {
                    let way = if k % 2 == 0 { 1.0 } else { -1.0 };
                    particles[p].position += normal * (PUCKER_NUDGE * way);
                }
            }
            self.gathers.push((seam, side, ratio));
        }
    }

    /// A zipper on seam `seam` whose sides run through `sides`' particles:
    /// both taped, and recorded for drawing. Whether its sides are also
    /// stitched together is up to the caller (only while it is closed).
    pub fn zipper(
        &mut self,
        seam: SewingLineId,
        zipper: Zipper,
        sides: [&[usize]; 2],
        particles: &mut [Particle],
    ) {
        for (side, chain) in sides.into_iter().enumerate() {
            let mut chain = chain.to_vec();
            chain.dedup();
            self.tape(&chain, particles, Tape::ZIPPER);
            self.edges.push(EdgeDetail {
                kind: EdgeKind::Zipper {
                    seam,
                    side,
                    closed: zipper.closed,
                },
                particles: chain,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloth::{build, ClothConfig};
    use crate::xpbd::World;
    use od_core::document::{Document, OutlinePoint, Pattern, PatternId, SeamEdge, SewingLine};
    use od_core::geometry::Vec2;
    use od_core::SeamDetails;
    use od_render::pattern::{Placement, RingLayout};

    fn rectangle(id: u64, w: f32, h: f32) -> Pattern {
        Pattern::new(
            PatternId(id),
            "panel",
            [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
                .map(|(x, y)| OutlinePoint::corner(Vec2::new(x, y)))
                .to_vec(),
        )
    }

    /// Two 150 × 300 panels, the first's right edge sewn top down to the
    /// second's left edge, with `details`.
    fn two_panels(details: SeamDetails) -> (Document, Vec<(PatternId, Placement)>) {
        let mut doc = Document::new("test");
        doc.patterns.push(rectangle(1, 150.0, 300.0));
        doc.patterns.push(rectangle(2, 150.0, 300.0));
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
                forward: false,
            }],
            details,
        });
        // Side by side in one plane, as if laid flat in front of the body.
        let placement = |x: f32| Placement {
            center: glam::Vec3::new(x, 1.2, 0.3),
            right: glam::Vec3::X,
            up: glam::Vec3::Y,
        };
        let placements = vec![
            (PatternId(1), placement(-0.075)),
            (PatternId(2), placement(0.075)),
        ];
        (doc, placements)
    }

    fn zipped(closed: bool) -> SeamDetails {
        SeamDetails {
            zipper: Some(Zipper {
                closed,
                ..Zipper::default()
            }),
            ..SeamDetails::default()
        }
    }

    #[test]
    fn a_closed_zipper_is_stitched_and_taped_and_an_open_one_only_taped() {
        let (plain_doc, placements) = two_panels(SeamDetails::default());
        let plain = build(&plain_doc, &placements, None, &ClothConfig::default());
        assert!(plain.details.edges.is_empty() && plain.details.constraints.is_empty());

        let (doc, _) = two_panels(zipped(true));
        let closed = build(&doc, &placements, None, &ClothConfig::default());
        // Sewn like the plain seam, by stitches of its own.
        assert!(closed.stitches.is_empty());
        let stitches: Vec<_> = closed.details.closed_stitches().collect();
        assert_eq!(stitches, plain.stitches);
        assert_eq!(closed.details.edges.len(), 2, "one tape each side");
        let side = &closed.details.edges[0].particles;
        // 300 mm at the 20 mm particle distance.
        assert_eq!(side.len(), 16);
        assert!(closed.details.constraints.len() >= 2 * (15 + 14));
        // The tape weighs the edge down.
        assert!(closed.particles[side[3]].inv_mass < plain.particles[side[3]].inv_mass);

        let (doc, _) = two_panels(zipped(false));
        let open = build(&doc, &placements, None, &ClothConfig::default());
        assert!(open.stitches.is_empty(), "an open zipper holds nothing");
        assert_eq!(open.details.closed_stitches().count(), 0);
        assert_eq!(open.details.zipper_stitches.len(), stitches.len());
        assert_eq!(open.details.edges.len(), 2);
        assert!(matches!(
            open.details.edges[0].kind,
            EdgeKind::Zipper { closed: false, .. }
        ));
    }

    #[test]
    fn a_zipper_tape_bends_less_than_a_plain_seam() {
        // Hold both panels by their top edge and push the cloth out of
        // its plane: the zipped edge's free end gives way less than a
        // plain seam's.
        let bend = |details: SeamDetails| {
            let (doc, placements) = two_panels(details);
            let scene = build(&doc, &placements, None, &ClothConfig::default());
            let mut world = World::new(scene.particles.clone());
            world.gravity = glam::Vec3::new(0.0, 0.0, -0.5);
            world.distance_constraints = scene.structural.clone();
            world.distance_constraints.extend(scene.bending.clone());
            world.distance_constraints.extend(scene.stitches.clone());
            world
                .distance_constraints
                .extend(scene.details.closed_stitches());
            world.distance_constraints.extend(scene.details.constraints);
            // Clamped: the top two rows of particles are held.
            for p in &mut world.particles {
                if p.position.y > 1.35 - 0.021 {
                    p.inv_mass = 0.0;
                }
            }
            for _ in 0..240 {
                world.step(1.0 / 60.0, 8, &[]);
            }
            // The bottom end of the first panel's sewn edge.
            let bottom = (0..scene.particles.len())
                .filter(|&i| scene.pattern_ranges[0].1.contains(&i))
                .min_by(|&i, &j| {
                    let d = |k: usize| {
                        scene.particles[k]
                            .position
                            .distance(glam::Vec3::new(0.0, 1.05, 0.3))
                    };
                    d(i).total_cmp(&d(j))
                })
                .unwrap();
            0.3 - world.particles[bottom].position.z
        };
        let plain = bend(SeamDetails::default());
        let zipper = bend(zipped(true));
        assert!(plain > 0.01, "the cloth bends: {plain}");
        assert!(zipper < plain * 0.8, "{zipper} vs {plain}");
    }

    /// A 300 mm edge eased into a 240 mm one, with `pucker` or not, in
    /// the plane z = 0.3 without gravity: how far the longer piece's
    /// cloth near the seam ends up out of that plane, at most and on
    /// average, and the scene's gathers.
    fn eased(pucker: Option<Pucker>) -> (f32, f32, Vec<(SewingLineId, usize, f32)>) {
        let (mut doc, mut placements) = two_panels(SeamDetails {
            pucker,
            ..SeamDetails::default()
        });
        doc.patterns[1] = rectangle(2, 150.0, 240.0);
        placements[1].1.center.y = 1.35 - 0.12;
        let scene = build(&doc, &placements, None, &ClothConfig::default());
        let gathers = scene.details.gathers.clone();
        let mut world = World::new(scene.particles.clone());
        world.gravity = glam::Vec3::ZERO;
        world.distance_constraints = scene.structural.clone();
        world.distance_constraints.extend(scene.bending.clone());
        world.distance_constraints.extend(scene.stitches.clone());
        for _ in 0..300 {
            world.step(1.0 / 60.0, 8, &[]);
        }
        let (range, mesh) = (&scene.pattern_ranges[0].1, &scene.pattern_meshes[0].1);
        let near: Vec<f32> = range
            .clone()
            .zip(&mesh.vertices)
            .filter(|(_, v)| v.pos.x >= 100.0)
            .map(|(i, _)| (world.particles[i].position.z - 0.3).abs())
            .collect();
        let max = near.iter().copied().fold(0.0, f32::max);
        (max, near.iter().sum::<f32>() / near.len() as f32, gathers)
    }

    #[test]
    fn a_puckered_seam_gathers_its_longer_side_into_ripples() {
        let (flat_max, flat_mean, none) = eased(None);
        assert!(none.is_empty());
        let (max, mean, gathers) = eased(Some(Pucker { amount: 0.0 }));
        // Only the longer side is drawn in, to the shorter one's length.
        assert_eq!(gathers.len(), 1);
        let (_, side, ratio) = gathers[0];
        assert_eq!(side, 0);
        assert!((ratio - 0.8).abs() < 0.01, "{ratio}");
        assert!(max > 0.004, "it ripples out of the plane: {max}");
        assert!(
            max > flat_max * 1.5 && mean > flat_mean * 1.5,
            "{max} / {mean} vs {flat_max} / {flat_mean}"
        );

        // More pucker draws both sides in further.
        let (_, _, more) = eased(Some(Pucker { amount: 0.1 }));
        assert_eq!(more.len(), 2);
        assert!((more[0].2 - 0.72).abs() < 0.01, "{more:?}");
        assert!((more[1].2 - 0.9).abs() < 0.01, "{more:?}");
    }

    #[test]
    fn piping_stiffens_the_edge_it_runs_along() {
        use od_core::{Binding, BindingKind};
        // One clamped panel pushed out of its plane: its piped right edge
        // gives way less than a bare one, and its binding is drawn.
        let bend = |piping: Option<BindingKind>| {
            let (mut doc, placements) = two_panels(SeamDetails::default());
            doc.sewing_lines.clear();
            doc.patterns.truncate(1);
            if let Some(kind) = piping {
                doc.patterns[0].bindings.push(Binding {
                    kind,
                    start: 1.0,
                    end: 2.0,
                    width: 6.0,
                });
            }
            let scene = build(&doc, &placements[..1], None, &ClothConfig::default());
            let edges = scene.details.edges.len();
            let mut world = World::new(scene.particles.clone());
            world.gravity = glam::Vec3::new(0.0, 0.0, -0.5);
            world.distance_constraints = scene.structural.clone();
            world.distance_constraints.extend(scene.bending.clone());
            world.distance_constraints.extend(scene.details.constraints);
            for p in &mut world.particles {
                if p.position.y > 1.35 - 0.021 {
                    p.inv_mass = 0.0;
                }
            }
            for _ in 0..240 {
                world.step(1.0 / 60.0, 8, &[]);
            }
            let corner = glam::Vec3::new(0.0, 1.05, 0.3);
            let bottom = (0..scene.particles.len())
                .min_by(|&i, &j| {
                    let d = |k: usize| scene.particles[k].position.distance(corner);
                    d(i).total_cmp(&d(j))
                })
                .unwrap();
            (0.3 - world.particles[bottom].position.z, edges)
        };
        let (bare, none) = bend(None);
        let (piped, one) = bend(Some(BindingKind::Piping));
        let (bound, _) = bend(Some(BindingKind::Binding));
        assert_eq!((none, one), (0, 1));
        assert!(piped < bare * 0.8, "{piped} vs {bare}");
        assert!(bound < bare && piped < bound, "{piped} < {bound} < {bare}");
    }

    #[test]
    fn a_fastened_button_is_drawn_into_its_buttonhole() {
        use crate::drape::Drape;
        use od_core::details::Fasten;
        use od_core::{AddTrim, Command, TackPoint, TrimKind};
        let (mut doc, placements) = two_panels(SeamDetails::default());
        doc.sewing_lines.clear();
        let (button, hole) = (Vec2::new(120.0, 150.0), Vec2::new(30.0, 150.0));
        AddTrim::new(PatternId(1), TrimKind::Button, button).apply(&mut doc);
        AddTrim::new(PatternId(2), TrimKind::Buttonhole, hole)
            .at_angle(90.0)
            .apply(&mut doc);
        Fasten::new(
            TackPoint {
                pattern: PatternId(1),
                at: button,
            },
            TackPoint {
                pattern: PatternId(2),
                at: hole,
            },
        )
        .apply(&mut doc);
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        drape.world.gravity = glam::Vec3::ZERO;
        let slits = drape.buttonhole_positions();
        assert_eq!(slits.len(), 1);
        let [from, to] = slits[0];
        assert!((from.distance(to) - 0.018).abs() < 1e-4, "18 mm long");
        assert!((from.x - to.x).abs() < 1e-4, "upright, at 90°");

        let gap = |drape: &Drape| {
            let button = drape.trim_positions()[0].1;
            let [a, b] = drape.buttonhole_positions()[0];
            button.distance((a + b) * 0.5)
        };
        let start = gap(&drape);
        assert!((start - 0.06).abs() < 1e-3, "{start}");
        for _ in 0..60 {
            drape.step(1.0 / 60.0, 8);
        }
        assert!(gap(&drape) < 0.005, "{}", gap(&drape));
    }

    #[test]
    fn a_zipper_opens_and_closes_mid_drape() {
        use crate::drape::Drape;
        let (mut doc, _) = two_panels(zipped(true));
        // On opposite sides of the ring, so the zipper has far to pull.
        let ring = RingLayout {
            radius: 0.3,
            height: 1.2,
        };
        let placements: Vec<_> = doc
            .patterns
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, ring.placement(i, 2)))
            .collect();
        let mut drape = Drape::new(&doc, &placements, None, &ClothConfig::default());
        let gap = |drape: &Drape| {
            let sides = drape.edge_detail_positions();
            let middle = |k: usize| sides[k].1[sides[k].1.len() / 2];
            middle(0).distance(middle(1))
        };
        let constraints = drape.world.distance_constraints.len();
        let start = gap(&drape);

        // Opened before it has pulled: the sides stay apart.
        doc.sewing_lines[0].details.zipper = Some(Zipper {
            closed: false,
            ..Zipper::default()
        });
        drape.sync_zippers(&doc.sewing_lines);
        assert!(drape.world.distance_constraints.len() < constraints);
        for _ in 0..30 {
            drape.world.step(1.0 / 60.0, 8, &[]);
        }
        assert!(gap(&drape) > start * 0.8, "{} vs {start}", gap(&drape));

        // Closed again, it pulls them together.
        doc.sewing_lines[0].details.zipper = Some(Zipper::default());
        drape.sync_zippers(&doc.sewing_lines);
        assert_eq!(drape.world.distance_constraints.len(), constraints);
        let open = gap(&drape);
        for _ in 0..30 {
            drape.world.step(1.0 / 60.0, 8, &[]);
        }
        assert!(gap(&drape) < open * 0.5, "{} vs {open}", gap(&drape));
    }
}
