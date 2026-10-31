//! Draping through a pose change: the avatar moves from one pose to
//! another (or along a motion) over time while the cloth simulates, so a
//! garment draped in one pose ends up on the next.
//!
//! The body has to move in small steps: the avatar collider pushes a
//! particle out through the nearest triangle, which only carries the
//! cloth along if the body moved less than its own thickness since the
//! last step. A jump to the new pose would leave the cloth inside the
//! arms, or push it out the wrong side.

use glam::Quat;
use od_core::document::Tack;
use od_render::mesh::Mesh;
use od_render::skeleton::{blend, Rig};

use crate::collision::Collider;
use crate::drape::Drape;

/// How far a particle is kept off the avatar when no collider says.
const DEFAULT_THICKNESS: f32 = 0.005;

/// The body turning from one set of bone rotations to another over
/// `duration` seconds, eased in and out.
#[derive(Debug, Clone)]
pub struct PoseChange<'a> {
    pub rig: &'a Rig,
    pub from: Vec<Quat>,
    pub to: Vec<Quat>,
    pub duration: f32,
}

impl PoseChange<'_> {
    /// The rotations `time` seconds in (held at the ends).
    pub fn rotations_at(&self, time: f32) -> Vec<Quat> {
        let t = if self.duration > 0.0 {
            (time / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        blend(&self.from, &self.to, t * t * (3.0 - 2.0 * t))
    }

    /// The posed body `time` seconds in.
    pub fn mesh_at(&self, time: f32) -> Mesh {
        self.rig.skin(&self.rotations_at(time))
    }

    /// Call `step` once per time step of `dt` seconds with the body as it
    /// is at the end of that step, until the body has arrived.
    pub fn each_step(&self, dt: f32, mut step: impl FnMut(Mesh, f32)) {
        let steps = (self.duration / dt).ceil().max(1.0) as usize;
        for i in 1..=steps {
            let time = (i as f32 * dt).min(self.duration);
            step(self.mesh_at(time), time);
        }
    }
}

/// Put `mesh` in place of the avatar among `colliders` (the first mesh
/// collider, keeping its thickness), or add it if there is none.
pub fn set_avatar(colliders: &mut Vec<Collider>, mesh: Mesh) {
    let found = colliders
        .iter()
        .position(|c| matches!(c, Collider::Mesh { .. }));
    match found {
        Some(i) => {
            let thickness = match &colliders[i] {
                Collider::Mesh { thickness, .. } => *thickness,
                _ => DEFAULT_THICKNESS,
            };
            colliders[i] = Collider::mesh(mesh, thickness);
        }
        None => colliders.push(Collider::mesh(mesh, DEFAULT_THICKNESS)),
    }
}

/// Simulate `drape` while the body makes `change`, then for `settle` more
/// seconds in the new pose. Tacks to the avatar's surface (`tacks`, the
/// document's) move with the body.
pub fn drape_through(
    drape: &mut Drape,
    change: &PoseChange,
    tacks: &[Tack],
    settle: f32,
    dt: f32,
    substeps: u32,
) {
    change.each_step(dt, |mesh, _| {
        drape.sync_tacks(tacks, Some(&mesh));
        set_avatar(&mut drape.colliders, mesh);
        drape.step(dt, substeps);
    });
    for _ in 0..(settle / dt).ceil() as usize {
        drape.step(dt, substeps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloth::ClothConfig;
    use crate::xpbd::{DistanceConstraint, Particle, Pin, World};
    use glam::Vec3;
    use od_core::document::TackTarget;
    use od_core::document::{Document, OutlinePoint, Pattern, PatternId, TackId, TackPoint};
    use od_core::geometry::Vec2;
    use od_core::pose::{builtin_poses, Joint};
    use od_render::avatar::AvatarKind;
    use od_render::pattern::Placement;
    use od_render::skeleton::pose_rotations;

    fn rig() -> Rig {
        let kind = AvatarKind::Female;
        Rig::avatar(kind, kind.default_measurements())
    }

    fn t_pose(rig: &Rig) -> Vec<Quat> {
        let pose = builtin_poses()
            .into_iter()
            .find(|(n, _)| *n == "T-pose")
            .unwrap()
            .1;
        pose_rotations(&pose, rig.bones())
    }

    /// Distance from `p` to the segment `a`–`b`, and how far along it.
    fn to_segment(p: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        (p.distance(a + ab * t), t)
    }

    /// A sleeve: a tube of cloth around the left arm, its top ring held to
    /// the upper arm (as if sewn to a bodice), draped in the rest pose and
    /// then carried into a T-pose. It ends up around the raised arm.
    #[test]
    fn a_sleeve_follows_the_arm_into_a_t_pose() {
        let rig = rig();
        let shoulder = Joint::LeftShoulder.index();
        let bones = rig.bones();
        let (s, e) = (bones[shoulder].rest, bones[Joint::LeftElbow.index()].rest);
        let axis = (e - s).normalize();
        let side = Vec3::Z.cross(axis).normalize();
        let (rings, around, radius) = (11usize, 12usize, 0.055);
        let mut particles = Vec::new();
        for r in 0..rings {
            let along = 0.2 + 0.025 * r as f32;
            for k in 0..around {
                let a = k as f32 / around as f32 * std::f32::consts::TAU;
                let offset = side * a.cos() + Vec3::Z * a.sin();
                particles.push(Particle::new(s + axis * along + offset * radius, 500.0));
            }
        }
        let at = |r: usize, k: usize| r * around + k % around;
        let mut world = World::new(particles);
        let link = |a: usize, b: usize, world: &mut World| {
            let rest = world.particles[a]
                .position
                .distance(world.particles[b].position);
            world
                .distance_constraints
                .push(DistanceConstraint::new(a, b, rest, 1e-8));
        };
        for r in 0..rings {
            for k in 0..around {
                link(at(r, k), at(r, k + 1), &mut world);
                if r + 1 < rings {
                    link(at(r, k), at(r + 1, k), &mut world);
                    link(at(r, k), at(r + 1, k + 1), &mut world);
                    link(at(r, k + 1), at(r + 1, k), &mut world);
                }
            }
        }
        // The top ring goes where the upper arm takes it.
        let top: Vec<(usize, Vec3)> = (0..around)
            .map(|k| (at(0, k), world.particles[at(0, k)].position))
            .collect();
        let hold = |world: &mut World, rotations: &[Quat]| {
            let carry = rig.transforms(rotations)[shoulder];
            world.pins = top
                .iter()
                .map(|&(particle, rest)| Pin {
                    particle,
                    target: carry.carry(rest),
                })
                .collect();
        };

        let dt = 1.0 / 60.0;
        let rest_pose = vec![Quat::IDENTITY; Joint::COUNT];
        let mut colliders = vec![Collider::mesh(rig.skin(&rest_pose), 0.005)];
        hold(&mut world, &rest_pose);
        for _ in 0..30 {
            world.step(dt, 8, &colliders);
        }
        let low = world
            .particles
            .iter()
            .map(|p| p.position.y)
            .fold(f32::MAX, f32::min);

        let change = PoseChange {
            rig: &rig,
            from: rest_pose,
            to: t_pose(&rig),
            duration: 1.0,
        };
        change.each_step(dt, |mesh, time| {
            hold(&mut world, &change.rotations_at(time));
            set_avatar(&mut colliders, mesh);
            world.step(dt, 8, &colliders);
        });
        for _ in 0..30 {
            world.step(dt, 8, &colliders);
        }

        // Every particle is around the raised arm, outside it. The sleeve
        // hangs a little below the arm, where the forearm is thinner.
        let joints = rig.joint_positions(&change.to);
        let (s, w) = (joints[shoulder], joints[Joint::LeftWrist.index()]);
        assert!((w.y - s.y).abs() < 0.01, "the arm is level");
        let body = &colliders[0];
        for r in 0..rings {
            let centre = (0..around)
                .map(|k| world.particles[at(r, k)].position)
                .sum::<Vec3>()
                / around as f32;
            let (off, _) = to_segment(centre, s, w);
            assert!(off < 0.04, "ring {r} is around the arm: {off}");
        }
        for (i, p) in world.particles.iter().enumerate() {
            let (distance, along) = to_segment(p.position, s, w);
            assert!(distance < 0.11, "particle {i} left the arm: {distance}");
            assert!(along > 0.1, "particle {i} slid up to the body: {along}");
            let pushed = body.resolve(p.position);
            assert!(
                pushed.distance(p.position) < 0.01,
                "particle {i} is inside the body"
            );
        }
        let now_low = world
            .particles
            .iter()
            .map(|p| p.position.y)
            .fold(f32::MAX, f32::min);
        assert!(now_low > low + 0.25, "raised from {low} to {now_low}");
    }

    /// A small patch tacked to the avatar near the left wrist moves with
    /// the wrist when the drape is carried into a T-pose.
    #[test]
    fn avatar_tacks_move_with_the_pose() {
        let rig = rig();
        let rest = rig.rest_mesh();
        let wrist = rig.bones()[Joint::LeftWrist.index()].rest;
        // The triangle nearest the outside of the wrist.
        let outside = wrist + Vec3::X * 0.03;
        let (triangle, _) = rest
            .triangles()
            .enumerate()
            .map(|(i, [a, b, c])| (i, ((a + b + c) / 3.0).distance(outside)))
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .unwrap();
        let tri = rest.triangles().nth(triangle).unwrap();
        let point = |mesh: &Mesh| {
            let [a, b, c] = mesh.triangles().nth(triangle).unwrap();
            (a + b + c) / 3.0
        };

        let mut doc = Document::new("patch");
        doc.patterns.push(Pattern::new(
            PatternId(1),
            "patch",
            vec![
                OutlinePoint::corner(Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(Vec2::new(60.0, 0.0)),
                OutlinePoint::corner(Vec2::new(60.0, 60.0)),
                OutlinePoint::corner(Vec2::new(0.0, 60.0)),
            ],
        ));
        doc.tacks.push(Tack {
            id: TackId(1),
            a: TackPoint {
                pattern: PatternId(1),
                at: Vec2::new(30.0, 30.0),
            },
            b: TackTarget::AvatarSurface {
                triangle: triangle as u32,
                weights: [1.0 / 3.0; 3],
            },
        });
        // The patch starts beside the wrist, facing out.
        let start = (tri[0] + tri[1] + tri[2]) / 3.0 + Vec3::X * 0.02;
        let placements = [(
            PatternId(1),
            Placement {
                center: start,
                right: Vec3::Z,
                up: Vec3::Y,
            },
        )];
        let mut drape = Drape::new(&doc, &placements, Some(rest), &ClothConfig::default());
        drape.colliders = vec![Collider::mesh(rest.clone(), 0.005), Collider::ground(0.0)];
        for _ in 0..20 {
            drape.step(1.0 / 60.0, 8);
        }

        let change = PoseChange {
            rig: &rig,
            from: vec![Quat::IDENTITY; Joint::COUNT],
            to: t_pose(&rig),
            duration: 0.5,
        };
        drape_through(&mut drape, &change, &doc.tacks, 0.25, 1.0 / 60.0, 8);
        let target = point(&change.mesh_at(change.duration));
        let [(_, on_patch, on_body)] = drape.tack_positions()[..] else {
            panic!("one tack");
        };
        assert!(
            on_body.distance(target) < 1e-4,
            "the anchor is on the posed body"
        );
        assert!(
            on_patch.distance(target) < 0.02,
            "the patch came along: {on_patch} vs {target}"
        );
        assert!(target.y > point(rest).y + 0.3, "the wrist went up");
        // And the floor is still there.
        assert!(matches!(drape.colliders[1], Collider::Ground { .. }));
    }

    #[test]
    fn the_avatar_collider_is_replaced_in_place() {
        let rig = rig();
        let mut colliders = vec![
            Collider::ground(0.0),
            Collider::mesh(rig.rest_mesh().clone(), 0.01),
        ];
        set_avatar(&mut colliders, rig.rest_mesh().clone());
        assert_eq!(colliders.len(), 2);
        assert!(matches!(colliders[0], Collider::Ground { .. }));
        assert!(matches!(colliders[1], Collider::Mesh { thickness, .. } if thickness == 0.01));
        let mut none = Vec::new();
        set_avatar(&mut none, rig.rest_mesh().clone());
        assert_eq!(none.len(), 1);

        let change = PoseChange {
            rig: &rig,
            from: vec![Quat::IDENTITY; Joint::COUNT],
            to: t_pose(&rig),
            duration: 0.1,
        };
        let mut times = Vec::new();
        change.each_step(0.03, |_, t| times.push(t));
        assert_eq!(times.len(), 4);
        assert_eq!(*times.last().unwrap(), 0.1, "ends in the new pose");
    }
}
