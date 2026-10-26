//! 3D garment window, placeholder edition.
//!
//! Draws a wireframe mannequin and the patterns arranged around it with
//! egui's 2D painter and a hand-rolled perspective projection. The real GPU
//! renderer (wgpu) replaces this in Phase 4. World units are metres, Y up.

use egui::{Align2, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2 as Px};

use crate::app::Session;
use crate::theme;

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn normalized(self) -> Vec3 {
        let len = self.dot(self).sqrt();
        Vec3::new(self.x / len, self.y / len, self.z / len)
    }
}

pub struct Viewport3D {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for Viewport3D {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.25,
            distance: 3.2,
        }
    }
}

/// Orbit camera looking at the mannequin's chest.
struct Camera {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    focal: f32,
    center: Pos2,
}

impl Camera {
    fn new(v: &Viewport3D, rect: Rect) -> Self {
        let target = Vec3::new(0.0, 1.05, 0.0);
        let eye = Vec3::new(
            target.x + v.distance * v.pitch.cos() * v.yaw.sin(),
            target.y + v.distance * v.pitch.sin(),
            target.z + v.distance * v.pitch.cos() * v.yaw.cos(),
        );
        let forward = target.sub(eye).normalized();
        let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalized();
        let up = right.cross(forward);
        Self {
            eye,
            right,
            up,
            forward,
            focal: rect.height() * 1.4,
            center: rect.center(),
        }
    }

    fn project(&self, p: Vec3) -> Option<Pos2> {
        let d = p.sub(self.eye);
        let z = d.dot(self.forward);
        if z < 0.05 {
            return None;
        }
        Some(self.center + Px::new(d.dot(self.right), -d.dot(self.up)) * (self.focal / z))
    }

    fn polyline(&self, points: impl IntoIterator<Item = Vec3>) -> Vec<Pos2> {
        points.into_iter().filter_map(|p| self.project(p)).collect()
    }
}

/// Mannequin cross-sections: (height, half-width, half-depth) in metres.
const BODY_RINGS: [(f32, f32, f32); 9] = [
    (0.80, 0.17, 0.12),
    (0.92, 0.18, 0.12),
    (1.02, 0.14, 0.10),
    (1.15, 0.16, 0.11),
    (1.25, 0.17, 0.11),
    (1.38, 0.19, 0.10),
    (1.44, 0.10, 0.07),
    (1.50, 0.05, 0.05),
    (1.55, 0.05, 0.05),
];

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "3D garment view")
    });
    let rect = response.rect;
    let pal = theme::palette(ui.visuals().dark_mode);
    let v = &mut s.viewport;

    if response.dragged_by(PointerButton::Primary) || response.dragged_by(PointerButton::Secondary)
    {
        let d = response.drag_delta();
        v.yaw -= d.x * 0.01;
        v.pitch = (v.pitch + d.y * 0.01).clamp(-1.2, 1.4);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        v.distance = (v.distance * (-scroll * 0.002).exp()).clamp(0.8, 12.0);
    }

    let cam = Camera::new(v, rect);
    painter.rect_filled(rect, 0.0, pal.canvas_bg);

    // Floor grid, 2 m square.
    for i in -10..=10 {
        let t = i as f32 * 0.1;
        let color = if i == 0 { pal.axis } else { pal.grid_minor };
        for line in [
            [Vec3::new(t, 0.0, -1.0), Vec3::new(t, 0.0, 1.0)],
            [Vec3::new(-1.0, 0.0, t), Vec3::new(1.0, 0.0, t)],
        ] {
            painter.add(Shape::line(cam.polyline(line), Stroke::new(1.0, color)));
        }
    }

    draw_mannequin(&painter, &cam, pal.avatar);

    // Arrange patterns on a ring around the body, like CLO's arrangement points.
    let count = s.doc.patterns.len().max(1) as f32;
    for (i, pattern) in s.doc.patterns.iter().enumerate() {
        let Some((min, max)) = pattern.bounds() else {
            continue;
        };
        let mid = (min + max) * 0.5;
        let angle = i as f32 / count * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let radius = 0.4;
        let points = pattern.outline.iter().map(|p| {
            // mm to m; pattern x runs along the ring tangent, y downward.
            let u = (p.x - mid.x) / 1000.0;
            let h = 1.15 - (p.y - mid.y) / 1000.0;
            Vec3::new(radius * sin + u * cos, h, radius * cos - u * sin)
        });
        let screen = cam.polyline(points);
        if screen.len() < 3 {
            continue;
        }
        let stroke = if s.selection == Some(pattern.id) {
            Stroke::new(2.5, pal.selected)
        } else {
            Stroke::new(1.5, pal.pattern_stroke)
        };
        painter.add(Shape::convex_polygon(
            screen.clone(),
            pal.pattern_fill,
            Stroke::NONE,
        ));
        painter.add(Shape::closed_line(screen, stroke));
    }

    painter.text(
        rect.left_top() + Px::new(10.0, 8.0),
        Align2::LEFT_TOP,
        "Placeholder view · drag to orbit · scroll to zoom · GPU renderer arrives in Phase 4",
        FontId::proportional(12.0),
        pal.hint,
    );
}

fn draw_mannequin(painter: &egui::Painter, cam: &Camera, color: egui::Color32) {
    let stroke = Stroke::new(1.2, color);
    let ring = |h: f32, rx: f32, rz: f32| {
        (0..=24).map(move |k| {
            let a = k as f32 / 24.0 * std::f32::consts::TAU;
            Vec3::new(rx * a.cos(), h, rz * a.sin())
        })
    };
    for (h, rx, rz) in BODY_RINGS {
        painter.add(Shape::line(cam.polyline(ring(h, rx, rz)), stroke));
    }
    // Vertical silhouette lines tie the rings together.
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        let line = BODY_RINGS
            .iter()
            .map(|&(h, rx, rz)| Vec3::new(rx * a.cos(), h, rz * a.sin()));
        painter.add(Shape::line(cam.polyline(line), stroke));
    }
    // Head.
    for h in [1.60, 1.68, 1.76] {
        let r = 0.09 * (1.0 - ((h - 1.68) / 0.11_f32).powi(2)).sqrt();
        painter.add(Shape::line(cam.polyline(ring(h, r, r)), stroke));
    }
    // Stand.
    painter.add(Shape::line(
        cam.polyline([Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.8, 0.0)]),
        stroke,
    ));
}
