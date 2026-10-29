//! Saved camera views: the 3D camera kept under a name in the document
//! (so it is saved with the project), to jump back to later.

use od_core::workspace::{unused_name, CameraView, SetCameraViews};
use od_render::camera::{OrbitCamera, MAX_DISTANCE, MAX_PITCH, MIN_DISTANCE};

use crate::app::Session;

/// `camera` saved as `name`.
pub fn capture(camera: &OrbitCamera, name: impl Into<String>) -> CameraView {
    CameraView {
        name: name.into(),
        target: camera.target.to_array(),
        yaw: camera.yaw,
        pitch: camera.pitch,
        distance: camera.distance,
    }
}

/// Point `camera` as `view` says, kept within the camera's limits (a file
/// can hold anything). The field of view stays as it is.
pub fn restore(view: &CameraView, camera: &mut OrbitCamera) {
    let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
    if view.target.iter().all(|v| v.is_finite()) {
        camera.target = view.target.into();
    }
    camera.yaw = finite(view.yaw, camera.yaw);
    camera.pitch = finite(view.pitch, camera.pitch).clamp(-MAX_PITCH, MAX_PITCH);
    camera.distance = finite(view.distance, camera.distance).clamp(MIN_DISTANCE, MAX_DISTANCE);
}

/// Save the current camera as `name`, replacing a view of that name, as
/// one undo step.
pub fn save(s: &mut Session, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let view = capture(&s.viewport.camera, name);
    let mut views = s.doc.camera_views.clone();
    let label = match views.iter_mut().find(|v| v.name == name) {
        Some(old) => {
            *old = view;
            format!("Update view {name}")
        }
        None => {
            views.push(view);
            format!("Save view {name}")
        }
    };
    s.execute(SetCameraViews::new(label, views));
}

/// Forget the view at `index`, as one undo step.
pub fn delete(s: &mut Session, index: usize) {
    let mut views = s.doc.camera_views.clone();
    if index < views.len() {
        let gone = views.remove(index);
        s.execute(SetCameraViews::new(
            format!("Delete view {}", gone.name),
            views,
        ));
    }
}

/// Move the 3D camera to the view at `index`. Not a document change.
pub fn show(s: &mut Session, index: usize) {
    if let Some(view) = s.doc.camera_views.get(index) {
        restore(view, &mut s.viewport.camera);
    }
}

/// The "Saved views" menu above the 3D view: jump to a view, delete it,
/// or save the current camera under a name.
pub fn menu(ui: &mut egui::Ui, s: &mut Session) {
    let mut jump = None;
    let mut forget = None;
    if s.doc.camera_views.is_empty() {
        ui.weak("No saved views yet.");
    }
    for (i, view) in s.doc.camera_views.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .button(&view.name)
                .on_hover_text("Move the camera here")
                .clicked()
            {
                jump = Some(i);
            }
            if ui
                .small_button("Delete")
                .on_hover_text(format!("Forget the view {}", view.name))
                .clicked()
            {
                forget = Some(i);
            }
        });
    }
    if let Some(i) = jump {
        show(s, i);
        ui.close();
    }
    if let Some(i) = forget {
        delete(s, i);
    }
    ui.separator();
    // The name being typed lives in egui's memory while the menu is open.
    let id = ui.id().with("new_view_name");
    let mut name: String = ui
        .data_mut(|d| d.get_temp(id))
        .unwrap_or_else(|| unused_name("View", s.doc.camera_views.iter().map(|v| v.name.as_str())));
    let mut saved = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(120.0)
                .hint_text("Name"),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui
            .button("Save view")
            .on_hover_text("Save the current camera under this name, with the project")
            .clicked()
            || entered
        {
            save(s, &name);
            saved = true;
        }
    });
    if saved {
        ui.data_mut(|d| d.remove::<String>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_restores_the_camera_it_was_taken_from() {
        let mut camera = OrbitCamera::default();
        camera.orbit(1.0, 0.3);
        camera.zoom(0.5);
        camera.target.x += 0.25;
        let view = capture(&camera, "Side");
        let mut other = OrbitCamera::default();
        restore(&view, &mut other);
        assert_eq!(other, camera);
    }

    #[test]
    fn a_damaged_view_stays_within_the_camera_limits() {
        let view = CameraView {
            name: "Odd".into(),
            target: [f32::NAN, 0.0, 0.0],
            yaw: f32::INFINITY,
            pitch: 9.0,
            distance: 1e6,
        };
        let mut camera = OrbitCamera::default();
        let before = camera;
        restore(&view, &mut camera);
        assert_eq!(camera.target, before.target);
        assert_eq!(camera.yaw, before.yaw);
        assert_eq!(camera.pitch, MAX_PITCH);
        assert_eq!(camera.distance, MAX_DISTANCE);
    }
}
