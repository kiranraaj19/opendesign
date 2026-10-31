//! Posing the avatar from the 3D view: the Pose panel (built-in poses and
//! per-joint angles, kept in the document through [`SetPose`] so they are
//! saved and undoable) and moving the body to the pose.
//!
//! While draping, the body moves to a new pose over [`TRANSITION`]
//! seconds and the cloth, stepped every frame against the moving body,
//! comes along: a garment draped in one pose ends up on the next. When
//! not draping it simply jumps.

use egui::{Pos2, Rect, UiBuilder};
use od_core::pose::{builtin_poses, Joint, JointAngles, Pose, SetPose, REST_ARM_ANGLE};
use od_render::glam::Quat;
use od_render::skeleton::{blend, pose_rotations};

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
}

impl Default for Posing {
    fn default() -> Self {
        Self {
            open: false,
            joint: Joint::LeftShoulder,
            preview: None,
            applied: None,
            moving: None,
        }
    }
}

impl Posing {
    /// Whether the body is on its way to a new pose.
    pub fn is_moving(&self) -> bool {
        self.moving.is_some()
    }
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
    let v = &mut s.viewport;
    let Some(rig) = v.avatar.rig() else {
        v.posing.applied = None;
        v.posing.moving = None;
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
