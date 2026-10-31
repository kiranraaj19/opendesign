//! Posing the avatar from the 3D view: the Pose panel (built-in poses and
//! per-joint angles, kept in the document through [`SetPose`] so they are
//! saved and undoable) and moving the body to the pose.
//!
//! While draping, the body moves to a new pose over [`TRANSITION`]
//! seconds and the cloth, stepped every frame against the moving body,
//! comes along: a garment draped in one pose ends up on the next. When
//! not draping it simply jumps.
//!
//! A BVH motion (a file, or the built-in walk) can be loaded and played
//! instead of the pose. It's session state, like the avatar: not saved.
//! Played while draping, the cloth moves with the body.

use egui::{Pos2, Rect, UiBuilder};
use od_core::pose::{builtin_poses, Joint, JointAngles, Pose, SetPose, REST_ARM_ANGLE};
use od_render::bvh::{retarget, Bvh, Motion};
use od_render::glam::Quat;
use od_render::skeleton::{blend, pose_rotations, Rig};

use crate::app::Session;

/// How long the body takes to reach a new pose while draping, in seconds.
pub const TRANSITION: f32 = 1.5;

/// The pose panel's state and the body's move between poses (session
/// state; the pose itself is in the document).
pub struct Posing {
    /// The Pose panel is showing.
    pub open: bool,
    /// The joint whose angles the panel edits.
    pub joint: Joint,
    /// A pose being dragged out on a slider, shown before it is executed.
    preview: Option<Pose>,
    /// The pose the body was last put in (or set moving towards), with the
    /// avatar's generation after, to spot a rebuilt avatar.
    applied: Option<(Pose, u64)>,
    moving: Option<Move>,
    /// A motion played instead of the pose.
    pub motion: Option<MotionPlayer>,
    /// Why the last motion file didn't load.
    pub motion_error: Option<String>,
}

impl Default for Posing {
    fn default() -> Self {
        Self {
            open: false,
            joint: Joint::LeftShoulder,
            preview: None,
            applied: None,
            moving: None,
            motion: None,
            motion_error: None,
        }
    }
}

impl Posing {
    /// Whether the body is on its way to a new pose.
    pub fn is_moving(&self) -> bool {
        self.moving.is_some()
    }

    /// Load a BVH motion from text and carry it over onto `rig`, ready to
    /// play from the start.
    pub fn load_motion(&mut self, name: &str, text: &str, rig: &Rig) -> Result<(), String> {
        let loaded = Bvh::parse(text).and_then(|bvh| {
            let motion = retarget(&bvh, rig.bones())?;
            Ok((bvh, motion))
        });
        match loaded {
            Ok((bvh, motion)) => {
                self.motion = Some(MotionPlayer {
                    name: name.to_owned(),
                    bvh,
                    motion,
                    time: 0.0,
                    playing: true,
                    looping: true,
                    shown: None,
                });
                self.motion_error = None;
                Ok(())
            }
            Err(e) => {
                let message = format!("Couldn't load {name}: {e}");
                self.motion_error = Some(message.clone());
                Err(message)
            }
        }
    }

    /// Stop playing the motion; the body goes back to the pose.
    pub fn stop_motion(&mut self) {
        self.motion = None;
        self.applied = None;
    }
}

/// A loaded motion and where it is in it.
pub struct MotionPlayer {
    pub name: String,
    bvh: Bvh,
    pub motion: Motion,
    /// Seconds from the start.
    pub time: f32,
    pub playing: bool,
    /// Start again at the end, rather than stop.
    pub looping: bool,
    /// The time the body was last put at.
    shown: Option<f32>,
}

/// Motion files bigger than this are refused.
const MAX_MOTION_BYTES: u64 = 64 * 1024 * 1024;

/// Ask for a BVH file and load it onto the avatar.
fn load_motion_dialog(s: &mut Session) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("BVH motion", &["bvh"])
        .pick_file()
    else {
        return;
    };
    let name = path
        .file_name()
        .map_or("motion".into(), |n| n.to_string_lossy().into_owned());
    let v = &mut s.viewport;
    let text = match std::fs::metadata(&path) {
        Ok(m) if m.len() > MAX_MOTION_BYTES => Err("the file is too big".to_string()),
        _ => std::fs::read_to_string(&path).map_err(|e| e.to_string()),
    };
    match (text, v.avatar.rig()) {
        (Ok(text), Some(rig)) => {
            // An error is kept in `motion_error` and shown in the panel.
            let _ = v.posing.load_motion(&name, &text, rig);
        }
        (Err(e), _) => v.posing.motion_error = Some(format!("Couldn't load {name}: {e}")),
        (_, None) => {}
    }
}

/// Play the loaded motion on the body. Returns false if there is none.
fn update_motion(ui: &egui::Ui, s: &mut Session, dt: f32, target: Pose) -> bool {
    let v = &mut s.viewport;
    let (Some(player), Some(rig)) = (v.posing.motion.as_mut(), v.avatar.rig()) else {
        return false;
    };
    let rebuilt = v
        .posing
        .applied
        .as_ref()
        .is_none_or(|(_, generation)| *generation != v.avatar.generation);
    if rebuilt {
        // A resized or swapped body has its joints elsewhere.
        if let Ok(motion) = retarget(&player.bvh, rig.bones()) {
            player.motion = motion;
        }
    }
    let duration = player.motion.duration();
    if player.playing {
        player.time += dt;
        if player.time > duration {
            if player.looping && duration > 0.0 {
                player.time %= duration;
            } else {
                player.time = duration;
                player.playing = false;
            }
        }
    }
    player.time = player.time.clamp(0.0, duration);
    if rebuilt || player.shown != Some(player.time) {
        let rotations = player.motion.rotations_at(player.time);
        player.shown = Some(player.time);
        v.posing.moving = None;
        v.avatar.set_rotations(rotations);
        v.posing.applied = Some((target, v.avatar.generation));
    }
    if v.posing.motion.as_ref().is_some_and(|p| p.playing) {
        ui.ctx().request_repaint();
    }
    true
}

/// The body turning from one set of bone rotations to another.
struct Move {
    from: Vec<Quat>,
    to: Vec<Quat>,
    elapsed: f32,
}

/// The pose the avatar should be in: one being dragged out, else the
/// document's.
fn target(s: &Session) -> Pose {
    s.viewport
        .posing
        .preview
        .clone()
        .unwrap_or_else(|| s.doc.avatar_pose.clone())
}

/// Bring the avatar's body to the pose it should be in: at once, or
/// gliding there while draping. Call every frame before the drape steps.
pub fn update(ui: &egui::Ui, s: &mut Session) {
    let dt = ui.input(|i| i.stable_dt).clamp(0.0, 1.0 / 30.0);
    let target = target(s);
    if update_motion(ui, s, dt, target.clone()) {
        return;
    }
    let v = &mut s.viewport;
    let Some(rig) = v.avatar.rig() else {
        v.posing.applied = None;
        v.posing.moving = None;
        v.posing.motion = None;
        return;
    };
    let rebuilt = v
        .posing
        .applied
        .as_ref()
        .is_none_or(|(_, generation)| *generation != v.avatar.generation);
    let changed = v
        .posing
        .applied
        .as_ref()
        .is_none_or(|(pose, _)| *pose != target);
    if rebuilt || changed {
        let to = pose_rotations(&target, rig.bones());
        let glide = changed && !rebuilt && v.draping && v.posing.preview.is_none();
        if glide {
            v.posing.moving = Some(Move {
                from: v.avatar.rotations().to_vec(),
                to,
                elapsed: 0.0,
            });
        } else {
            v.posing.moving = None;
            v.avatar.set_rotations(to);
        }
        v.posing.applied = Some((target, v.avatar.generation));
    }
    if let Some(m) = &mut v.posing.moving {
        m.elapsed += dt;
        let t = (m.elapsed / TRANSITION).min(1.0);
        // Ease in and out, so the cloth isn't jerked.
        let rotations = blend(&m.from, &m.to, t * t * (3.0 - 2.0 * t));
        if t >= 1.0 {
            v.posing.moving = None;
        }
        v.avatar.set_rotations(rotations);
        if let Some((_, generation)) = &mut v.posing.applied {
            *generation = v.avatar.generation;
        }
        ui.ctx().request_repaint();
    }
}

/// The Pose panel, at the left of the 3D view.
pub fn panel(ui: &mut egui::Ui, rect: Rect, s: &mut Session) {
    let width = 270.0;
    let panel = Rect::from_min_size(
        Pos2::new(rect.left() + 8.0, rect.top() + 8.0),
        egui::vec2(width, rect.height() - 16.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(panel), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("pose_panel")
                .show(ui, |ui| controls(ui, s));
        });
    });
}

/// What the Pose panel holds: built-in poses, then one joint's angles.
pub fn controls(ui: &mut egui::Ui, s: &mut Session) {
    ui.strong("Avatar pose");
    let posable = s.viewport.avatar.rig().is_some();
    if !posable {
        ui.weak("Loaded avatars can't be posed.");
    }
    if s.viewport.posing.is_moving() {
        ui.weak("Moving to the new pose; the cloth follows.");
    }
    ui.add_enabled_ui(posable, |ui| {
        motion_controls(ui, s);
        ui.separator();
    });
    let posing = posable && s.viewport.posing.motion.is_none();
    ui.add_enabled_ui(posing, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (name, pose) in builtin_poses() {
                let current = s.doc.avatar_pose == pose;
                if ui.selectable_label(current, name).clicked() && !current {
                    s.execute(SetPose::new(format!("Pose: {name}"), pose));
                }
            }
        });
        ui.separator();
        joint_controls(ui, s);
    });
}

/// Loading and playing a motion.
fn motion_controls(ui: &mut egui::Ui, s: &mut Session) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Load BVH motion…")
            .on_hover_text("Play a motion capture file on the avatar, in place")
            .clicked()
        {
            load_motion_dialog(s);
        }
        if ui
            .button("Walk in place")
            .on_hover_text("A built-in walk cycle")
            .clicked()
        {
            let v = &mut s.viewport;
            if let Some(rig) = v.avatar.rig() {
                let walk = od_render::bvh::walk_cycle();
                let _ = v.posing.load_motion("Walk in place", &walk, rig);
            }
        }
    });
    let v = &mut s.viewport;
    if let Some(error) = &v.posing.motion_error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let Some(player) = &mut v.posing.motion else {
        return;
    };
    ui.label(format!(
        "Motion: {} ({} of {} joints matched)",
        player.name,
        player.motion.matched.len(),
        Joint::COUNT
    ));
    let mut stop = false;
    ui.horizontal(|ui| {
        let label = if player.playing { "Pause" } else { "Play" };
        if ui.button(label).clicked() {
            if !player.playing && player.time >= player.motion.duration() {
                player.time = 0.0;
            }
            player.playing = !player.playing;
        }
        ui.checkbox(&mut player.looping, "Loop");
        stop = ui
            .button("Stop motion")
            .on_hover_text("Go back to the pose")
            .clicked();
    });
    let duration = player.motion.duration();
    // A copy, since the slider rounds what it is given: only a change by
    // hand moves the motion (and pauses it).
    let mut time = player.time;
    let scrub = ui.add(
        egui::Slider::new(&mut time, 0.0..=duration)
            .suffix(" s")
            .max_decimals(2),
    );
    if scrub.changed() && (scrub.dragged() || scrub.clicked() || scrub.has_focus()) {
        player.time = time;
        player.playing = false;
    }
    ui.weak("Drape while it plays and the cloth moves with the body.");
    if stop {
        v.posing.stop_motion();
    }
}

fn joint_controls(ui: &mut egui::Ui, s: &mut Session) {
    let posing = &mut s.viewport.posing;
    egui::ComboBox::from_label("Joint")
        .selected_text(posing.joint.label())
        .show_ui(ui, |ui| {
            for joint in Joint::ALL {
                ui.selectable_value(&mut posing.joint, joint, joint.label());
            }
        });
    let joint = posing.joint;
    let mut pose = target(s);
    let mut angles = pose.get(joint);
    let (mut changed, mut dragging) = (false, false);
    let middle = joint.side() == 0.0;
    egui::Grid::new("joint_angles")
        .num_columns(2)
        .show(ui, |ui| {
            for (label, value, tip) in [
                (
                    "Bend",
                    &mut angles.bend,
                    "Forward: arms and legs swing forward, elbows and the spine bend, \
                     knees bend back, ankles lift the toes",
                ),
                (
                    if middle { "Lean" } else { "Spread" },
                    &mut angles.spread,
                    if middle {
                        "Lean towards the avatar's left"
                    } else {
                        "Out, away from the body"
                    },
                ),
                (
                    "Twist",
                    &mut angles.twist,
                    if middle {
                        "Turn towards the avatar's left"
                    } else {
                        "Turn outward about the bone"
                    },
                ),
            ] {
                ui.label(label).on_hover_text(tip);
                let response = ui.add(
                    egui::Slider::new(value, JointAngles::RANGE)
                        .suffix("°")
                        .max_decimals(0),
                );
                changed |= response.changed();
                dragging |= response.dragged() && !response.drag_stopped();
                ui.end_row();
            }
        });
    if matches!(joint, Joint::LeftShoulder | Joint::RightShoulder) {
        ui.weak(format!(
            "Angles are from the rest pose, where the arms hang {REST_ARM_ANGLE}° out."
        ));
    }
    let posing = &mut s.viewport.posing;
    if changed {
        pose.set(joint, angles);
        posing.preview = Some(pose.clone());
    }
    // One drag is one undo step: preview while dragging, then execute.
    if !dragging {
        if let Some(pose) = posing.preview.take() {
            if pose != s.doc.avatar_pose {
                s.execute(SetPose::new(
                    format!("Pose {}", joint.label().to_lowercase()),
                    pose,
                ));
            }
        }
    }

    ui.horizontal_wrapped(|ui| {
        let pose = s.doc.avatar_pose.clone();
        if ui
            .button("Reset joint")
            .on_hover_text("Put this joint back as it is at rest")
            .clicked()
            && !pose.get(joint).is_zero()
        {
            let pose = pose.clone().with(joint, JointAngles::default());
            s.execute(SetPose::new(format!("Reset {}", joint.label()), pose));
        }
        if !middle
            && ui
                .button("Copy to other side")
                .on_hover_text(format!(
                    "Give the {} the same angles",
                    joint.mirror().label()
                ))
                .clicked()
        {
            let copied = pose.clone().with(joint.mirror(), pose.get(joint));
            if copied != pose {
                s.execute(SetPose::new(format!("Copy {}", joint.label()), copied));
            }
        }
        if ui
            .button("Mirror pose")
            .on_hover_text("Swap the avatar's left and right")
            .clicked()
        {
            let mirrored = pose.mirrored();
            if mirrored != pose {
                s.execute(SetPose::new("Mirror pose", mirrored));
            }
        }
    });
}
