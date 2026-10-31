//! UI tests for posing the avatar: the Pose panel's built-in poses and
//! joint sliders, the pose kept through resizing, and the body gliding
//! to a new pose while draping.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use od_core::pose::{builtin_poses, Joint, JointAngles, Pose, SetPose};
use od_render::glam::Quat;
use od_render::skeleton::pose_rotations;

use super::OpenDesignApp;

fn harness() -> Harness<'static, OpenDesignApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 880.0))
        .build_eframe(|cc| OpenDesignApp::new(cc));
    h.run();
    h
}

fn named(name: &str) -> Pose {
    builtin_poses()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap()
        .1
}

fn mesh_width(mesh: &od_render::mesh::Mesh) -> f32 {
    mesh.bounds().unwrap().size().x
}

fn width(h: &Harness<'_, OpenDesignApp>) -> f32 {
    mesh_width(h.state().session.viewport.avatar.mesh())
}

fn open_panel(h: &mut Harness<'_, OpenDesignApp>) {
    h.get_by_label("Pose").click();
    h.run();
    assert!(h.state().session.viewport.posing.open);
}

#[test]
fn a_builtin_pose_poses_the_avatar_and_undoes() {
    let mut h = harness();
    let rest_width = width(&h);
    open_panel(&mut h);
    h.get_by_label("T-pose").click();
    h.run();
    h.run();
    assert_eq!(h.state().session.doc.avatar_pose, named("T-pose"));
    assert!(width(&h) > rest_width + 0.8, "arms out: {}", width(&h));
    assert_eq!(
        h.state().session.history.undo_label().as_deref(),
        Some("Pose: T-pose")
    );

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    h.run();
    assert!(h.state().session.doc.avatar_pose.is_rest());
    assert!((width(&h) - rest_width).abs() < 1e-5);
}

#[test]
fn dragging_a_joint_slider_is_one_undo_step() {
    let mut h = harness();
    open_panel(&mut h);
    h.state_mut().session.viewport.posing.joint = Joint::LeftElbow;
    h.run();
    let undo_steps = h.state().session.history.done_labels().len();
    let bend = h.get_all_by_role(Role::Slider).next().unwrap().rect();
    let (from, to) = (bend.center(), bend.right_center() - egui::vec2(4.0, 0.0));
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    // Previewed on the body, not yet in the document.
    assert!(h.state().session.doc.avatar_pose.is_rest());
    let previewed = h.state().session.viewport.avatar.rotations()[Joint::LeftElbow.index()];
    assert!(!previewed.abs_diff_eq(Quat::IDENTITY, 1e-3));
    h.drop_at(to);
    h.run();
    h.run();
    let s = &h.state().session;
    let bend = s.doc.avatar_pose.get(Joint::LeftElbow).bend;
    assert!(bend > 150.0, "dragged to near the end: {bend}");
    assert_eq!(s.history.done_labels().len(), undo_steps + 1, "one step");
    assert_eq!(s.history.undo_label().as_deref(), Some("Pose left elbow"));
}

#[test]
fn copy_and_mirror_buttons() {
    let mut h = harness();
    open_panel(&mut h);
    let s = &mut h.state_mut().session;
    s.viewport.posing.joint = Joint::LeftKnee;
    let bent = Pose::rest().with(Joint::LeftKnee, JointAngles::new(45.0, 0.0, 0.0));
    s.execute(SetPose::new("Bend", bent.clone()));
    h.run();
    h.get_by_label("Copy to other side").click();
    h.run();
    let pose = &h.state().session.doc.avatar_pose;
    assert_eq!(pose.get(Joint::RightKnee).bend, 45.0);
    h.state_mut()
        .session
        .execute(SetPose::new("Bend", bent.clone()));
    h.run();
    h.get_by_label("Mirror pose").click();
    h.run();
    assert_eq!(h.state().session.doc.avatar_pose, bent.mirrored());
    h.get_by_label("Reset joint").click();
    h.run();
    // The panel's joint is the left knee, which mirroring cleared.
    assert_eq!(h.state().session.doc.avatar_pose, bent.mirrored());
}

#[test]
fn the_pose_survives_resizing_and_tacks_follow_it() {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.execute(SetPose::new("Pose", named("T-pose")));
    h.run();
    let before = width(&h);
    let v = &mut h.state_mut().session.viewport;
    let mut m = v.avatar.measurements;
    m.height = 190.0;
    v.avatar.set_measurements(m);
    h.run();
    assert!(width(&h) > before + 0.05, "still in T-pose, and taller");

    // A point held as a triangle and weights is on the posed body.
    let v = &h.state().session.viewport;
    let mesh = v.avatar.mesh();
    let rig = v.avatar.rig().unwrap();
    let wrist = rig.joint_positions(v.avatar.rotations())[Joint::LeftWrist.index()];
    let near = mesh
        .triangles()
        .any(|[a, b, c]| ((a + b + c) / 3.0).distance(wrist) < 0.05);
    assert!(near, "the posed mesh has triangles at the raised wrist");
}

#[test]
fn while_draping_the_body_glides_to_a_new_pose() {
    let mut h = harness();
    h.state_mut().session.viewport.draping = true;
    h.run();
    let s = &mut h.state_mut().session;
    s.execute(SetPose::new("Pose", named("T-pose")));
    // The body keeps moving, so the UI keeps repainting: step frames.
    h.step();
    h.step();
    let v = &h.state().session.viewport;
    assert!(v.posing.is_moving(), "on its way");
    let rig = v.avatar.rig().unwrap();
    let end = mesh_width(&rig.skin(&pose_rotations(&named("T-pose"), rig.bones())));
    assert!(width(&h) < end - 0.01, "not there yet");
    for _ in 0..600 {
        h.step();
        if !h.state().session.viewport.posing.is_moving() {
            break;
        }
    }
    assert!(!h.state().session.viewport.posing.is_moving());
    assert!((width(&h) - end).abs() < 1e-4);
}

/// A screenshot of the Pose panel for a look by eye, written to
/// `$OD_SHOTS` (default: a temporary folder). Needs a wgpu adapter.
#[test]
#[ignore = "writes screenshots; run with --ignored"]
fn screenshots_of_the_pose_panel() {
    let dir = std::env::var_os("OD_SHOTS")
        .map_or_else(|| std::env::temp_dir().join("od_shots"), Into::into);
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 880.0))
        .wgpu()
        .build_eframe(|cc| OpenDesignApp::new(cc));
    // The scene itself can't be drawn here (see `drop_gpu`).
    h.state_mut().session.viewport.drop_gpu();
    h.run();
    open_panel(&mut h);
    h.get_by_label("Walking").click();
    h.run();
    h.run();
    let image = h.render().expect("a wgpu adapter to render with");
    image.save(dir.join("pose-panel.png")).unwrap();

    // The built-in walk, paused part way.
    h.get_by_label("Walk in place").click();
    h.step();
    h.get_by_label("Pause").click();
    h.run();
    let image = h.render().expect("a wgpu adapter to render with");
    image.save(dir.join("pose-panel-motion.png")).unwrap();
}

#[test]
fn the_built_in_walk_plays_pauses_and_stops() {
    let mut h = harness();
    open_panel(&mut h);
    h.get_by_label("Walk in place").click();
    h.step();
    let player = h.state().session.viewport.posing.motion.as_ref().unwrap();
    assert!(player.playing);
    assert_eq!(player.motion.matched.len(), Joint::COUNT);
    // The body moves from frame to frame.
    let rotations =
        |h: &Harness<'_, OpenDesignApp>| h.state().session.viewport.avatar.rotations().to_vec();
    let before = rotations(&h);
    for _ in 0..5 {
        h.step();
    }
    let after = rotations(&h);
    assert_ne!(before, after, "walking");

    h.get_by_label("Pause").click();
    h.run();
    let paused = rotations(&h);
    h.run();
    assert_eq!(rotations(&h), paused, "held still");
    assert!(
        h.state().session.doc.avatar_pose.is_rest(),
        "the pose is untouched"
    );

    h.get_by_label("Stop motion").click();
    h.run();
    h.run();
    assert!(h.state().session.viewport.posing.motion.is_none());
    let rest = rotations(&h);
    assert!(
        rest.iter().all(|q| q.abs_diff_eq(Quat::IDENTITY, 1e-6)),
        "back to the pose"
    );
}

#[test]
fn a_bad_motion_file_shows_why() {
    let mut h = harness();
    open_panel(&mut h);
    let v = &mut h.state_mut().session.viewport;
    let rig = v.avatar.rig().unwrap().clone();
    assert!(v
        .posing
        .load_motion("junk.bvh", "not a motion", &rig)
        .is_err());
    h.run();
    assert!(h.state().session.viewport.posing.motion.is_none());
    h.get_by_label("Couldn't load junk.bvh: not a BVH file (no HIERARCHY)");
}
