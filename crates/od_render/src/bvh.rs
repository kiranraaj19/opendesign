//! BVH motion capture files: parsing, and mapping a motion onto the
//! built-in avatars' skeleton ([`crate::skeleton`]).
//!
//! A BVH file is a joint hierarchy (offsets and rotation channels) and a
//! table of channel values per frame. Its skeleton differs from ours in
//! proportions, rest pose (often a T-pose, where ours hangs its arms in an
//! A) and joint names, so the motion is carried over by direction: each
//! frame, every one of our bones is turned to point the way the matching
//! BVH bone points, twisting as little as it can. The pelvis and spine
//! take their facing from the hips and shoulders. The root's travel is
//! left out, so the avatar moves in place. Files must be Y-up (as most
//! are); which way the BVH skeleton faces doesn't matter.

use glam::{Mat4, Quat, Vec3};
use od_core::pose::Joint;

use crate::skeleton::Bone;

/// One rotation or position channel of a joint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Xposition,
    Yposition,
    Zposition,
    Xrotation,
    Yrotation,
    Zrotation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BvhJoint {
    pub name: String,
    pub parent: Option<usize>,
    pub offset: Vec3,
    pub channels: Vec<Channel>,
    /// Where an `End Site` child sits, relative to this joint.
    pub end_site: Option<Vec3>,
}

/// A parsed BVH file.
#[derive(Debug, Clone, PartialEq)]
pub struct Bvh {
    /// Parents before children, in file order.
    pub joints: Vec<BvhJoint>,
    /// Seconds per frame.
    pub frame_time: f32,
    /// One value per channel, joints in order, for each frame.
    pub frames: Vec<Vec<f32>>,
}

/// Files bigger than this are refused rather than parsed.
const MAX_FRAMES: usize = 200_000;

impl Bvh {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut tokens = text.split_whitespace().peekable();
        let mut next = |what: &str| {
            tokens
                .next()
                .ok_or_else(|| format!("the file ends early (expected {what})"))
        };
        if next("HIERARCHY")? != "HIERARCHY" {
            return Err("not a BVH file (no HIERARCHY)".into());
        }
        let mut joints: Vec<BvhJoint> = Vec::new();
        // Joints whose `{` is open, innermost last; `None` for an end site.
        let mut open: Vec<Option<usize>> = Vec::new();
        let number = |t: &str| {
            t.parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("expected a number, found {t:?}"))
        };
        loop {
            let token = next("a joint")?;
            match token {
                "ROOT" | "JOINT" => {
                    if (token == "ROOT") != open.is_empty() {
                        return Err(format!("{token} in the wrong place"));
                    }
                    let name = next("a joint name")?.to_owned();
                    let parent = open.last().copied().flatten();
                    if next("{")? != "{" {
                        return Err(format!("expected {{ after {name}"));
                    }
                    joints.push(BvhJoint {
                        name,
                        parent,
                        offset: Vec3::ZERO,
                        channels: Vec::new(),
                        end_site: None,
                    });
                    open.push(Some(joints.len() - 1));
                }
                "End" => {
                    next("Site")?;
                    if next("{")? != "{" {
                        return Err("expected { after End Site".into());
                    }
                    open.push(None);
                }
                "OFFSET" => {
                    let v = Vec3::new(
                        number(next("an offset")?)?,
                        number(next("an offset")?)?,
                        number(next("an offset")?)?,
                    );
                    match open.last() {
                        Some(Some(j)) => joints[*j].offset = v,
                        Some(None) => {
                            let owner = open.iter().rev().find_map(|o| *o);
                            if let Some(j) = owner {
                                joints[j].end_site = Some(v);
                            }
                        }
                        None => return Err("OFFSET outside a joint".into()),
                    }
                }
                "CHANNELS" => {
                    let Some(Some(j)) = open.last().copied() else {
                        return Err("CHANNELS outside a joint".into());
                    };
                    let count = next("a channel count")?
                        .parse::<usize>()
                        .map_err(|_| "bad channel count".to_string())?;
                    if count > 6 {
                        return Err("more than 6 channels on a joint".into());
                    }
                    for _ in 0..count {
                        let channel = match next("a channel")?.to_ascii_lowercase().as_str() {
                            "xposition" => Channel::Xposition,
                            "yposition" => Channel::Yposition,
                            "zposition" => Channel::Zposition,
                            "xrotation" => Channel::Xrotation,
                            "yrotation" => Channel::Yrotation,
                            "zrotation" => Channel::Zrotation,
                            other => return Err(format!("unknown channel {other}")),
                        };
                        joints[j].channels.push(channel);
                    }
                }
                "}" => {
                    open.pop().ok_or("unbalanced }")?;
                    if open.is_empty() {
                        break;
                    }
                }
                other => return Err(format!("unexpected {other:?} in the hierarchy")),
            }
        }
        if next("MOTION")? != "MOTION" {
            return Err("expected MOTION after the hierarchy".into());
        }
        if next("Frames:")? != "Frames:" {
            return Err("expected Frames:".into());
        }
        let count = next("a frame count")?
            .parse::<usize>()
            .map_err(|_| "bad frame count".to_string())?;
        if count > MAX_FRAMES {
            return Err(format!("too many frames ({count})"));
        }
        if (next("Frame")?, next("Time:")?) != ("Frame", "Time:") {
            return Err("expected Frame Time:".into());
        }
        let frame_time = number(next("a frame time")?)?;
        if frame_time <= 0.0 {
            return Err("the frame time must be positive".into());
        }
        let width: usize = joints.iter().map(|j| j.channels.len()).sum();
        let mut frames = Vec::with_capacity(count);
        for f in 0..count {
            let mut row = Vec::with_capacity(width);
            for _ in 0..width {
                let t = next("frame values")
                    .map_err(|_| format!("frame {} of {count} is missing", f + 1))?;
                row.push(number(t)?);
            }
            frames.push(row);
        }
        if frames.is_empty() {
            return Err("the file has no frames".into());
        }
        Ok(Self {
            joints,
            frame_time,
            frames,
        })
    }

    /// Joint positions in the file's space for one frame (`None`: the rest
    /// pose, every channel zero), and each joint's end site if it has one.
    pub fn positions(&self, frame: Option<usize>) -> (Vec<Vec3>, Vec<Option<Vec3>>) {
        let values = frame.and_then(|f| self.frames.get(f));
        let mut world: Vec<Mat4> = Vec::with_capacity(self.joints.len());
        let mut at = 0;
        for joint in &self.joints {
            let mut translation = joint.offset;
            let mut rotation = Quat::IDENTITY;
            for &channel in &joint.channels {
                let v = values.map_or(0.0, |row| row.get(at).copied().unwrap_or(0.0));
                at += 1;
                let r = v.to_radians();
                match channel {
                    Channel::Xposition => translation.x += v,
                    Channel::Yposition => translation.y += v,
                    Channel::Zposition => translation.z += v,
                    Channel::Xrotation => rotation *= Quat::from_rotation_x(r),
                    Channel::Yrotation => rotation *= Quat::from_rotation_y(r),
                    Channel::Zrotation => rotation *= Quat::from_rotation_z(r),
                }
            }
            // The root's offset and position channels both place it; the
            // rest place a joint from its parent.
            let local = Mat4::from_rotation_translation(rotation, translation);
            let m = match joint.parent {
                Some(p) => world[p] * local,
                None => local,
            };
            world.push(m);
        }
        let positions = world.iter().map(|m| m.w_axis.truncate()).collect();
        let ends = self
            .joints
            .iter()
            .zip(&world)
            .map(|(j, m)| j.end_site.map(|e| m.transform_point3(e)))
            .collect();
        (positions, ends)
    }
}

/// A motion carried over onto an avatar skeleton: per frame, one rotation
/// per bone (as [`crate::skeleton::Rig::skin`] takes them).
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    pub frame_time: f32,
    pub frames: Vec<Vec<Quat>>,
    /// Our joints that found a match in the file; the rest stay at rest.
    pub matched: Vec<Joint>,
}

impl Motion {
    /// Length in seconds, from the first frame to the last.
    pub fn duration(&self) -> f32 {
        self.frame_time * (self.frames.len().saturating_sub(1)) as f32
    }

    /// The rotations at `time` seconds, between the two nearest frames;
    /// held at the ends.
    pub fn rotations_at(&self, time: f32) -> Vec<Quat> {
        let last = self.frames.len().saturating_sub(1);
        let f = (time / self.frame_time).clamp(0.0, last as f32);
        let i = (f.floor() as usize).min(last);
        let j = (i + 1).min(last);
        crate::skeleton::blend(&self.frames[i], &self.frames[j], f - i as f32)
    }
}

/// A BVH joint name boiled down to a side and a plain lowercase name:
/// `mixamorig:LeftForeArm` → (left, "forearm"), `lShldr` → (left,
/// "shldr"), `upperarm_r` → (right, "upperarm").
fn split_name(name: &str) -> (i8, String) {
    let tail = name.rsplit(':').next().unwrap_or(name);
    let plain: String = tail
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    for (word, side) in [("left", 1), ("right", -1)] {
        if let Some(rest) = plain.strip_prefix(word) {
            return (side, rest.to_owned());
        }
        if let Some(rest) = plain.strip_suffix(word) {
            return (side, rest.to_owned());
        }
    }
    // A single letter, as in lShldr or upperarm_l: only for names that are
    // known limb parts, so "leg" or "root" aren't split.
    let known = |base: &str| LIMB_NAMES.iter().any(|names| names.contains(&base));
    for (letter, side) in [('l', 1), ('r', -1)] {
        if let Some(rest) = plain.strip_prefix(letter).filter(|r| known(r)) {
            return (side, rest.to_owned());
        }
        if let Some(rest) = plain.strip_suffix(letter).filter(|r| known(r)) {
            return (side, rest.to_owned());
        }
    }
    (0, plain)
}

/// Names a limb joint goes by, best first: upper arm (at the shoulder),
/// forearm (elbow), hand (wrist), thigh (hip), shin (knee), foot (ankle).
const LIMB_NAMES: [&[&str]; 6] = [
    &["arm", "upperarm", "uparm", "shldr", "shoulder"],
    &["forearm", "lowerarm", "elbow"],
    &["hand", "wrist"],
    &["upleg", "upperleg", "thigh", "hip"],
    &["leg", "lowerleg", "shin", "knee", "calf"],
    &["foot", "ankle"],
];

/// Names for the joints down the middle, best first.
const PELVIS_NAMES: &[&str] = &["hips", "hip", "pelvis", "root"];
const SPINE_NAMES: &[&str] = &["spine", "lowerback", "abdomen", "spine1", "torso", "chest"];
const NECK_NAMES: &[&str] = &["neck", "neck1"];
const HEAD_NAMES: &[&str] = &["head"];

/// Which BVH joint stands for each of our joints, and for the head.
struct Mapping {
    joints: [Option<usize>; Joint::COUNT],
    head: Option<usize>,
}

fn map_joints(bvh: &Bvh) -> Mapping {
    let names: Vec<(i8, String)> = bvh.joints.iter().map(|j| split_name(&j.name)).collect();
    let find = |side: i8, candidates: &[&str]| {
        candidates
            .iter()
            .find_map(|c| names.iter().position(|(s, n)| *s == side && n == c))
    };
    let mut joints = [None; Joint::COUNT];
    joints[Joint::Pelvis.index()] =
        find(0, PELVIS_NAMES).or_else(|| bvh.joints.iter().position(|j| j.parent.is_none()));
    joints[Joint::Spine.index()] = find(0, SPINE_NAMES);
    joints[Joint::Neck.index()] = find(0, NECK_NAMES);
    let limbs = [
        (
            1,
            [
                Joint::LeftShoulder,
                Joint::LeftElbow,
                Joint::LeftWrist,
                Joint::LeftHip,
                Joint::LeftKnee,
                Joint::LeftAnkle,
            ],
        ),
        (
            -1,
            [
                Joint::RightShoulder,
                Joint::RightElbow,
                Joint::RightWrist,
                Joint::RightHip,
                Joint::RightKnee,
                Joint::RightAnkle,
            ],
        ),
    ];
    for (side, ours) in limbs {
        for (joint, names) in ours.into_iter().zip(LIMB_NAMES) {
            joints[joint.index()] = find(side, names);
        }
    }
    Mapping {
        joints,
        head: find(0, HEAD_NAMES),
    }
}

/// The turn about Y that brings `from` (flattened) onto `to`; always
/// about Y, even for opposite directions.
fn yaw_between(from: Vec3, to: Vec3) -> Quat {
    // A turn by θ about Y takes +X to (cos θ, 0, -sin θ).
    let angle = |v: Vec3| (-v.z).atan2(v.x);
    Quat::from_rotation_y(angle(to) - angle(from))
}

/// A rotation frame from an up direction and a direction to the left.
fn frame(up: Vec3, left: Vec3) -> Option<Quat> {
    let y = up.try_normalize()?;
    let x = (left - y * left.dot(y)).try_normalize()?;
    let z = x.cross(y);
    Some(Quat::from_mat3(&glam::Mat3::from_cols(x, y, z)))
}

/// Carry `bvh`'s motion onto a skeleton with one bone per [`Joint`] (an
/// avatar rig's [`crate::skeleton::Rig::bones`]).
pub fn retarget(bvh: &Bvh, bones: &[Bone]) -> Result<Motion, String> {
    if bones.len() != Joint::COUNT {
        return Err("only the built-in avatars can play motion".into());
    }
    let map = map_joints(bvh);
    let matched: Vec<Joint> = Joint::ALL
        .into_iter()
        .filter(|j| map.joints[j.index()].is_some())
        .collect();
    let has = |j: Joint| map.joints[j.index()].is_some();
    let limbs_found = [
        Joint::LeftShoulder,
        Joint::RightShoulder,
        Joint::LeftHip,
        Joint::RightHip,
    ]
    .into_iter()
    .all(has);
    if !limbs_found {
        return Err(format!(
            "its joints ({}) don't match the avatar's: it needs hips, \
             upper legs and upper arms on both sides",
            bvh.joints
                .iter()
                .take(8)
                .map(|j| j.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Turn the file's skeleton to face +Z like ours: its left hip on +X.
    let (rest, rest_ends) = bvh.positions(None);
    let bvh_at = |positions: &[Vec3], j: Joint| map.joints[j.index()].map(|i| positions[i]);
    let across = |p: &[Vec3]| bvh_at(p, Joint::LeftHip).zip(bvh_at(p, Joint::RightHip));
    let (l, r) = across(&rest).expect("checked above");
    let facing = yaw_between(l - r, Vec3::X);

    // Where each of our bones points to in the file: the matching joint of
    // its child, or for the last joint of a limb, that joint's first child
    // or end site.
    let tip = |positions: &[Vec3], ends: &[Option<Vec3>], j: Joint| -> Option<Vec3> {
        let next = match j {
            Joint::Pelvis => Some(Joint::Spine),
            Joint::Spine => Some(Joint::Neck),
            Joint::Neck => None,
            Joint::LeftShoulder => Some(Joint::LeftElbow),
            Joint::LeftElbow => Some(Joint::LeftWrist),
            Joint::RightShoulder => Some(Joint::RightElbow),
            Joint::RightElbow => Some(Joint::RightWrist),
            Joint::LeftHip => Some(Joint::LeftKnee),
            Joint::LeftKnee => Some(Joint::LeftAnkle),
            Joint::RightHip => Some(Joint::RightKnee),
            Joint::RightKnee => Some(Joint::RightAnkle),
            _ => None,
        };
        if let Some(p) = next.and_then(|n| bvh_at(positions, n)) {
            return Some(p);
        }
        if j == Joint::Neck {
            if let Some(h) = map.head {
                return Some(positions[h]);
            }
        }
        let i = map.joints[j.index()]?;
        bvh.joints
            .iter()
            .position(|c| c.parent == Some(i))
            .map(|c| positions[c])
            .or(ends[i])
    };
    // The shoulders, for the spine's facing: the upper arm joints.
    let shoulders =
        |p: &[Vec3]| bvh_at(p, Joint::LeftShoulder).zip(bvh_at(p, Joint::RightShoulder));

    let rest_frame = |positions: &[Vec3], ends: &[Option<Vec3>], j: Joint, left: Vec3| {
        let start = bvh_at(positions, j)?;
        frame(tip(positions, ends, j)? - start, left)
    };
    let pelvis_rest = rest_frame(&rest, &rest_ends, Joint::Pelvis, l - r);
    let spine_rest =
        shoulders(&rest).and_then(|(sl, sr)| rest_frame(&rest, &rest_ends, Joint::Spine, sl - sr));

    let mut frames = Vec::with_capacity(bvh.frames.len());
    for f in 0..bvh.frames.len() {
        let (raw, raw_ends) = bvh.positions(Some(f));
        let positions: Vec<Vec3> = raw.iter().map(|p| facing * *p).collect();
        let ends: Vec<Option<Vec3>> = raw_ends.iter().map(|e| e.map(|e| facing * e)).collect();
        let rest_facing: Vec<Vec3> = rest.iter().map(|p| facing * *p).collect();
        let rest_ends_facing: Vec<Option<Vec3>> =
            rest_ends.iter().map(|e| e.map(|e| facing * e)).collect();

        let mut global = [Quat::IDENTITY; Joint::COUNT];
        let mut local = vec![Quat::IDENTITY; Joint::COUNT];
        for joint in Joint::ALL {
            let i = joint.index();
            let parent = joint.parent().map_or(Quat::IDENTITY, |p| global[p.index()]);
            let turned = match joint {
                // The pelvis and spine: how the file's hips (or shoulders)
                // have turned from its rest pose.
                Joint::Pelvis | Joint::Spine => {
                    let (rest_turn, left) = if joint == Joint::Pelvis {
                        (pelvis_rest, across(&positions).map(|(l, r)| l - r))
                    } else {
                        (spine_rest, shoulders(&positions).map(|(l, r)| l - r))
                    };
                    let now = left.and_then(|left| rest_frame(&positions, &ends, joint, left));
                    match (rest_turn, now) {
                        (Some(rest_turn), Some(now)) => Some(now * (facing * rest_turn).inverse()),
                        _ => None,
                    }
                }
                // Feet: turned as the file's foot turned from its rest
                // pose, since its feet and ours rest differently placed.
                Joint::LeftAnkle | Joint::RightAnkle => {
                    let dir = |p: &[Vec3], e: &[Option<Vec3>]| {
                        Some(tip(p, e, joint)? - bvh_at(p, joint)?)
                    };
                    match (dir(&rest_facing, &rest_ends_facing), dir(&positions, &ends)) {
                        (Some(a), Some(b)) => {
                            let ours = parent * Vec3::Z;
                            let want = Quat::from_rotation_arc(
                                a.normalize_or(Vec3::Z),
                                b.normalize_or(Vec3::Z),
                            ) * Vec3::Z;
                            Some(Quat::from_rotation_arc(ours, want.normalize()) * parent)
                        }
                        _ => None,
                    }
                }
                // Everything else points along the file's bone.
                _ => {
                    let start = bvh_at(&positions, joint);
                    let end = tip(&positions, &ends, joint);
                    match start.zip(end).and_then(|(s, e)| (e - s).try_normalize()) {
                        Some(want) => {
                            let now = (parent * bones[i].axis).normalize();
                            Some(Quat::from_rotation_arc(now, want) * parent)
                        }
                        None => None,
                    }
                }
            };
            global[i] = turned.unwrap_or(parent).normalize();
            local[i] = (parent.inverse() * global[i]).normalize();
        }
        frames.push(local);
    }
    Ok(Motion {
        frame_time: bvh.frame_time,
        frames,
        matched,
    })
}

/// A small skeleton written for OpenDesign (not taken from any capture),
/// with CMU-style joint names, standing in a T-pose facing +Z,
/// centimetres, Y up. Joints in file order: Hips, Spine, Neck, Head,
/// LeftArm, LeftForeArm, LeftHand, RightArm, RightForeArm, RightHand,
/// LeftUpLeg, LeftLeg, LeftFoot, RightUpLeg, RightLeg, RightFoot; the hips
/// have six channels, the rest three (Z, X, Y rotation).
pub const SAMPLE_HIERARCHY: &str = "HIERARCHY
ROOT Hips
{
  OFFSET 0 95 0
  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation
  JOINT Spine
  {
    OFFSET 0 10 0
    CHANNELS 3 Zrotation Xrotation Yrotation
    JOINT Neck
    {
      OFFSET 0 45 0
      CHANNELS 3 Zrotation Xrotation Yrotation
      JOINT Head
      {
        OFFSET 0 10 0
        CHANNELS 3 Zrotation Xrotation Yrotation
        End Site
        {
          OFFSET 0 18 0
        }
      }
    }
    JOINT LeftArm
    {
      OFFSET 18 40 0
      CHANNELS 3 Zrotation Xrotation Yrotation
      JOINT LeftForeArm
      {
        OFFSET 28 0 0
        CHANNELS 3 Zrotation Xrotation Yrotation
        JOINT LeftHand
        {
          OFFSET 25 0 0
          CHANNELS 3 Zrotation Xrotation Yrotation
          End Site
          {
            OFFSET 15 0 0
          }
        }
      }
    }
    JOINT RightArm
    {
      OFFSET -18 40 0
      CHANNELS 3 Zrotation Xrotation Yrotation
      JOINT RightForeArm
      {
        OFFSET -28 0 0
        CHANNELS 3 Zrotation Xrotation Yrotation
        JOINT RightHand
        {
          OFFSET -25 0 0
          CHANNELS 3 Zrotation Xrotation Yrotation
          End Site
          {
            OFFSET -15 0 0
          }
        }
      }
    }
  }
  JOINT LeftUpLeg
  {
    OFFSET 9 0 0
    CHANNELS 3 Zrotation Xrotation Yrotation
    JOINT LeftLeg
    {
      OFFSET 0 -45 0
      CHANNELS 3 Zrotation Xrotation Yrotation
      JOINT LeftFoot
      {
        OFFSET 0 -42 0
        CHANNELS 3 Zrotation Xrotation Yrotation
        End Site
        {
          OFFSET 0 -6 14
        }
      }
    }
  }
  JOINT RightUpLeg
  {
    OFFSET -9 0 0
    CHANNELS 3 Zrotation Xrotation Yrotation
    JOINT RightLeg
    {
      OFFSET 0 -45 0
      CHANNELS 3 Zrotation Xrotation Yrotation
      JOINT RightFoot
      {
        OFFSET 0 -42 0
        CHANNELS 3 Zrotation Xrotation Yrotation
        End Site
        {
          OFFSET 0 -6 14
        }
      }
    }
  }
}
";

/// A one-second walk in place on [`SAMPLE_HIERARCHY`], generated from a
/// few sine waves: a built-in motion to try without a file.
pub fn walk_cycle() -> String {
    const FRAMES: usize = 30;
    let mut text = format!(
        "{SAMPLE_HIERARCHY}MOTION\nFrames: {}\nFrame Time: {}\n",
        FRAMES + 1,
        1.0 / FRAMES as f32
    );
    for i in 0..=FRAMES {
        let phase = i as f32 / FRAMES as f32 * std::f32::consts::TAU;
        let (sin, cos) = phase.sin_cos();
        // [Z, X, Y] rotation per joint after the hips, in file order.
        let mut joints = [[0.0f32; 3]; 15];
        // Thighs swing opposite ways (negative X is forward); knees bend
        // most as the leg swings through.
        joints[9] = [0.0, -25.0 * sin, 0.0];
        joints[12] = [0.0, 25.0 * sin, 0.0];
        joints[10] = [0.0, 8.0 + 22.0 * (0.5 - 0.5 * cos), 0.0];
        joints[13] = [0.0, 8.0 + 22.0 * (0.5 + 0.5 * cos), 0.0];
        // Arms down from the T and swinging against the legs, elbows a
        // little bent.
        joints[3] = [-75.0, 0.0, 20.0 * sin];
        joints[6] = [75.0, 0.0, 20.0 * sin];
        joints[4] = [0.0, 0.0, -15.0];
        joints[7] = [0.0, 0.0, 15.0];
        // The spine turns a little against the hips.
        joints[0] = [0.0, 0.0, -4.0 * sin];
        let hips = [
            0.0,
            95.0 + 1.5 * (2.0 * phase).cos(),
            0.0,
            0.0,
            0.0,
            4.0 * sin,
        ];
        let values = hips.into_iter().chain(joints.into_iter().flatten());
        let row: Vec<String> = values.map(|v| format!("{v:.3}")).collect();
        text += &row.join(" ");
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::AvatarKind;
    use crate::skeleton::Rig;

    /// The sample skeleton at rest, then the left elbow bent forward 90°
    /// and the right knee bent 60°, half a second apart.
    fn t_pose() -> String {
        format!("{SAMPLE_HIERARCHY}MOTION\nFrames: 2\nFrame Time: 0.5\n")
    }

    /// Channel values for one frame: 6 for the hips, 3 for each of the
    /// other 15 joints, with some set.
    fn row(set: &[(usize, [f32; 3])]) -> String {
        let mut values = vec![0.0f32; 6 + 15 * 3];
        values[1] = 95.0;
        for (joint, zxy) in set {
            let at = 6 + (joint - 1) * 3;
            values[at..at + 3].copy_from_slice(zxy);
        }
        values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    // Joint numbers in file order: Hips 0, Spine 1, Neck 2, Head 3,
    // LeftArm 4, LeftForeArm 5, LeftHand 6, RightArm 7, RightForeArm 8,
    // RightHand 9, LeftUpLeg 10, LeftLeg 11, LeftFoot 12, RightUpLeg 13,
    // RightLeg 14, RightFoot 15.
    fn two_frames() -> String {
        format!(
            "{}{}\n{}\n",
            t_pose(),
            row(&[]),
            // Elbow: Y -90 turns the forearm from +X to +Z. Knee: X 60
            // turns the shin back.
            row(&[(5, [0.0, 0.0, -90.0]), (14, [0.0, 60.0, 0.0])])
        )
    }

    fn rig() -> Rig {
        let kind = AvatarKind::Female;
        Rig::avatar(kind, kind.default_measurements())
    }

    #[test]
    fn parses_the_hierarchy_and_frames() {
        let bvh = Bvh::parse(&two_frames()).unwrap();
        assert_eq!(bvh.joints.len(), 16);
        assert_eq!(bvh.joints[0].channels.len(), 6);
        assert_eq!(bvh.joints[5].name, "LeftForeArm");
        assert_eq!(bvh.joints[5].parent, Some(4));
        assert_eq!(bvh.joints[3].end_site, Some(Vec3::new(0.0, 18.0, 0.0)));
        assert_eq!(bvh.frames.len(), 2);
        assert_eq!(bvh.frame_time, 0.5);
        let (rest, _) = bvh.positions(None);
        assert_eq!(
            rest[6],
            Vec3::new(18.0 + 28.0 + 25.0, 95.0 + 10.0 + 40.0, 0.0)
        );
        let (bent, _) = bvh.positions(Some(1));
        // The left hand is now in front of the elbow.
        assert!((bent[6] - (bent[5] + Vec3::new(0.0, 0.0, 25.0))).length() < 1e-3);
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        for text in [
            "",
            "hello",
            "HIERARCHY ROOT Hips { OFFSET 0 0 }",
            "HIERARCHY ROOT Hips { OFFSET 0 0 0 CHANNELS 1 Wrotation } MOTION Frames: 1 Frame Time: 1 0",
            "HIERARCHY ROOT Hips { OFFSET 0 0 0 CHANNELS 1 Xrotation } MOTION Frames: 2 Frame Time: 0.1 0",
            "HIERARCHY ROOT Hips { OFFSET 0 0 0 CHANNELS 1 Xrotation } MOTION Frames: 1 Frame Time: 0 1",
        ] {
            assert!(Bvh::parse(text).is_err(), "{text}");
        }
        // Parses, but nothing in it is an arm or a leg.
        let tail = "HIERARCHY ROOT Tail { OFFSET 0 0 0 CHANNELS 1 Xrotation } \
                    MOTION Frames: 1 Frame Time: 0.1 5";
        let bvh = Bvh::parse(tail).unwrap();
        assert!(retarget(&bvh, rig().bones()).is_err());
    }

    #[test]
    fn names_are_matched_across_conventions() {
        assert_eq!(split_name("mixamorig:LeftForeArm"), (1, "forearm".into()));
        assert_eq!(split_name("lShldr"), (1, "shldr".into()));
        assert_eq!(split_name("upperarm_r"), (-1, "upperarm".into()));
        assert_eq!(split_name("RightUpLeg"), (-1, "upleg".into()));
        assert_eq!(split_name("Hips"), (0, "hips".into()));
        // Not split: "leg" alone isn't a side and a limb.
        assert_eq!(split_name("Leg"), (0, "leg".into()));
    }

    #[test]
    fn a_t_pose_file_raises_the_arms_and_bends_where_it_bends() {
        let rig = rig();
        let bvh = Bvh::parse(&two_frames()).unwrap();
        let motion = retarget(&bvh, rig.bones()).unwrap();
        assert_eq!(motion.matched.len(), Joint::COUNT);
        assert_eq!(motion.duration(), 0.5);

        let at = |frame: usize, j: Joint| rig.joint_positions(&motion.frames[frame])[j.index()];
        // Frame 0: the file's T-pose; our arms come up level.
        for (shoulder, wrist) in [
            (Joint::LeftShoulder, Joint::LeftWrist),
            (Joint::RightShoulder, Joint::RightWrist),
        ] {
            let (s, w) = (at(0, shoulder), at(0, wrist));
            assert!((w.y - s.y).abs() < 0.01, "level: {s:?} {w:?}");
            assert!((w.x - s.x).abs() > 0.4);
        }
        // Legs stay straight down, feet flat.
        let (hip, ankle) = (at(0, Joint::LeftHip), at(0, Joint::LeftAnkle));
        assert!((hip.x - ankle.x).abs() < 0.01 && (hip.z - ankle.z).abs() < 0.01);
        let foot = motion.frames[0][Joint::LeftAnkle.index()];
        assert!(foot.abs_diff_eq(Quat::IDENTITY, 1e-3), "{foot:?}");

        // Frame 1: the left forearm points forward, the right shin back.
        let (e, w) = (at(1, Joint::LeftElbow), at(1, Joint::LeftWrist));
        let fore = (w - e).normalize();
        assert!(fore.z > 0.99, "{fore:?}");
        let (k, a) = (at(1, Joint::RightKnee), at(1, Joint::RightAnkle));
        let shin = (a - k).normalize();
        assert!(shin.z < -0.8 && shin.y < -0.4, "{shin:?}");

        // Half way between the frames, half way bent.
        let half = rig.joint_positions(&motion.rotations_at(0.25));
        let fore = (half[Joint::LeftWrist.index()] - half[Joint::LeftElbow.index()]).normalize();
        assert!(fore.z > 0.5 && fore.z < 0.9, "{fore:?}");
    }

    #[test]
    fn a_file_facing_the_other_way_maps_the_same() {
        // Mirror the whole file front to back and left to right (a turn of
        // 180° about Y), and rename its sides Mixamo-style.
        let mut text = String::new();
        let mut motion_part = false;
        for line in two_frames().lines() {
            let t = line.trim();
            if t.starts_with("MOTION") {
                motion_part = true;
            }
            if let (false, Some(rest)) = (motion_part, t.strip_prefix("OFFSET ")) {
                let v: Vec<f32> = rest.split(' ').map(|n| n.parse().unwrap()).collect();
                text += &format!("OFFSET {} {} {}\n", -v[0], v[1], -v[2]);
            } else if let Some(name) = t.strip_prefix("JOINT ") {
                text += &format!("JOINT mixamorig:{name}\n");
            } else {
                text += &format!("{t}\n");
            }
        }
        let bvh = Bvh::parse(&text).unwrap();
        let rig = rig();
        let motion = retarget(&bvh, rig.bones()).unwrap();
        // At rest the arms come up level and nothing is turned round.
        let p = rig.joint_positions(&motion.frames[0]);
        let (s, w) = (p[Joint::LeftShoulder.index()], p[Joint::LeftWrist.index()]);
        assert!((w.y - s.y).abs() < 0.01 && w.x > s.x + 0.4, "{s:?} {w:?}");
        let hips = motion.frames[0][Joint::Pelvis.index()];
        assert!(hips.angle_between(Quat::IDENTITY) < 1e-3, "{hips:?}");
    }

    #[test]
    fn the_built_in_walk_swings_arms_and_legs() {
        let bvh = Bvh::parse(&walk_cycle()).unwrap();
        assert_eq!(bvh.frames.len(), 31);
        let rig = rig();
        let motion = retarget(&bvh, rig.bones()).unwrap();
        assert!((motion.duration() - 1.0).abs() < 1e-4);
        let at = |t: f32| rig.joint_positions(&motion.rotations_at(t));
        // A quarter in, the left foot is forward and the right back, and
        // the arms swing the other way.
        let p = at(0.25);
        let (la, ra) = (p[Joint::LeftAnkle.index()], p[Joint::RightAnkle.index()]);
        assert!(la.z > 0.1 && ra.z < -0.1, "{la:?} {ra:?}");
        let (lw, rw) = (p[Joint::LeftWrist.index()], p[Joint::RightWrist.index()]);
        assert!(lw.z < rw.z - 0.1, "{lw:?} {rw:?}");
        // Arms hang down, not out in a T.
        let s = p[Joint::LeftShoulder.index()];
        assert!(lw.y < s.y - 0.3, "{lw:?}");
        // A quarter later the legs have swapped.
        let p = at(0.75);
        assert!(p[Joint::LeftAnkle.index()].z < -0.1);
    }
}
