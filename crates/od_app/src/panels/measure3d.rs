//! Measuring the avatar in the 3D view: the girth around the body where it
//! is clicked, and a tape laid over the surface through clicked points.
//! The maths is in [`od_render::measure`]; this keeps what was clicked
//! (session state, like the avatar itself) and draws the results.
//!
//! Clicked points are kept as a triangle of the avatar mesh and weights in
//! it, as avatar tacks are, so they follow the body when it is resized.

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};
use od_render::camera::ndc_from_pixel;
use od_render::glam::{self, Vec3};
use od_render::measure::{girth, normal_at, path_length, surface_path, Girth};
use od_render::mesh::Mesh;

use crate::app::Session;
use crate::panels::viewport3d::Avatar;
use crate::theme;

/// What a click on the avatar measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Clicks select, as usual.
    #[default]
    Off,
    /// A click measures the girth around the body at that height.
    Girth,
    /// Each click adds a point to the tape.
    Tape,
}

/// A point on the avatar's surface: a triangle of its mesh and
/// barycentric weights in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    pub triangle: usize,
    pub weights: [f32; 3],
}

impl SurfacePoint {
    /// Where it is on `mesh`, in metres.
    pub fn world(&self, mesh: &Mesh) -> Option<Vec3> {
        let corners = mesh.indices.get(self.triangle * 3..self.triangle * 3 + 3)?;
        corners
            .iter()
            .zip(self.weights)
            .map(|(&i, w)| Some(*mesh.positions.get(i as usize)? * w))
            .sum()
    }

    fn normal(&self, mesh: &Mesh) -> Vec3 {
        normal_at(mesh, self.triangle, self.weights).unwrap_or(Vec3::Z)
    }
}

/// The 3D measuring tools' state.
#[derive(Debug, Clone, Default)]
pub struct Measure3D {
    pub mode: Mode,
    /// Where each girth was clicked.
    pub girths: Vec<SurfacePoint>,
    /// The points the tape goes through, in order.
    pub tape: Vec<SurfacePoint>,
    /// What was worked out from them, and from which avatar.
    results: Option<Results>,
}

#[derive(Debug, Clone)]
struct Results {
    /// The avatar's generation and how many points of each kind there
    /// were, to tell when these are stale.
    key: (u64, usize, usize),
    girths: Vec<Option<Girth>>,
    /// The tape's whole path over the surface, with each clicked point.
    tape: Vec<Vec3>,
    tape_points: Vec<Vec3>,
}

impl Measure3D {
    /// Work the girths and tape out again if they were clicked or the
    /// avatar changed since last time.
    pub fn refresh(&mut self, avatar: &Avatar) {
        let key = (avatar.generation, self.girths.len(), self.tape.len());
        if self.results.as_ref().is_some_and(|r| r.key == key) {
            return;
        }
        let mesh = avatar.mesh();
        let girths = self
            .girths
            .iter()
            .map(|p| {
                let at = p.world(mesh)?;
                girth(mesh, at.y, at)
            })
            .collect();
        let points: Vec<(Vec3, Vec3)> = self
            .tape
            .iter()
            .filter_map(|p| Some((p.world(mesh)?, p.normal(mesh))))
            .collect();
        let mut tape: Vec<Vec3> = points.first().map(|p| p.0).into_iter().collect();
        for w in points.windows(2) {
            let ((a, na), (b, nb)) = (w[0], w[1]);
            let up = (na + nb).try_normalize().unwrap_or(na);
            tape.extend(surface_path(mesh, a, b, up).into_iter().skip(1));
        }
        self.results = Some(Results {
            key,
            girths,
            tape,
            tape_points: points.into_iter().map(|p| p.0).collect(),
        });
    }

    /// Each girth's tape length in metres (`None` where the click found
    /// no loop round the body), as last worked out.
    pub fn girth_lengths(&self) -> Vec<Option<f32>> {
        self.results.as_ref().map_or_else(Vec::new, |r| {
            r.girths
                .iter()
                .map(|g| g.as_ref().map(|g| g.tape))
                .collect()
        })
    }

    /// The tape's length over the surface in metres, as last worked out.
    pub fn tape_length(&self) -> f32 {
        self.results.as_ref().map_or(0.0, |r| path_length(&r.tape))
    }

    pub fn clear(&mut self) {
        self.girths.clear();
        self.tape.clear();
    }
}

fn cm(metres: f32) -> String {
    format!("{:.1} cm", metres * 100.0)
}

/// The Measure menu above the 3D view: which tool, the results so far, and
/// a way to clear them.
pub fn buttons(ui: &mut egui::Ui, s: &mut Session) {
    let v = &mut s.viewport;
    v.measure.refresh(&v.avatar);
    let m = &mut v.measure;
    ui.menu_button("Measure body", |ui| {
        ui.radio_value(&mut m.mode, Mode::Off, "Off");
        ui.radio_value(&mut m.mode, Mode::Girth, "Circumference")
            .on_hover_text("Click the avatar to measure around the body at that height");
        ui.radio_value(&mut m.mode, Mode::Tape, "Tape")
            .on_hover_text("Click points on the avatar to lay a tape over its surface");
        let girths = m.girth_lengths();
        if !girths.is_empty() || !m.tape.is_empty() {
            ui.separator();
        }
        for (i, length) in girths.iter().enumerate() {
            ui.label(format!(
                "Circumference {}: {}",
                i + 1,
                length.map_or_else(|| "–".to_owned(), cm)
            ));
        }
        if m.tape.len() >= 2 {
            ui.label(format!("Tape: {}", cm(m.tape_length())));
        }
        ui.separator();
        if ui
            .add_enabled(
                !m.girths.is_empty() || !m.tape.is_empty(),
                egui::Button::new("Clear measurements"),
            )
            .clicked()
        {
            m.clear();
        }
    })
    .response
    .on_hover_text("Measure the avatar: circumferences and a tape over the body");
}

/// A click in the 3D view at `pos`: with a measuring tool on, measure the
/// avatar where it was clicked. Returns whether the click was taken.
pub fn click(s: &mut Session, rect: Rect, pos: Pos2) -> bool {
    let v = &mut s.viewport;
    if v.measure.mode == Mode::Off {
        return false;
    }
    let ndc = ndc_from_pixel(
        glam::Vec2::new(pos.x, pos.y),
        glam::Vec2::new(rect.min.x, rect.min.y),
        glam::Vec2::new(rect.width(), rect.height()),
    );
    let ray = v.camera.ray(ndc, rect.width() / rect.height().max(1.0));
    let mesh = v.avatar.mesh();
    if let Some((t, triangle)) = od_render::pick::ray_mesh_triangle(&ray, mesh) {
        let corners = mesh.triangles().nth(triangle);
        if let Some(corners) = corners {
            let point = SurfacePoint {
                triangle,
                weights: od_render::pick::triangle_weights(corners, ray.at(t)),
            };
            match v.measure.mode {
                Mode::Girth => v.measure.girths.push(point),
                Mode::Tape => v.measure.tape.push(point),
                Mode::Off => {}
            }
        }
    }
    true
}

/// What the 3D view's hint line says while measuring.
pub fn hint(s: &Session) -> Option<&'static str> {
    match s.viewport.measure.mode {
        Mode::Off => None,
        Mode::Girth => {
            Some("Circumference: click the avatar where to measure around it · Drag: orbit")
        }
        Mode::Tape => {
            Some("Tape: click points on the avatar to lay the tape through · Drag: orbit")
        }
    }
}

/// A label with a backing, so it reads over the avatar.
fn label(painter: &egui::Painter, at: Pos2, text: String, color: Color32, pal: &theme::Palette) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), color);
    let rect = Align2::LEFT_CENTER
        .anchor_size(at, galley.size())
        .expand(2.0);
    painter.rect_filled(rect, 2.0, pal.canvas_bg.gamma_multiply(0.85));
    painter.galley(rect.min + egui::vec2(2.0, 2.0), galley, color);
}

/// The girths and the tape over the 3D view, with their lengths.
pub fn draw(painter: &egui::Painter, s: &Session, rect: Rect, pal: &theme::Palette) {
    let v = &s.viewport;
    let Some(results) = &v.measure.results else {
        return;
    };
    let screen = |p: Vec3| v.screen_pos(rect, p);
    let stroke = Stroke::new(2.0, pal.selected);
    for (i, g) in results.girths.iter().enumerate() {
        let Some(g) = g else { continue };
        let mut pts: Vec<Pos2> = g.section.points.iter().filter_map(|&p| screen(p)).collect();
        if let Some(&first) = pts.first() {
            pts.push(first);
            painter.add(Shape::line(pts, stroke));
        }
        // By the loop's right-hand side as seen from the camera.
        let (right, _) = v.camera.basis();
        let side = g
            .section
            .points
            .iter()
            .copied()
            .max_by(|a, b| a.dot(right).total_cmp(&b.dot(right)));
        if let Some(at) = side.and_then(screen) {
            let text = format!("Circumference {}: {}", i + 1, cm(g.tape));
            label(painter, at + egui::vec2(10.0, 0.0), text, pal.selected, pal);
        }
    }
    let tape: Vec<Pos2> = results.tape.iter().filter_map(|&p| screen(p)).collect();
    if tape.len() >= 2 {
        painter.add(Shape::line(tape.clone(), Stroke::new(2.0, pal.preview)));
    }
    for p in results.tape_points.iter().filter_map(|&p| screen(p)) {
        painter.circle_filled(p, 4.0, pal.preview);
    }
    if let (Some(&last), true) = (tape.last(), results.tape_points.len() >= 2) {
        let text = format!("Tape: {}", cm(path_length(&results.tape)));
        label(
            painter,
            last + egui::vec2(10.0, 0.0),
            text,
            pal.preview,
            pal,
        );
    }
}
