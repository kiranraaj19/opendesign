//! Orbit camera around a target point. World units are metres, Y up, and
//! the avatar faces +Z, so the front view looks down -Z.

use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};

use crate::mesh::Aabb;
use crate::pick::Ray;

/// Pitch stays just short of straight up/down so the view never flips.
pub const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;
pub const MIN_DISTANCE: f32 = 0.3;
pub const MAX_DISTANCE: f32 = 30.0;
const NEAR: f32 = 0.02;
const FAR: f32 = 200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    Front,
    Back,
    /// From the avatar's left side (+X).
    Left,
    /// From the avatar's right side (-X).
    Right,
    Top,
    Bottom,
}

impl ViewPreset {
    pub const ALL: [ViewPreset; 6] = [
        ViewPreset::Front,
        ViewPreset::Back,
        ViewPreset::Left,
        ViewPreset::Right,
        ViewPreset::Top,
        ViewPreset::Bottom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ViewPreset::Front => "Front",
            ViewPreset::Back => "Back",
            ViewPreset::Left => "Left",
            ViewPreset::Right => "Right",
            ViewPreset::Top => "Top",
            ViewPreset::Bottom => "Bottom",
        }
    }

    /// (yaw, pitch) in radians.
    fn angles(self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, PI};
        match self {
            ViewPreset::Front => (0.0, 0.0),
            ViewPreset::Back => (PI, 0.0),
            ViewPreset::Left => (FRAC_PI_2, 0.0),
            ViewPreset::Right => (-FRAC_PI_2, 0.0),
            ViewPreset::Top => (0.0, MAX_PITCH),
            ViewPreset::Bottom => (0.0, -MAX_PITCH),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Rotation about Y; 0 looks at the avatar's front.
    pub yaw: f32,
    /// Elevation above the horizon.
    pub pitch: f32,
    pub distance: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.95, 0.0),
            yaw: 0.6,
            pitch: 0.15,
            distance: 3.4,
            fov_y: 35f32.to_radians(),
        }
    }
}

impl OrbitCamera {
    /// Unit vector from the target towards the eye.
    pub fn back(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * sy, sp, cp * cy)
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }

    /// Screen-right and screen-up directions in world space.
    pub fn basis(&self) -> (Vec3, Vec3) {
        let forward = -self.back();
        let right = forward.cross(Vec3::Y).normalize();
        (right, right.cross(forward))
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    /// Depth maps to 0..1, as wgpu expects.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, aspect.max(1e-3), NEAR, FAR)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// Rotate by `delta_yaw`/`delta_pitch` radians, keeping pitch in range.
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        use std::f32::consts::{PI, TAU};
        self.yaw = (self.yaw + delta_yaw + PI).rem_euclid(TAU) - PI;
        self.pitch = (self.pitch + delta_pitch).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Slide the target so the scene follows a drag of `delta_px` pixels in
    /// a view `viewport_height_px` tall.
    pub fn pan(&mut self, delta_px: Vec2, viewport_height_px: f32) {
        let metres_per_px =
            2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_height_px.max(1.0);
        let (right, up) = self.basis();
        self.target += (up * delta_px.y - right * delta_px.x) * metres_per_px;
    }

    /// Move closer (`factor` < 1) or further away.
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn set_view(&mut self, preset: ViewPreset) {
        (self.yaw, self.pitch) = preset.angles();
    }

    /// The camera for frame `frame` of a `total`-frame turntable: one full
    /// revolution around `target`, starting from this camera's current
    /// yaw, keeping distance, pitch and fov. Frame `0` is this camera
    /// unchanged; frame `total` would (if it existed) land back on it too,
    /// so a sequence of `0..total` loops seamlessly. `total == 0` is
    /// treated as `1` (a single still, unrotated).
    pub fn turntable(&self, frame: u32, total: u32) -> Self {
        use std::f32::consts::{PI, TAU};
        let total = total.max(1);
        let t = (frame % total) as f32 / total as f32;
        let mut cam = *self;
        cam.yaw = (self.yaw + t * TAU + PI).rem_euclid(TAU) - PI;
        cam
    }

    /// Aim at `bounds` and back off until its bounding sphere fits the view.
    pub fn frame(&mut self, bounds: Aabb, aspect: f32) {
        self.target = bounds.center();
        let radius = (bounds.size().length() * 0.5).max(0.05);
        let half_y = self.fov_y * 0.5;
        let half_x = (half_y.tan() * aspect.max(1e-3)).atan();
        let half = half_y.min(half_x);
        // A small margin keeps the scene off the edges.
        self.distance = (radius / half.sin() * 1.05).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Normalised device coordinates (x right, y up, -1..1) of a point, or
    /// `None` when it is behind the camera.
    pub fn project(&self, point: Vec3, aspect: f32) -> Option<Vec2> {
        let clip = self.view_proj(aspect) * point.extend(1.0);
        (clip.w > NEAR * 0.5).then(|| clip.xy() / clip.w)
    }

    /// The ray from the eye through a point in normalised device coordinates.
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let inverse = self.view_proj(aspect).inverse();
        let near = inverse.project_point3(ndc.extend(0.0));
        let far = inverse.project_point3(ndc.extend(1.0));
        Ray::new(near, far - near)
    }
}

/// Pixel position inside a viewport to normalised device coordinates.
pub fn ndc_from_pixel(pixel: Vec2, viewport_min: Vec2, viewport_size: Vec2) -> Vec2 {
    let t = (pixel - viewport_min) / viewport_size.max(Vec2::ONE);
    Vec2::new(t.x * 2.0 - 1.0, 1.0 - t.y * 2.0)
}

/// The inverse of [`ndc_from_pixel`].
pub fn pixel_from_ndc(ndc: Vec2, viewport_min: Vec2, viewport_size: Vec2) -> Vec2 {
    viewport_min + Vec2::new(ndc.x + 1.0, 1.0 - ndc.y) * 0.5 * viewport_size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn target_projects_to_the_centre() {
        let cam = OrbitCamera::default();
        let ndc = cam.project(cam.target, 1.5).unwrap();
        assert!(ndc.length() < 1e-5, "{ndc}");
        assert!(cam.project(cam.eye() + cam.back(), 1.5).is_none(), "behind");
    }

    #[test]
    fn projection_keeps_screen_directions() {
        let mut cam = OrbitCamera::default();
        cam.set_view(ViewPreset::Front);
        let right = cam.project(cam.target + Vec3::X * 0.2, 1.0).unwrap();
        let up = cam.project(cam.target + Vec3::Y * 0.2, 1.0).unwrap();
        assert!(right.x > 0.0 && right.y.abs() < 1e-5);
        assert!(up.y > 0.0 && up.x.abs() < 1e-5);
        // Depth is 0..1 front to back.
        let vp = cam.view_proj(1.0);
        let near = vp.project_point3(cam.target + Vec3::Z);
        let far = vp.project_point3(cam.target - Vec3::Z);
        assert!(0.0 < near.z && near.z < far.z && far.z < 1.0);
    }

    #[test]
    fn presets_look_from_the_named_side() {
        let mut cam = OrbitCamera::default();
        let side = |cam: &OrbitCamera| cam.eye() - cam.target;
        cam.set_view(ViewPreset::Front);
        assert!(side(&cam).z > 0.0 && side(&cam).x.abs() < 1e-5);
        cam.set_view(ViewPreset::Back);
        assert!(side(&cam).z < 0.0);
        cam.set_view(ViewPreset::Left);
        assert!(side(&cam).x > 0.0);
        cam.set_view(ViewPreset::Right);
        assert!(side(&cam).x < 0.0);
        cam.set_view(ViewPreset::Top);
        assert!(side(&cam).y > cam.distance * 0.99);
        cam.set_view(ViewPreset::Bottom);
        assert!(side(&cam).y < -cam.distance * 0.99);
        // Even straight down the view matrix stays well-formed.
        assert!(cam.view().is_finite());
    }

    #[test]
    fn orbit_clamps_pitch_and_wraps_yaw() {
        let mut cam = OrbitCamera::default();
        cam.orbit(0.0, 10.0);
        assert_eq!(cam.pitch, MAX_PITCH);
        cam.orbit(0.0, -20.0);
        assert_eq!(cam.pitch, -MAX_PITCH);
        cam.yaw = 3.0;
        cam.orbit(1.0, 0.0);
        assert!((cam.yaw - (4.0 - std::f32::consts::TAU)).abs() < 1e-5);
        // Orbiting never changes what the camera looks at or how far away it is.
        let before = cam.eye().distance(cam.target);
        cam.orbit(0.7, 0.3);
        assert!((cam.eye().distance(cam.target) - before).abs() < 1e-4);
    }

    #[test]
    fn zoom_is_limited() {
        let mut cam = OrbitCamera::default();
        cam.zoom(0.5);
        assert!((cam.distance - 1.7).abs() < 1e-5);
        cam.zoom(1e-6);
        assert_eq!(cam.distance, MIN_DISTANCE);
        cam.zoom(1e6);
        assert_eq!(cam.distance, MAX_DISTANCE);
    }

    #[test]
    fn pan_moves_the_target_with_the_drag() {
        let mut cam = OrbitCamera::default();
        cam.set_view(ViewPreset::Front);
        let before = cam.target;
        // Dragging right moves the scene right, so the target moves left.
        cam.pan(Vec2::new(100.0, 0.0), 500.0);
        assert!(cam.target.x < before.x);
        assert!((cam.target.y - before.y).abs() < 1e-6);
        // A point under the cursor follows it: the old target now sits
        // 100 px right of centre.
        let ndc = cam.project(before, 1.0).unwrap();
        assert!((ndc.x * 250.0 - 100.0).abs() < 0.5, "{ndc}");
    }

    #[test]
    fn framing_fits_the_bounds() {
        let bounds = Aabb {
            min: Vec3::new(-0.5, 0.0, -0.3),
            max: Vec3::new(0.5, 1.8, 0.3),
        };
        for aspect in [0.5, 1.0, 2.0] {
            let mut cam = OrbitCamera::default();
            cam.frame(bounds, aspect);
            assert!(close(cam.target, bounds.center()));
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 {
                        bounds.min.x
                    } else {
                        bounds.max.x
                    },
                    if i & 2 == 0 {
                        bounds.min.y
                    } else {
                        bounds.max.y
                    },
                    if i & 4 == 0 {
                        bounds.min.z
                    } else {
                        bounds.max.z
                    },
                );
                let ndc = cam.project(corner, aspect).unwrap();
                assert!(ndc.abs().max_element() <= 1.0, "aspect {aspect}: {ndc}");
            }
        }
        // Not absurdly far either: the bounds fill a good part of the view.
        let mut cam = OrbitCamera::default();
        cam.set_view(ViewPreset::Front);
        cam.frame(bounds, 1.0);
        let top = cam.project(Vec3::new(0.0, 1.8, 0.0), 1.0).unwrap();
        assert!(top.y > 0.6, "{top}");
    }

    #[test]
    fn rays_go_through_the_pixel() {
        let cam = OrbitCamera::default();
        let centre = cam.ray(Vec2::ZERO, 1.3);
        assert!(close(
            centre.origin + centre.dir * centre.dir.dot(cam.target - centre.origin),
            cam.target
        ));
        let p = Vec3::new(0.3, 1.2, 0.1);
        let ray = cam.ray(cam.project(p, 1.3).unwrap(), 1.3);
        let t = ray.dir.dot(p - ray.origin);
        assert!(close(ray.at(t), p));
    }

    #[test]
    fn turntable_sweeps_a_full_revolution_and_keeps_everything_else() {
        let mut cam = OrbitCamera::default();
        cam.set_view(ViewPreset::Front);
        let start = cam.turntable(0, 12);
        assert_eq!(start.yaw, cam.yaw, "frame 0 is unrotated");
        assert_eq!(start.pitch, cam.pitch);
        assert_eq!(start.distance, cam.distance);
        assert_eq!(start.target, cam.target);
        assert_eq!(start.fov_y, cam.fov_y);

        let halfway = cam.turntable(6, 12);
        let diff = (halfway.yaw - cam.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        assert!((diff.abs() - std::f32::consts::PI).abs() < 1e-4, "{diff}");

        // Frame `total` would be a full turn, same angle as frame 0; the
        // modulo in `turntable` means asking for it directly (as if
        // looping past the end) gives the same camera too.
        assert_eq!(cam.turntable(12, 12).yaw, cam.turntable(0, 12).yaw);

        // A zero-frame "total" doesn't divide by zero.
        assert!(cam.turntable(0, 0).yaw.is_finite());
    }

    #[test]
    fn pixel_ndc_round_trip() {
        let (min, size) = (Vec2::new(100.0, 50.0), Vec2::new(400.0, 300.0));
        assert_eq!(ndc_from_pixel(min, min, size), Vec2::new(-1.0, 1.0));
        assert_eq!(ndc_from_pixel(min + size, min, size), Vec2::new(1.0, -1.0));
        let p = Vec2::new(321.0, 99.0);
        let back = pixel_from_ndc(ndc_from_pixel(p, min, size), min, size);
        assert!(back.distance(p) < 1e-3);
    }
}
