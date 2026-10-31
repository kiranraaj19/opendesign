//! A skeleton for the built-in avatars and linear blend skinning, so a
//! [`Pose`] (or a frame of motion) bends the mesh.
//!
//! A [`Rig`] is the rest mesh, its bones (a joint position and a parent
//! each, parents first) and up to two bone weights per vertex. Posing
//! gives every bone a rotation relative to its parent; a vertex follows
//! the bones it is weighted to. The posed mesh keeps the rest mesh's
//! vertices and triangles, so anything held as a triangle plus weights
//! (avatar tacks, measuring points) follows the body without remapping.

use glam::{Quat, Vec3};
use od_core::pose::{Joint, JointAngles, Pose};

use crate::avatar::{self, AvatarKind, Measurements};
use crate::mesh::Mesh;

/// One bone: the joint it turns about and the bone it hangs from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bone {
    /// Index of the parent bone, always lower than this bone's.
    pub parent: Option<usize>,
    /// The joint, in the rest pose.
    pub rest: Vec3,
    /// Unit direction along the bone in the rest pose; twist turns about it.
    pub axis: Vec3,
}

/// The bones moving one vertex and how much each does; weights sum to one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Influence {
    pub bones: [u16; 2],
    pub weights: [f32; 2],
}

impl Influence {
    fn single(bone: usize) -> Self {
        Self {
            bones: [bone as u16; 2],
            weights: [1.0, 0.0],
        }
    }

    /// `bone_a` and `bone_b` mixed, `t` = how much of `bone_b`.
    fn mix(bone_a: usize, bone_b: usize, t: f32) -> Self {
        Self {
            bones: [bone_a as u16, bone_b as u16],
            weights: [1.0 - t, t],
        }
    }
}

/// Where a bone is in a pose: its joint moved to `position` and turned by
/// `rotation` (both from the rest pose, in world space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneTransform {
    pub rest: Vec3,
    pub position: Vec3,
    pub rotation: Quat,
}

impl BoneTransform {
    /// Where a rest-pose point carried by this bone ends up.
    pub fn carry(&self, p: Vec3) -> Vec3 {
        self.position + self.rotation * (p - self.rest)
    }
}

/// A mesh with a skeleton to pose it by.
#[derive(Debug, Clone)]
pub struct Rig {
    bones: Vec<Bone>,
    rest: Mesh,
    influences: Vec<Influence>,
}

impl Rig {
    /// A rig from its parts. Panics if a bone's parent doesn't come before
    /// it, or there isn't one influence per vertex.
    pub fn new(bones: Vec<Bone>, rest: Mesh, influences: Vec<Influence>) -> Self {
        for (i, bone) in bones.iter().enumerate() {
            assert!(bone.parent.is_none_or(|p| p < i), "bone {i}: parent first");
        }
        assert_eq!(influences.len(), rest.positions.len(), "one per vertex");
        assert!(influences
            .iter()
            .all(|inf| inf.bones.iter().all(|&b| (b as usize) < bones.len())));
        Self {
            bones,
            rest,
            influences,
        }
    }

    /// The built-in avatar at this size, with one bone per [`Joint`] (in
    /// [`Joint::ALL`] order) and weights from where each vertex sits along
    /// its body part.
    pub fn avatar(kind: AvatarKind, m: Measurements) -> Self {
        let h = m.clamped().height / 100.0;
        let joints = avatar::joint_positions(kind, m);
        let [head, left_hand, right_hand, left_sole, right_sole] = avatar::limb_ends(kind, m);
        let at = |j: Joint| joints[j.index()];
        // Which point each bone points at: its first child, or the end of
        // its limb.
        let tip = |j: Joint| match j {
            Joint::Pelvis => at(Joint::Spine),
            Joint::Spine => at(Joint::Neck),
            Joint::Neck => head,
            Joint::LeftShoulder => at(Joint::LeftElbow),
            Joint::LeftElbow => at(Joint::LeftWrist),
            Joint::LeftWrist => left_hand,
            Joint::RightShoulder => at(Joint::RightElbow),
            Joint::RightElbow => at(Joint::RightWrist),
            Joint::RightWrist => right_hand,
            Joint::LeftHip => at(Joint::LeftKnee),
            Joint::LeftKnee => at(Joint::LeftAnkle),
            Joint::LeftAnkle => left_sole,
            Joint::RightHip => at(Joint::RightKnee),
            Joint::RightKnee => at(Joint::RightAnkle),
            Joint::RightAnkle => right_sole,
        };
        let bones = Joint::ALL
            .into_iter()
            .map(|j| Bone {
                parent: j.parent().map(Joint::index),
                rest: at(j),
                axis: (tip(j) - at(j)).normalize_or(Vec3::Y),
            })
            .collect();

        let parts = avatar::avatar_parts(kind, m);
        let mut influences = Vec::new();
        // Each chain: its joints with how wide (fraction of the height) the
        // blend between a joint and the one before it is, and where the
        // chain ends.
        let chain = |part: &Mesh, joints: &[(Joint, f32)], end: Vec3| {
            let chain: Vec<(usize, Vec3, f32)> = joints
                .iter()
                .map(|&(j, b)| (j.index(), at(j), b * h))
                .collect();
            chain_weights(part, &chain, end)
        };
        for (i, part) in parts.iter().enumerate() {
            // Parts come as torso, then leg, foot and arm on the left, then
            // the same on the right (see `avatar_parts`).
            let left = i <= 3;
            let pick = |l: Joint, r: Joint| if left { l } else { r };
            let part_influences = match i {
                0 => chain(
                    part,
                    &[
                        (Joint::Pelvis, 0.0),
                        (Joint::Spine, 0.05),
                        (Joint::Neck, 0.012),
                    ],
                    head,
                ),
                1 | 4 => chain(
                    part,
                    &[
                        (pick(Joint::LeftHip, Joint::RightHip), 0.0),
                        (pick(Joint::LeftKnee, Joint::RightKnee), 0.035),
                        (pick(Joint::LeftAnkle, Joint::RightAnkle), 0.012),
                    ],
                    if left { left_sole } else { right_sole },
                ),
                2 | 5 => {
                    let ankle = pick(Joint::LeftAnkle, Joint::RightAnkle).index();
                    vec![Influence::single(ankle); part.positions.len()]
                }
                _ => chain(
                    part,
                    &[
                        (pick(Joint::LeftShoulder, Joint::RightShoulder), 0.0),
                        (pick(Joint::LeftElbow, Joint::RightElbow), 0.03),
                        (pick(Joint::LeftWrist, Joint::RightWrist), 0.012),
                    ],
                    if left { left_hand } else { right_hand },
                ),
            };
            influences.extend(part_influences);
        }
        Self::new(bones, Mesh::merged(&parts), influences)
    }

    pub fn bones(&self) -> &[Bone] {
        &self.bones
    }

    pub fn rest_mesh(&self) -> &Mesh {
        &self.rest
    }

    pub fn influences(&self) -> &[Influence] {
        &self.influences
    }

    /// Each bone's place for `rotations` (one per bone, relative to its
    /// parent; missing ones count as no turn).
    pub fn transforms(&self, rotations: &[Quat]) -> Vec<BoneTransform> {
        let mut out: Vec<BoneTransform> = Vec::with_capacity(self.bones.len());
        for (i, bone) in self.bones.iter().enumerate() {
            let local = rotations.get(i).copied().unwrap_or(Quat::IDENTITY);
            let t = match bone.parent {
                Some(p) => {
                    let parent = out[p];
                    BoneTransform {
                        rest: bone.rest,
                        position: parent.carry(bone.rest),
                        rotation: (parent.rotation * local).normalize(),
                    }
                }
                None => BoneTransform {
                    rest: bone.rest,
                    position: bone.rest,
                    rotation: local.normalize(),
                },
            };
            out.push(t);
        }
        out
    }

    /// The joints' positions for `rotations`.
    pub fn joint_positions(&self, rotations: &[Quat]) -> Vec<Vec3> {
        self.transforms(rotations)
            .iter()
            .map(|t| t.position)
            .collect()
    }

    /// The mesh posed by `rotations` (see [`Self::transforms`]). The rest
    /// mesh itself when nothing turns.
    pub fn skin(&self, rotations: &[Quat]) -> Mesh {
        if rotations
            .iter()
            .all(|q| q.abs_diff_eq(Quat::IDENTITY, 1e-7))
        {
            return self.rest.clone();
        }
        let transforms = self.transforms(rotations);
        let mut mesh = self.rest.clone();
        for ((p, n), inf) in mesh
            .positions
            .iter_mut()
            .zip(mesh.normals.iter_mut())
            .zip(&self.influences)
        {
            let mut position = Vec3::ZERO;
            let mut normal = Vec3::ZERO;
            for (&b, &w) in inf.bones.iter().zip(&inf.weights) {
                if w > 0.0 {
                    let t = &transforms[b as usize];
                    position += t.carry(*p) * w;
                    normal += t.rotation * *n * w;
                }
            }
            *p = position;
            *n = normal.normalize_or(*n);
        }
        mesh
    }
}

/// Weights for a body part along a straight chain of joints (index, rest
/// position, blend half-width), ending at `end`. A vertex goes with the
/// bone of the stretch it's beside, measured along the chain; within a
/// joint's blend width it mixes smoothly with the bone before.
fn chain_weights(part: &Mesh, chain: &[(usize, Vec3, f32)], end: Vec3) -> Vec<Influence> {
    let origin = chain[0].1;
    let axis = (end - origin).normalize_or(Vec3::Y);
    let along: Vec<f32> = chain
        .iter()
        .map(|&(_, p, _)| (p - origin).dot(axis))
        .collect();
    part.positions
        .iter()
        .map(|&v| {
            let s = (v - origin).dot(axis);
            for k in 1..chain.len() {
                let b = chain[k].2;
                if b > 0.0 && (s - along[k]).abs() < b {
                    let t = smoothstep((s - (along[k] - b)) / (2.0 * b));
                    return Influence::mix(chain[k - 1].0, chain[k].0, t);
                }
            }
            let k = (1..chain.len()).rev().find(|&k| s >= along[k]).unwrap_or(0);
            Influence::single(chain[k].0)
        })
        .collect()
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One joint's turn for its pose angles (see [`JointAngles`] for what
/// they mean), about the bone's rest `axis` for the twist.
pub fn joint_rotation(joint: Joint, angles: JointAngles, axis: Vec3) -> Quat {
    let side = joint.side();
    // Signs that make each angle mean the same movement on both sides.
    // A bone hanging down swings forward for a negative turn about +X, one
    // pointing up for a positive one.
    let (bend, spread, twist) = match joint {
        Joint::Pelvis | Joint::Spine | Joint::Neck => (1.0, -1.0, 1.0),
        Joint::LeftKnee | Joint::RightKnee => (1.0, side, -side),
        _ => (-1.0, side, -side),
    };
    let r = |deg: f32, sign: f32| deg.to_radians() * sign;
    Quat::from_rotation_z(r(angles.spread, spread))
        * Quat::from_rotation_x(r(angles.bend, bend))
        * Quat::from_axis_angle(axis, r(angles.twist, twist))
}

/// Per-bone rotations for `pose` on an avatar rig (one bone per [`Joint`]).
pub fn pose_rotations(pose: &Pose, bones: &[Bone]) -> Vec<Quat> {
    Joint::ALL
        .into_iter()
        .zip(bones)
        .map(|(j, bone)| joint_rotation(j, pose.get(j), bone.axis))
        .collect()
}

/// Part way from one set of rotations to another (`t` from 0 to 1), each
/// bone turning the short way round.
pub fn blend(from: &[Quat], to: &[Quat], t: f32) -> Vec<Quat> {
    let n = from.len().max(to.len());
    (0..n)
        .map(|i| {
            let a = from.get(i).copied().unwrap_or(Quat::IDENTITY);
            let b = to.get(i).copied().unwrap_or(Quat::IDENTITY);
            a.slerp(b, t.clamp(0.0, 1.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use od_core::pose::{builtin_poses, REST_ARM_ANGLE};

    fn rig() -> Rig {
        let kind = AvatarKind::Female;
        Rig::avatar(kind, kind.default_measurements())
    }

    fn posed(rig: &Rig, pose: &Pose) -> Mesh {
        rig.skin(&pose_rotations(pose, rig.bones()))
    }

    fn joint(rig: &Rig, pose: &Pose, j: Joint) -> Vec3 {
        rig.joint_positions(&pose_rotations(pose, rig.bones()))[j.index()]
    }

    #[test]
    fn rest_pose_is_the_generated_avatar() {
        for kind in AvatarKind::ALL {
            let m = kind.default_measurements();
            let rig = Rig::avatar(kind, m);
            assert_eq!(rig.bones().len(), Joint::COUNT);
            let mesh = posed(&rig, &Pose::rest());
            assert_eq!(mesh.positions, avatar::avatar_mesh(kind, m).positions);
            for inf in rig.influences() {
                assert!((inf.weights[0] + inf.weights[1] - 1.0).abs() < 1e-5);
                assert!(inf.weights.iter().all(|w| (0.0..=1.0).contains(w)));
            }
        }
        assert!((REST_ARM_ANGLE.to_radians() - avatar::ARM_ANGLE).abs() < 1e-6);
    }

    #[test]
    fn joints_sit_inside_the_body_part_they_move() {
        let rig = rig();
        let wrist = rig.bones()[Joint::LeftWrist.index()].rest;
        let elbow = rig.bones()[Joint::LeftElbow.index()].rest;
        assert!(
            wrist.x > elbow.x && wrist.y < elbow.y,
            "arm hangs out and down"
        );
        // Vertices near the wrist go with the hand or forearm, near the
        // shoulder with the upper arm.
        let mesh = rig.rest_mesh();
        for (p, inf) in mesh.positions.iter().zip(rig.influences()) {
            if p.distance(elbow) < 0.01 {
                let bones = [inf.bones[0] as usize, inf.bones[1] as usize];
                assert!(
                    bones.contains(&Joint::LeftElbow.index())
                        || bones.contains(&Joint::LeftShoulder.index()),
                    "{p:?}: {inf:?}"
                );
            }
        }
    }

    #[test]
    fn bending_an_elbow_moves_the_forearm_only() {
        let rig = rig();
        let pose = Pose::rest().with(Joint::LeftElbow, JointAngles::new(90.0, 0.0, 0.0));
        let rest = rig.rest_mesh();
        let mesh = posed(&rig, &pose);
        let elbow = joint(&rig, &pose, Joint::LeftElbow);
        let wrist = joint(&rig, &pose, Joint::LeftWrist);
        assert_eq!(elbow, rig.bones()[Joint::LeftElbow.index()].rest);
        // The forearm now points forward, level.
        let fore = wrist - elbow;
        assert!(fore.z > 0.2 && fore.y.abs() < 0.01, "{fore:?}");
        // Only the vertices the elbow and wrist move have moved.
        let arm = [Joint::LeftElbow.index(), Joint::LeftWrist.index()];
        for ((p, q), inf) in rest
            .positions
            .iter()
            .zip(&mesh.positions)
            .zip(rig.influences())
        {
            let follows = inf
                .bones
                .iter()
                .zip(inf.weights)
                .any(|(&b, w)| w > 0.0 && arm.contains(&(b as usize)));
            if !follows {
                assert!(p.distance(*q) < 1e-6, "{p:?} moved to {q:?}");
            }
        }
        // But the hand has: further forward than the body.
        let front = mesh.positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(front > 0.3, "hand reaches forward: {front}");
        // Normals still unit length.
        assert!(mesh.normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-4));
    }

    #[test]
    fn t_pose_lifts_both_arms_level() {
        let rig = rig();
        let poses = builtin_poses();
        let t = &poses.iter().find(|(n, _)| *n == "T-pose").unwrap().1;
        let shoulder = joint(&rig, t, Joint::LeftShoulder);
        let wrist = joint(&rig, t, Joint::LeftWrist);
        assert!((wrist.y - shoulder.y).abs() < 1e-3, "level: {wrist:?}");
        assert!(wrist.x > shoulder.x + 0.4);
        let right = joint(&rig, t, Joint::RightWrist);
        assert!((right + Vec3::new(wrist.x, 0.0, 0.0) * 2.0 - wrist).length() < 1e-4);
        let b = posed(&rig, t).bounds().unwrap();
        assert!((b.min.x + b.max.x).abs() < 1e-3, "symmetric");
        assert!(b.size().x > 1.4, "arm span {:?}", b.size());
        // Feet stay on the floor.
        assert!(b.min.y.abs() < 1e-3);
    }

    #[test]
    fn signs_mean_the_same_on_both_sides() {
        let rig = rig();
        let out = JointAngles::new(0.0, 30.0, 0.0);
        let forward = JointAngles::new(30.0, 0.0, 0.0);
        for (j, child, angles, check) in [
            (Joint::LeftHip, Joint::LeftKnee, out, 0),
            (Joint::LeftHip, Joint::LeftKnee, forward, 1),
            (Joint::LeftShoulder, Joint::LeftElbow, forward, 1),
            (Joint::LeftShoulder, Joint::LeftElbow, out, 0),
        ] {
            let rest_l = rig.bones()[child.index()].rest;
            let rest_r = rig.bones()[child.mirror().index()].rest;
            let pose = Pose::rest().with_both(j, angles);
            let l = joint(&rig, &pose, child) - rest_l;
            let r = joint(&rig, &pose, child.mirror()) - rest_r;
            match check {
                0 => assert!(l.x > 0.02 && r.x < -0.02, "{j:?} out: {l:?} {r:?}"),
                _ => assert!(l.z > 0.02 && r.z > 0.02, "{j:?} forward: {l:?} {r:?}"),
            }
        }
        // The knee bends the shin back; the spine bends the head forward.
        let pose = Pose::rest()
            .with(Joint::LeftKnee, JointAngles::new(40.0, 0.0, 0.0))
            .with(Joint::Spine, JointAngles::new(20.0, 0.0, 0.0));
        let ankle = joint(&rig, &pose, Joint::LeftAnkle);
        assert!(ankle.z < -0.1, "{ankle:?}");
        assert!(joint(&rig, &pose, Joint::Neck).z > 0.08);
    }

    #[test]
    fn surface_points_follow_the_pose() {
        // An avatar tack holds a triangle and weights: the posed mesh has
        // the same triangles, so the point moves with the body.
        let rig = rig();
        let rest = rig.rest_mesh();
        let wrist = rig.bones()[Joint::LeftWrist.index()].rest;
        let tri = rest
            .indices
            .chunks(3)
            .position(|t| rest.positions[t[0] as usize].distance(wrist) < 0.05)
            .unwrap();
        let point = |mesh: &Mesh| {
            let t = &mesh.indices[tri * 3..tri * 3 + 3];
            (mesh.positions[t[0] as usize]
                + mesh.positions[t[1] as usize]
                + mesh.positions[t[2] as usize])
                / 3.0
        };
        let poses = builtin_poses();
        let t = &poses.iter().find(|(n, _)| *n == "T-pose").unwrap().1;
        let mesh = posed(&rig, t);
        assert_eq!(mesh.indices, rest.indices);
        let moved = point(&mesh);
        let wrist_t = joint(&rig, t, Joint::LeftWrist);
        assert!(moved.distance(wrist_t) < 0.06, "{moved:?} near {wrist_t:?}");
        assert!(moved.y > point(rest).y + 0.3, "went up with the arm");
    }

    #[test]
    fn sizing_still_works_when_posed() {
        let kind = AvatarKind::Male;
        let base = kind.default_measurements();
        let tall = Measurements {
            height: 195.0,
            ..base
        };
        let poses = builtin_poses();
        let walk = &poses.iter().find(|(n, _)| *n == "Walking").unwrap().1;
        let a = Rig::avatar(kind, base);
        let b = Rig::avatar(kind, tall);
        let (ma, mb) = (posed(&a, walk), posed(&b, walk));
        let (ba, bb) = (ma.bounds().unwrap(), mb.bounds().unwrap());
        assert!(bb.size().y > ba.size().y + 0.1);
        assert!(ma.positions.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn every_builtin_pose_skins_cleanly() {
        let rig = rig();
        for (name, pose) in builtin_poses() {
            let mesh = posed(&rig, &pose);
            assert!(mesh.positions.iter().all(|p| p.is_finite()), "{name}");
            let b = mesh.bounds().unwrap();
            assert!(b.min.y > -0.05, "{name}: nothing far below the floor");
            assert!(b.max.y < 1.9, "{name}");
        }
    }

    #[test]
    fn blending_goes_between_poses() {
        let rig = rig();
        let a = pose_rotations(&Pose::rest(), rig.bones());
        let poses = builtin_poses();
        let t = &poses.iter().find(|(n, _)| *n == "T-pose").unwrap().1;
        let b = pose_rotations(t, rig.bones());
        assert_eq!(blend(&a, &b, 0.0), a);
        let end = blend(&a, &b, 1.0);
        for (x, y) in end.iter().zip(&b) {
            assert!(x.abs_diff_eq(*y, 1e-5));
        }
        let half = rig.joint_positions(&blend(&a, &b, 0.5))[Joint::LeftWrist.index()];
        let (rest, up) = (
            rig.joint_positions(&a)[Joint::LeftWrist.index()],
            rig.joint_positions(&b)[Joint::LeftWrist.index()],
        );
        assert!(half.y > rest.y + 0.1 && half.y < up.y - 0.1);
    }
}
