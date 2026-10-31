//! Avatar poses: angles for the joints of the built-in avatars' skeleton.
//!
//! A [`Pose`] is plain data (degrees per joint), so it can be saved with
//! the document; turning it into a deformed mesh is `od_render::skeleton`'s
//! job. Angles are relative to the avatar's rest pose, the mannequin as it
//! is generated: standing, arms hanging 16° out ([`REST_ARM_ANGLE`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::document::Document;

/// How far out from vertical the avatar's arms hang in its rest pose, in
/// degrees. A shoulder's `spread` adds to this.
pub const REST_ARM_ANGLE: f32 = 16.0;

/// The joints of the built-in avatars' skeleton, parents before children.
/// Left and right are the avatar's own (its left is +X, as it faces +Z).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Joint {
    Pelvis,
    Spine,
    Neck,
    LeftShoulder,
    LeftElbow,
    LeftWrist,
    RightShoulder,
    RightElbow,
    RightWrist,
    LeftHip,
    LeftKnee,
    LeftAnkle,
    RightHip,
    RightKnee,
    RightAnkle,
}

impl Joint {
    pub const COUNT: usize = 15;
    pub const ALL: [Joint; Joint::COUNT] = [
        Joint::Pelvis,
        Joint::Spine,
        Joint::Neck,
        Joint::LeftShoulder,
        Joint::LeftElbow,
        Joint::LeftWrist,
        Joint::RightShoulder,
        Joint::RightElbow,
        Joint::RightWrist,
        Joint::LeftHip,
        Joint::LeftKnee,
        Joint::LeftAnkle,
        Joint::RightHip,
        Joint::RightKnee,
        Joint::RightAnkle,
    ];

    /// Position in [`Joint::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Joint::Pelvis => "Pelvis",
            Joint::Spine => "Spine",
            Joint::Neck => "Neck",
            Joint::LeftShoulder => "Left shoulder",
            Joint::LeftElbow => "Left elbow",
            Joint::LeftWrist => "Left wrist",
            Joint::RightShoulder => "Right shoulder",
            Joint::RightElbow => "Right elbow",
            Joint::RightWrist => "Right wrist",
            Joint::LeftHip => "Left hip",
            Joint::LeftKnee => "Left knee",
            Joint::LeftAnkle => "Left ankle",
            Joint::RightHip => "Right hip",
            Joint::RightKnee => "Right knee",
            Joint::RightAnkle => "Right ankle",
        }
    }

    /// The joint this one hangs from; the pelvis is the root.
    pub fn parent(self) -> Option<Joint> {
        Some(match self {
            Joint::Pelvis => return None,
            Joint::Spine | Joint::LeftHip | Joint::RightHip => Joint::Pelvis,
            Joint::Neck | Joint::LeftShoulder | Joint::RightShoulder => Joint::Spine,
            Joint::LeftElbow => Joint::LeftShoulder,
            Joint::LeftWrist => Joint::LeftElbow,
            Joint::RightElbow => Joint::RightShoulder,
            Joint::RightWrist => Joint::RightElbow,
            Joint::LeftKnee => Joint::LeftHip,
            Joint::LeftAnkle => Joint::LeftKnee,
            Joint::RightKnee => Joint::RightHip,
            Joint::RightAnkle => Joint::RightKnee,
        })
    }

    /// `1.0` on the avatar's left (+X), `-1.0` on its right, `0.0` for the
    /// joints down the middle.
    pub fn side(self) -> f32 {
        match self {
            Joint::Pelvis | Joint::Spine | Joint::Neck => 0.0,
            Joint::LeftShoulder
            | Joint::LeftElbow
            | Joint::LeftWrist
            | Joint::LeftHip
            | Joint::LeftKnee
            | Joint::LeftAnkle => 1.0,
            _ => -1.0,
        }
    }

    /// The same joint on the other side (itself for the middle ones).
    pub fn mirror(self) -> Joint {
        match self {
            Joint::LeftShoulder => Joint::RightShoulder,
            Joint::LeftElbow => Joint::RightElbow,
            Joint::LeftWrist => Joint::RightWrist,
            Joint::LeftHip => Joint::RightHip,
            Joint::LeftKnee => Joint::RightKnee,
            Joint::LeftAnkle => Joint::RightAnkle,
            Joint::RightShoulder => Joint::LeftShoulder,
            Joint::RightElbow => Joint::LeftElbow,
            Joint::RightWrist => Joint::LeftWrist,
            Joint::RightHip => Joint::LeftHip,
            Joint::RightKnee => Joint::LeftKnee,
            Joint::RightAnkle => Joint::LeftAnkle,
            middle => middle,
        }
    }
}

/// One joint's rotation from the rest pose, in degrees, as three turns
/// applied twist first, then bend, then spread. The signs are chosen so
/// the same value means the same movement on both sides of the body:
///
/// - `bend`: forward, about the side-to-side axis. Arms and thighs swing
///   forward, elbows and the spine bend forward, knees bend back, ankles
///   lift the toes, the neck tips the head down.
/// - `spread`: about the front-to-back axis. Arms and legs move out, away
///   from the body; the pelvis, spine and neck lean to the avatar's left.
/// - `twist`: about the bone itself. Limbs turn outward; the pelvis, spine
///   and neck turn to the avatar's left.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct JointAngles {
    #[serde(default)]
    pub bend: f32,
    #[serde(default)]
    pub spread: f32,
    #[serde(default)]
    pub twist: f32,
}

impl JointAngles {
    pub const RANGE: std::ops::RangeInclusive<f32> = -180.0..=180.0;

    pub const fn new(bend: f32, spread: f32, twist: f32) -> Self {
        Self {
            bend,
            spread,
            twist,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.bend == 0.0 && self.spread == 0.0 && self.twist == 0.0
    }

    /// Each angle kept in [`Self::RANGE`]; anything not finite becomes 0.
    pub fn clamped(self) -> Self {
        let c = |v: f32| {
            if v.is_finite() {
                v.clamp(*Self::RANGE.start(), *Self::RANGE.end())
            } else {
                0.0
            }
        };
        Self::new(c(self.bend), c(self.spread), c(self.twist))
    }
}

/// Angles for any of the joints; a joint not listed is at rest. Saved as a
/// map from joint name to its angles.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pose {
    joints: BTreeMap<Joint, JointAngles>,
}

impl Pose {
    /// The rest pose: every joint at zero.
    pub fn rest() -> Self {
        Self::default()
    }

    pub fn get(&self, joint: Joint) -> JointAngles {
        self.joints.get(&joint).copied().unwrap_or_default()
    }

    /// Set one joint's angles (clamped to [`JointAngles::RANGE`]).
    pub fn set(&mut self, joint: Joint, angles: JointAngles) {
        let angles = angles.clamped();
        if angles.is_zero() {
            self.joints.remove(&joint);
        } else {
            self.joints.insert(joint, angles);
        }
    }

    /// This pose with one joint changed.
    pub fn with(mut self, joint: Joint, angles: JointAngles) -> Self {
        self.set(joint, angles);
        self
    }

    /// This pose with `angles` on a joint and its mirror image.
    pub fn with_both(self, joint: Joint, angles: JointAngles) -> Self {
        self.with(joint, angles).with(joint.mirror(), angles)
    }

    pub fn is_rest(&self) -> bool {
        self.joints.is_empty()
    }

    /// The same pose with the avatar's left and right swapped.
    pub fn mirrored(&self) -> Self {
        let mut out = Self::rest();
        for (&joint, &a) in &self.joints {
            // Spread and twist of the middle joints are towards the avatar's
            // left, so mirroring turns them the other way.
            let a = if joint.side() == 0.0 {
                JointAngles::new(a.bend, -a.spread, -a.twist)
            } else {
                a
            };
            out.set(joint.mirror(), a);
        }
        out
    }

    /// Keep every angle in range, as after loading a hand-edited file.
    pub fn clamped(&self) -> Self {
        let mut out = Self::rest();
        for (&joint, &a) in &self.joints {
            out.set(joint, a);
        }
        out
    }
}

/// Put the avatar in `pose`; undo puts back the pose it had.
pub struct SetPose {
    label: String,
    /// The pose to put in place; holds the old one after apply.
    pose: Pose,
}

impl SetPose {
    pub fn new(label: impl Into<String>, pose: Pose) -> Self {
        Self {
            label: label.into(),
            pose,
        }
    }
}

impl Command for SetPose {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        std::mem::swap(&mut doc.avatar_pose, &mut self.pose);
    }

    fn undo(&mut self, doc: &mut Document) {
        std::mem::swap(&mut doc.avatar_pose, &mut self.pose);
    }
}

/// The poses offered in the app, written for these avatars: name and pose.
pub fn builtin_poses() -> Vec<(&'static str, Pose)> {
    use Joint::*;
    let arms = |out: f32| Pose::rest().with_both(LeftShoulder, JointAngles::new(0.0, out, 0.0));
    vec![
        ("Rest", Pose::rest()),
        ("Arms down", arms(8.0 - REST_ARM_ANGLE)),
        ("A-pose", arms(45.0 - REST_ARM_ANGLE)),
        ("T-pose", arms(90.0 - REST_ARM_ANGLE)),
        (
            "Arms forward",
            Pose::rest()
                .with_both(LeftShoulder, JointAngles::new(80.0, -10.0, 0.0))
                .with_both(LeftElbow, JointAngles::new(10.0, 0.0, 0.0)),
        ),
        (
            "Walking",
            Pose::rest()
                .with(LeftHip, JointAngles::new(22.0, 0.0, 0.0))
                .with(LeftKnee, JointAngles::new(8.0, 0.0, 0.0))
                .with(LeftAnkle, JointAngles::new(8.0, 0.0, 0.0))
                .with(RightHip, JointAngles::new(-14.0, 0.0, 0.0))
                .with(RightKnee, JointAngles::new(24.0, 0.0, 0.0))
                .with(RightAnkle, JointAngles::new(-6.0, 0.0, 0.0))
                .with(LeftShoulder, JointAngles::new(-18.0, -4.0, 0.0))
                .with(LeftElbow, JointAngles::new(12.0, 0.0, 0.0))
                .with(RightShoulder, JointAngles::new(20.0, -4.0, 0.0))
                .with(RightElbow, JointAngles::new(25.0, 0.0, 0.0))
                .with(Spine, JointAngles::new(0.0, 0.0, -4.0)),
        ),
        (
            "Hands on hips",
            Pose::rest()
                .with_both(LeftShoulder, JointAngles::new(-10.0, 30.0, -70.0))
                .with_both(LeftElbow, JointAngles::new(100.0, 0.0, 0.0))
                .with_both(LeftWrist, JointAngles::new(0.0, 0.0, 0.0)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joints_are_listed_parents_first() {
        for (i, joint) in Joint::ALL.into_iter().enumerate() {
            assert_eq!(joint.index(), i);
            if let Some(parent) = joint.parent() {
                assert!(parent.index() < i, "{joint:?}");
            }
            assert_eq!(joint.mirror().mirror(), joint);
            assert_eq!(joint.mirror().side(), -joint.side());
        }
        assert_eq!(Joint::LeftShoulder.mirror(), Joint::RightShoulder);
    }

    #[test]
    fn zero_angles_are_not_stored() {
        let mut pose = Pose::rest();
        pose.set(Joint::LeftKnee, JointAngles::new(30.0, 0.0, 0.0));
        assert!(!pose.is_rest());
        assert_eq!(pose.get(Joint::LeftKnee).bend, 30.0);
        assert_eq!(pose.get(Joint::RightKnee), JointAngles::default());
        pose.set(Joint::LeftKnee, JointAngles::default());
        assert!(pose.is_rest());
        assert_eq!(pose, Pose::rest());
    }

    #[test]
    fn angles_are_clamped() {
        let pose = Pose::rest().with(Joint::Neck, JointAngles::new(500.0, f32::NAN, -200.0));
        assert_eq!(pose.get(Joint::Neck), JointAngles::new(180.0, 0.0, -180.0));
    }

    #[test]
    fn saves_as_a_map_of_joint_names() {
        let pose = Pose::rest().with(Joint::LeftElbow, JointAngles::new(90.0, 0.0, 5.0));
        let json = serde_json::to_string(&pose).unwrap();
        assert_eq!(
            json,
            r#"{"left_elbow":{"bend":90.0,"spread":0.0,"twist":5.0}}"#
        );
        let back: Pose = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pose);
        // Missing angles default to zero.
        let short: Pose = serde_json::from_str(r#"{"neck":{"bend":10}}"#).unwrap();
        assert_eq!(short.get(Joint::Neck), JointAngles::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn mirroring_swaps_sides() {
        let pose = Pose::rest()
            .with(Joint::LeftHip, JointAngles::new(20.0, 5.0, 0.0))
            .with(Joint::Spine, JointAngles::new(10.0, 4.0, 3.0));
        let m = pose.mirrored();
        assert_eq!(m.get(Joint::RightHip), JointAngles::new(20.0, 5.0, 0.0));
        assert_eq!(m.get(Joint::LeftHip), JointAngles::default());
        assert_eq!(m.get(Joint::Spine), JointAngles::new(10.0, -4.0, -3.0));
        assert_eq!(m.mirrored(), pose);
    }

    #[test]
    fn set_pose_is_undoable_and_saved() {
        use crate::{project, History};
        let mut doc = Document::new("Posed");
        let mut history = History::new();
        let before = doc.clone();
        let t_pose = builtin_poses()
            .into_iter()
            .find(|(n, _)| *n == "T-pose")
            .unwrap()
            .1;
        history.execute(
            &mut doc,
            Box::new(SetPose::new("Pose: T-pose", t_pose.clone())),
        );
        assert_eq!(doc.avatar_pose, t_pose);
        let loaded = project::from_json(&project::to_json(&doc)).unwrap();
        assert_eq!(loaded, doc);
        history.undo(&mut doc);
        assert_eq!(doc, before);
        history.redo(&mut doc);
        assert_eq!(doc.avatar_pose, t_pose);

        // Version 13 files from before poses load standing at rest.
        let mut json: serde_json::Value = serde_json::from_str(&project::to_json(&doc)).unwrap();
        json["document"]
            .as_object_mut()
            .unwrap()
            .remove("avatar_pose");
        let loaded = project::from_json(&json.to_string()).unwrap();
        assert!(loaded.avatar_pose.is_rest());
    }

    #[test]
    fn builtin_poses_have_unique_names_and_a_rest_pose() {
        let poses = builtin_poses();
        assert_eq!(poses[0], ("Rest", Pose::rest()));
        for (i, (name, _)) in poses.iter().enumerate() {
            assert!(poses[..i].iter().all(|(n, _)| n != name), "{name}");
        }
        let t = &poses.iter().find(|(n, _)| *n == "T-pose").unwrap().1;
        assert_eq!(
            t.get(Joint::LeftShoulder).spread + REST_ARM_ANGLE,
            90.0,
            "T-pose arms are horizontal"
        );
        assert_eq!(t, &t.mirrored(), "symmetric");
    }
}
