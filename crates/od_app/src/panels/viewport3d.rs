//! 3D garment window: the avatar and the patterns arranged around it.
//!
//! Rendering, geometry and picking live in `od_render`; this tab turns
//! input into camera moves and selection, hosts the wgpu paint callback,
//! and holds the avatar choice (session state, not saved in the document).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use eframe::egui_wgpu;
use egui::{Color32, PointerButton, Pos2, Rect, Sense, UiBuilder};
use od_core::{Pattern, PatternId, TrimKind, Vec2};
use od_render::avatar::{avatar_mesh, AvatarKind, Measurements};
use od_render::camera::{ndc_from_pixel, OrbitCamera, ViewPreset};
use od_render::glam;
use od_render::gpu::{Renderer, Targets};
use od_render::mesh::{Aabb, Mesh};
use od_render::pattern::RingLayout;
use od_render::scene::{linear, AvatarDraw, Frame, GridColors};
use od_render::view::{self, FrameInput, PlacedPattern, Theme};
use od_sim::collision::Collider;

use crate::app::Session;
use crate::theme;

/// Shown instead of the scene when there is no wgpu renderer.
pub const NO_GPU: &str = "3D preview unavailable: no GPU renderer";

/// A pattern's outline as a polyline in millimetres, with curves flattened.
/// The 3D view reads patterns only through this.
fn pattern_polyline(p: &Pattern) -> Vec<Vec2> {
    p.polyline(od_core::document::TOLERANCE)
}

#[derive(Debug, Clone)]
pub enum AvatarSource {
    Preset(AvatarKind),
    /// Loaded from a file; only its height can change (uniform scale).
    File {
        name: String,
        mesh: Arc<Mesh>,
    },
}

pub struct Avatar {
    pub source: AvatarSource,
    /// Centimetres. For file avatars only `height` is used.
    pub measurements: Measurements,
    mesh: Arc<Mesh>,
    bounds: Aabb,
    /// Bumped on every change so the renderer re-uploads the mesh.
    generation: u64,
}

impl Avatar {
    fn preset(kind: AvatarKind) -> Self {
        let mut avatar = Self {
            source: AvatarSource::Preset(kind),
            measurements: kind.default_measurements(),
            mesh: Arc::default(),
            bounds: Aabb {
                min: glam::Vec3::ZERO,
                max: glam::Vec3::ZERO,
            },
            generation: 0,
        };
        avatar.rebuild();
        avatar
    }

    pub fn kind(&self) -> Option<AvatarKind> {
        match self.source {
            AvatarSource::Preset(kind) => Some(kind),
            AvatarSource::File { .. } => None,
        }
    }

    pub fn name(&self) -> &str {
        match &self.source {
            AvatarSource::Preset(kind) => kind.label(),
            AvatarSource::File { name, .. } => name,
        }
    }

    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    /// Switch to a preset body at its default size.
    pub fn set_preset(&mut self, kind: AvatarKind) {
        self.source = AvatarSource::Preset(kind);
        self.measurements = kind.default_measurements();
        self.rebuild();
    }

    pub fn set_measurements(&mut self, m: Measurements) {
        self.measurements = m.clamped();
        self.rebuild();
    }

    /// Replace the avatar with an OBJ or glTF model.
    pub fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let mesh = od_render::import::load_avatar(path)?;
        let height = mesh.bounds().map_or(0.0, |b| b.size().y) * 100.0;
        let name = path
            .file_name()
            .map_or("Custom avatar".into(), |n| n.to_string_lossy().into_owned());
        self.source = AvatarSource::File {
            name,
            mesh: Arc::new(mesh),
        };
        self.measurements.height = height;
        self.rebuild();
        Ok(())
    }

    fn rebuild(&mut self) {
        let mesh = match &self.source {
            AvatarSource::Preset(kind) => avatar_mesh(*kind, self.measurements),
            AvatarSource::File { mesh, .. } => {
                let mut mesh = Mesh::clone(mesh);
                let natural = mesh.bounds().map_or(1.0, |b| b.size().y);
                let scale = self.measurements.height / 100.0 / natural.max(1e-6);
                mesh.scale_translate(scale, glam::Vec3::ZERO);
                mesh
            }
        };
        self.bounds = mesh.bounds().unwrap_or(self.bounds);
        self.mesh = Arc::new(mesh);
        self.generation += 1;
    }
}

pub struct Viewport3D {
    pub camera: OrbitCamera,
    pub avatar: Avatar,
    /// The render pass we draw into, when eframe runs on wgpu.
    gpu: Option<Targets>,
    pub show_sizing: bool,
    error: Option<String>,
    /// Fit the scene to the view once its size is known.
    needs_framing: bool,
    /// Built fresh each time draping is turned on, from the document's
    /// patterns and sewing data as they stood then; kept (but not
    /// stepped) while paused so the last pose stays on screen.
    pub drape: Option<od_sim::drape::Drape>,
    pub draping: bool,
    /// While draping, clicking the garment pins or unpins the nearest
    /// point instead of selecting a pattern.
    pub pinning: bool,
    /// Color the draped cloth by how stretched it is, instead of the
    /// theme fill.
    pub show_strain: bool,
}

impl Viewport3D {
    /// Set up the GPU renderer if eframe has a wgpu render state; without
    /// one (headless tests, or the glow backend) the tab shows a message.
    pub fn new(render_state: Option<&egui_wgpu::RenderState>) -> Self {
        let gpu = render_state.map(|rs| {
            // Must match the depth buffer and MSAA set in main.rs.
            let targets = Targets {
                color: rs.target_format,
                depth: Some(od_render::DEPTH_FORMAT),
                samples: u32::from(od_render::MSAA_SAMPLES),
            };
            let renderer = Renderer::new(&rs.device, targets);
            rs.renderer.write().callback_resources.insert(renderer);
            targets
        });
        Self {
            camera: OrbitCamera::default(),
            avatar: Avatar::preset(AvatarKind::Female),
            gpu,
            show_sizing: false,
            error: None,
            needs_framing: true,
            drape: None,
            draping: false,
            pinning: false,
            show_strain: false,
        }
    }

    /// Ask for an avatar file and load it.
    pub fn load_avatar_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Avatar (OBJ, glTF)", &od_render::import::EXTENSIONS)
            .pick_file();
        if let Some(path) = picked {
            self.load_avatar(&path);
        }
    }

    pub fn load_avatar(&mut self, path: &Path) {
        self.error = self.avatar.load_file(path).err();
        if self.error.is_some() {
            self.show_sizing = true;
        }
    }

    /// Where a world point appears in the view `rect`, if in front of the camera.
    pub fn screen_pos(&self, rect: Rect, world: glam::Vec3) -> Option<Pos2> {
        let ndc = self.camera.project(world, aspect(rect))?;
        let p = od_render::camera::pixel_from_ndc(
            ndc,
            to_glam(rect.min),
            to_glam(rect.size().to_pos2()),
        );
        Some(Pos2::new(p.x, p.y))
    }

    /// The pattern under `pos` in the view `rect`; the avatar blocks clicks.
    pub fn pick(
        &self,
        rect: Rect,
        pos: Pos2,
        placed: &[PlacedPattern<PatternId>],
    ) -> Option<PatternId> {
        let ndc = ndc_from_pixel(
            to_glam(pos),
            to_glam(rect.min),
            to_glam(rect.size().to_pos2()),
        );
        let ray = self.camera.ray(ndc, aspect(rect));
        view::pick(&ray, Some(self.avatar.mesh()), placed)
    }

    /// Like [`Self::pick`], but also returns where on the pattern the ray
    /// landed, for a tool that pins the cloth where it was clicked.
    pub fn pick_point(
        &self,
        rect: Rect,
        pos: Pos2,
        placed: &[PlacedPattern<PatternId>],
    ) -> Option<(PatternId, glam::Vec3)> {
        let ndc = ndc_from_pixel(
            to_glam(pos),
            to_glam(rect.min),
            to_glam(rect.size().to_pos2()),
        );
        let ray = self.camera.ray(ndc, aspect(rect));
        let items = placed.iter().map(|p| (p.key, &p.mesh));
        let (key, t) = od_render::pick::closest_hit(&ray, items)?;
        Some((key, ray.at(t)))
    }

    /// Everything in view, from the current direction.
    fn frame_all(&mut self, rect: Rect, placed: &[PlacedPattern<PatternId>]) {
        self.camera
            .frame(view::scene_bounds(self.avatar.bounds, placed), aspect(rect));
    }
}

fn to_glam(p: Pos2) -> glam::Vec2 {
    glam::Vec2::new(p.x, p.y)
}

fn aspect(rect: Rect) -> f32 {
    rect.width() / rect.height().max(1.0)
}

/// The document's patterns, laid out around the avatar.
pub fn placed_patterns(s: &Session) -> Vec<PlacedPattern<PatternId>> {
    let outlines = s.doc.patterns.iter().map(|p| {
        let outline = pattern_polyline(p)
            .into_iter()
            .map(|v| glam::Vec2::new(v.x, v.y))
            .collect();
        (p.id, outline)
    });
    view::place_patterns(s.viewport.avatar.bounds, outlines)
}

/// Where each pattern starts out on the ring, for [`od_sim::drape::Drape`]
/// to build a cloth scene from. Mirrors [`view::place_patterns`]'s layout
/// so draping starts from the same pose the flat preview shows.
fn ring_placements(s: &Session) -> Vec<(PatternId, od_render::pattern::Placement)> {
    let ring = RingLayout::around(s.viewport.avatar.bounds);
    let count = s.doc.patterns.len();
    s.doc
        .patterns
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id, ring.placement(i, count)))
        .collect()
}

/// Start simulating from the document's current patterns and sewing data,
/// or pause in place if already draping.
fn toggle_drape(s: &mut Session) {
    if s.viewport.draping {
        s.viewport.draping = false;
        return;
    }
    let placements = ring_placements(s);
    let mut drape =
        od_sim::drape::Drape::new(&s.doc, &placements, &od_sim::cloth::ClothConfig::default());
    drape
        .colliders
        .push(Collider::mesh(s.viewport.avatar.mesh().clone(), 0.005));
    drape
        .colliders
        .push(Collider::ground(s.viewport.avatar.bounds.min.y));
    s.viewport.drape = Some(drape);
    s.viewport.draping = true;
}

/// Advance the simulation by this frame's time, if draping is running.
fn step_drape(ui: &egui::Ui, s: &mut Session) {
    if !s.viewport.draping {
        return;
    }
    if let Some(drape) = &mut s.viewport.drape {
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 1.0 / 30.0);
        drape.step(dt, 8);
        ui.ctx().request_repaint();
    }
}

/// The patterns to show: draped if a simulation is running or paused,
/// otherwise the flat ring layout.
fn current_placed_patterns(s: &Session) -> Vec<PlacedPattern<PatternId>> {
    match &s.viewport.drape {
        Some(drape) => drape.placed_patterns(),
        None => placed_patterns(s),
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let action = ui.horizontal(|ui| view_buttons(ui, s)).inner;
    step_drape(ui, s);
    let placed = current_placed_patterns(s);
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "3D garment view")
    });
    if std::mem::take(&mut s.viewport.needs_framing) {
        s.viewport.frame_all(rect, &placed);
    }
    match action {
        Some(ViewAction::Preset(preset)) => {
            s.viewport.camera.set_view(preset);
            s.viewport.frame_all(rect, &placed);
        }
        Some(ViewAction::Frame) => s.viewport.frame_all(rect, &placed),
        None => {}
    }

    handle_input(ui, &response, rect, s, &placed);
    paint(ui, rect, s, &placed);
    if s.viewport.show_sizing {
        sizing_panel(ui, rect, s);
    }
}

enum ViewAction {
    Preset(ViewPreset),
    Frame,
}

/// Camera presets and toggles above the scene. Camera moves are returned,
/// since framing needs the scene's rect, laid out after the buttons.
fn view_buttons(ui: &mut egui::Ui, s: &mut Session) -> Option<ViewAction> {
    let mut action = None;
    for preset in ViewPreset::ALL {
        if ui
            .button(preset.label())
            .on_hover_text(format!("{} view", preset.label()))
            .clicked()
        {
            action = Some(ViewAction::Preset(preset));
        }
    }
    ui.separator();
    if ui
        .button("Frame")
        .on_hover_text("Fit the avatar and patterns in view (or double-click the view)")
        .clicked()
    {
        action = Some(ViewAction::Frame);
    }
    ui.separator();
    ui.toggle_value(&mut s.viewport.show_sizing, "Avatar size")
        .on_hover_text("Height, chest, waist and hip of the avatar");
    ui.separator();
    let label = if s.viewport.draping {
        "Stop draping"
    } else {
        "Drape"
    };
    if ui
        .button(label)
        .on_hover_text("Simulate the sewn patterns draping on the avatar (XPBD)")
        .clicked()
    {
        toggle_drape(s);
    }
    if s.viewport.drape.is_some() {
        if ui
            .button("Reset drape")
            .on_hover_text("Discard the simulation and go back to the flat layout")
            .clicked()
        {
            s.viewport.drape = None;
            s.viewport.draping = false;
        }
        ui.toggle_value(&mut s.viewport.pinning, "Pin")
            .on_hover_text("Click the draped cloth to pin or unpin a point in place");
        ui.toggle_value(&mut s.viewport.show_strain, "Strain")
            .on_hover_text(
                "Color the cloth by how stretched it is: green relaxed, red overstretched",
            );
    } else {
        s.viewport.pinning = false;
        s.viewport.show_strain = false;
    }
    action
}

/// Green-to-red stretch heatmap: relaxed fabric at `0.0`, fully red by
/// 15% over its rest length.
fn strain_color(strain: f32) -> [f32; 4] {
    let t = (strain / 0.15).clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (f32::from(a) + t * (f32::from(b) - f32::from(a))) as u8;
    color(Color32::from_rgb(
        lerp(40, 210),
        lerp(160, 40),
        lerp(40, 40),
    ))
}

fn handle_input(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    s: &mut Session,
    placed: &[PlacedPattern<PatternId>],
) {
    let v = &mut s.viewport;
    let delta = response.drag_delta();
    if response.dragged_by(PointerButton::Primary) {
        v.camera.orbit(-delta.x * 0.01, delta.y * 0.01);
    } else if response.dragged_by(PointerButton::Secondary)
        || response.dragged_by(PointerButton::Middle)
    {
        v.camera.pan(to_glam(delta.to_pos2()), rect.height());
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            v.camera.zoom((-scroll * 0.002).exp());
        }
    }
    if response.double_clicked() {
        // Back to the starting angle, everything in view.
        let fov = v.camera.fov_y;
        v.camera = OrbitCamera {
            fov_y: fov,
            ..OrbitCamera::default()
        };
        v.frame_all(rect, placed);
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if v.pinning {
                if let Some((pattern, point)) = v.pick_point(rect, pos, placed) {
                    if let Some(drape) = v.drape.as_mut() {
                        drape.toggle_pin_near(pattern, point);
                    }
                }
            } else {
                s.selection = v.pick(rect, pos, placed);
            }
        }
    }
}

fn paint(ui: &mut egui::Ui, rect: Rect, s: &Session, placed: &[PlacedPattern<PatternId>]) {
    let dark = ui.visuals().dark_mode;
    let pal = theme::palette(dark);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, pal.canvas_bg);

    let v = &s.viewport;
    if let Some(targets) = v.gpu {
        let theme = Theme {
            avatar: color(pal.avatar),
            pattern: color(pal.pattern_fill),
            selected: color(pal.selected),
            outline: color(pal.pattern_stroke),
            grid: GridColors {
                minor: color(pal.grid_minor),
                major: color(pal.grid_major),
                axis: color(pal.axis),
            },
            shadow: [0.0, 0.0, 0.0, if dark { 0.35 } else { 0.18 }],
        };
        let strain_colors = if v.show_strain {
            v.drape.as_ref()
        } else {
            None
        }
        .map(|drape| {
            let by_pattern: HashMap<_, _> = drape.strain_colors(strain_color).into_iter().collect();
            placed
                .iter()
                .map(|p| by_pattern.get(&p.key).cloned().unwrap_or_default())
                .collect::<Vec<_>>()
        });
        let frame = view::compose_frame(FrameInput {
            camera: &v.camera,
            aspect: aspect(rect),
            encode_srgb: !targets.color.is_srgb(),
            avatar: Some(AvatarDraw {
                key: v.avatar.generation << 1 | u64::from(dark),
                mesh: v.avatar.mesh.clone(),
                color: theme.avatar,
            }),
            avatar_bounds: v.avatar.bounds,
            patterns: placed,
            selected: s.selection,
            theme: &theme,
            pattern_colors: strain_colors.as_deref(),
        });
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            SceneCallback { frame },
        ));
    } else {
        // A real label (not painted text) so it is readable by screen
        // readers and tests; it must not take the pointer from the view.
        let label = egui::Label::new(egui::RichText::new(NO_GPU).color(pal.hint))
            .selectable(false)
            .sense(Sense::hover());
        let size = egui::vec2(rect.width().min(320.0), 24.0);
        ui.put(Rect::from_center_size(rect.center(), size), label);
    }

    if let Some(drape) = &v.drape {
        for pos in drape.pin_positions() {
            if let Some(screen) = v.screen_pos(rect, pos) {
                painter.circle_filled(screen, 5.0, pal.selected);
                painter.circle_stroke(screen, 5.0, (1.0, pal.canvas_bg));
            }
        }
        for line in drape.topstitch_positions() {
            let pts: Vec<Pos2> = line.iter().filter_map(|&p| v.screen_pos(rect, p)).collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::dashed_line(&pts, (1.5, pal.preview), 5.0, 3.0));
            }
        }
        for (kind, pos) in drape.trim_positions() {
            if let Some(screen) = v.screen_pos(rect, pos) {
                match kind {
                    TrimKind::Button => {
                        painter.circle_filled(screen, 4.0, pal.selected);
                    }
                    TrimKind::Snap => {
                        painter.circle_stroke(screen, 4.0, (1.5, pal.selected));
                    }
                }
            }
        }
    }

    painter.text(
        rect.left_top() + egui::vec2(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        if v.pinning {
            "Click the cloth to pin or unpin a point"
        } else {
            "Drag: orbit · Right-drag: pan · Scroll: zoom · Double-click: frame · Click: select"
        },
        egui::FontId::proportional(12.0),
        pal.hint,
    );
}

/// A theme colour (sRGB, maybe translucent) as opaque linear RGBA.
fn color(c: Color32) -> [f32; 4] {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    linear([r, g, b, 255])
}

fn sizing_panel(ui: &mut egui::Ui, rect: Rect, s: &mut Session) {
    let width = 240.0;
    let panel = Rect::from_min_size(
        Pos2::new(rect.right() - width - 8.0, rect.top() + 8.0),
        egui::vec2(width, rect.height() - 16.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(panel), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let v = &mut s.viewport;
            ui.strong(format!("Avatar: {}", v.avatar.name()));
            let preset = v.avatar.kind().is_some();
            let mut m = v.avatar.measurements;
            let mut changed = false;
            egui::Grid::new("avatar_sizes")
                .num_columns(2)
                .show(ui, |ui| {
                    let mut row =
                        |ui: &mut egui::Ui, label: &str, value: &mut f32, enabled: bool| {
                            ui.label(label);
                            let range = if label == "Height" {
                                Measurements::HEIGHT_RANGE
                            } else {
                                Measurements::GIRTH_RANGE
                            };
                            changed |= ui
                                .add_enabled(
                                    enabled,
                                    egui::Slider::new(value, range)
                                        .suffix(" cm")
                                        .max_decimals(1),
                                )
                                .changed();
                            ui.end_row();
                        };
                    row(ui, "Height", &mut m.height, true);
                    row(ui, "Chest", &mut m.chest, preset);
                    row(ui, "Waist", &mut m.waist, preset);
                    row(ui, "Hip", &mut m.hip, preset);
                });
            if changed {
                v.avatar.set_measurements(m);
            }
            if !preset {
                ui.weak("Loaded avatars can only be scaled.");
            }
            ui.horizontal(|ui| {
                if let Some(kind) = v.avatar.kind() {
                    if ui.button("Reset sizes").clicked() {
                        v.avatar.set_preset(kind);
                    }
                }
                if ui.button("Load avatar file…").clicked() {
                    v.load_avatar_dialog();
                }
            });
            if let Some(error) = &v.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });
    });
}

struct SceneCallback {
    frame: Frame,
}

impl egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        if let Some(renderer) = resources.get_mut::<Renderer>() {
            renderer.prepare(device, queue, &self.frame);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<Renderer>() {
            renderer.paint(pass);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{OpenDesignApp, Tool};
    use crate::panels::{library, pattern2d::Canvas2D, properties::PropertiesState};
    use egui::{vec2, Event, Modifiers, MouseWheelUnit, TouchPhase};
    use egui_kittest::{kittest::Queryable, Harness};
    use od_core::{geometry, Document, History};
    use od_render::camera::MAX_PITCH;

    fn session() -> Session {
        Session {
            doc: Document::default(),
            history: History::new(),
            selection: None,
            tool: Tool::Select,
            canvas: Canvas2D::default(),
            viewport: Viewport3D::new(None),
            props: PropertiesState::default(),
        }
    }

    /// The Library beside the 3D view, without the rest of the app.
    fn harness() -> Harness<'static, Session> {
        harness_with(Harness::builder())
    }

    fn harness_with(builder: egui_kittest::HarnessBuilder<Session>) -> Harness<'static, Session> {
        let mut h = builder.with_size(vec2(1200.0, 800.0)).build_ui_state(
            |ui, s: &mut Session| {
                egui::Panel::left("library").show(ui, |ui| library::ui(ui, s));
                egui::CentralPanel::default().show(ui, |ui| super::ui(ui, s));
            },
            session(),
        );
        h.run();
        h
    }

    fn view_rect(h: &Harness<'_, Session>) -> Rect {
        h.get_by_label("3D garment view").rect()
    }

    fn click_at(h: &mut Harness<'_, Session>, pos: Pos2) {
        h.hover_at(pos);
        h.run();
        h.drag_at(pos);
        h.run();
        h.drop_at(pos);
        h.run();
    }

    fn drag(h: &mut Harness<'_, Session>, from: Pos2, by: egui::Vec2, button: PointerButton) {
        let press = |h: &mut Harness<'_, Session>, pos, pressed| {
            h.event(Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.run();
        };
        h.hover_at(from);
        h.run();
        press(h, from, true);
        for t in [0.25, 0.5, 0.75, 1.0] {
            h.hover_at(from + by * t);
            h.run();
        }
        press(h, from + by, false);
    }

    fn add_block(h: &mut Harness<'_, Session>) -> PatternId {
        let s = h.state_mut();
        let outline = geometry::rectangle(Vec2::ZERO, Vec2::new(400.0, 500.0));
        s.add_pattern("Panel", outline);
        s.selection = None;
        let id = s.doc.patterns[0].id;
        h.run();
        id
    }

    #[test]
    fn app_shows_the_3d_tab_and_falls_back_without_a_gpu() {
        let mut h = Harness::builder()
            .with_size(vec2(1440.0, 880.0))
            .build_eframe(|cc| OpenDesignApp::new(cc));
        h.run();
        h.get_by_label("3D garment view");
        h.get_by_label(NO_GPU);
        for preset in ViewPreset::ALL {
            h.get_by_label(preset.label());
        }
        h.get_by_label("Frame");
        h.get_by_label("Avatar size");
    }

    #[test]
    fn dragging_orbits_and_scrolling_zooms() {
        let mut h = harness();
        let centre = view_rect(&h).center();
        let before = h.state().viewport.camera;

        drag(&mut h, centre, vec2(120.0, 40.0), PointerButton::Primary);
        let after = h.state().viewport.camera;
        assert!(
            (after.yaw - (before.yaw - 1.2)).abs() < 0.05,
            "yaw {}",
            after.yaw
        );
        assert!(after.pitch > before.pitch);
        assert_eq!(after.target, before.target, "orbit keeps the target");
        assert_eq!(after.distance, before.distance);

        h.hover_at(centre);
        h.event(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
        assert!(
            h.state().viewport.camera.distance < after.distance * 0.9,
            "zoomed in"
        );
    }

    #[test]
    fn drape_button_starts_and_pauses_a_simulation() {
        let mut h = harness();
        add_block(&mut h);
        h.run();

        h.get_by_label("Drape").click();
        h.step();
        assert!(h.state().viewport.draping);
        let drape = h.state().viewport.drape.as_ref().expect("built on toggle");
        let before = drape.world.particles[0].position;

        // A few more frames of simulated gravity should move the cloth.
        // Draping keeps requesting repaints, so step frames individually
        // rather than `run()`, which gives up after a few in that case.
        for _ in 0..10 {
            h.step();
        }
        let after = h.state().viewport.drape.as_ref().unwrap().world.particles[0].position;
        assert_ne!(before, after, "draping should move particles over time");

        h.get_by_label("Stop draping").click();
        h.step();
        assert!(!h.state().viewport.draping);
        assert!(
            h.state().viewport.drape.is_some(),
            "pausing keeps the last pose"
        );

        h.get_by_label("Reset drape").click();
        h.run();
        assert!(h.state().viewport.drape.is_none());
        h.get_by_label("Drape");
    }

    #[test]
    fn trims_and_topstitches_follow_the_drape() {
        use od_core::{AddTopstitch, AddTrim, Command, TrimKind};
        let mut h = harness();
        let id = add_block(&mut h);
        AddTrim::new(id, TrimKind::Button, Vec2::new(0.0, 0.0)).apply(&mut h.state_mut().doc);
        AddTopstitch::new(id, vec![Vec2::new(0.0, 0.0), Vec2::new(400.0, 0.0)])
            .apply(&mut h.state_mut().doc);
        h.run();

        h.get_by_label("Drape").click();
        h.step();
        for _ in 0..5 {
            h.step();
        }
        let drape = h.state().viewport.drape.as_ref().unwrap();
        let trims = drape.trim_positions();
        assert_eq!(trims.len(), 1);
        assert_eq!(trims[0].0, TrimKind::Button);
        let lines = drape.topstitch_positions();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 2);
        // Drawing them in `paint` must not panic.
        h.step();
    }

    #[test]
    fn pin_tool_toggles_a_pin_where_the_cloth_is_clicked() {
        let mut h = harness();
        add_block(&mut h);
        h.run();

        h.get_by_label("Drape").click();
        h.step();
        // One more settled frame before a freshly-shown button's click can
        // reliably register.
        h.step();
        h.get_by_label("Pin").click();
        h.step();
        assert!(h.state().viewport.pinning);

        let rect = view_rect(&h);
        // The cloth keeps falling each step, so find where it currently is
        // right before clicking rather than reusing a stale position.
        let cloth_centre = |h: &Harness<'_, Session>| {
            let s = h.state();
            let placed = s.viewport.drape.as_ref().unwrap().placed_patterns();
            let centre = placed[0].mesh.bounds().unwrap().center();
            s.viewport.screen_pos(rect, centre).unwrap()
        };

        // Like `click_at`, but stepping (not `run`-ing) each frame: while
        // draping is on, every frame requests a repaint, which `run()`
        // treats as never converging.
        let click_while_draping = |h: &mut Harness<'_, Session>, pos: Pos2| {
            h.hover_at(pos);
            h.step();
            h.drag_at(pos);
            h.step();
            h.drop_at(pos);
            h.step();
        };

        let on_cloth = cloth_centre(&h);
        assert!(rect.contains(on_cloth));
        click_while_draping(&mut h, on_cloth);
        assert_eq!(
            h.state()
                .viewport
                .drape
                .as_ref()
                .unwrap()
                .pin_positions()
                .len(),
            1,
            "clicking the cloth while pinning adds a pin"
        );
        // Pinning takes over clicks instead of selecting the pattern.
        assert_eq!(h.state().selection, None);

        // Click just inside the mesh from the pin itself: close enough
        // that it's still the nearest particle, but off the exact vertex,
        // which is a numerically fragile spot for a ray/triangle hit test.
        let near_pin = {
            let s = h.state();
            let drape = s.viewport.drape.as_ref().unwrap();
            let pin = drape.pin_positions()[0];
            let centre = drape.placed_patterns()[0].mesh.bounds().unwrap().center();
            s.viewport
                .screen_pos(rect, pin * 0.9 + centre * 0.1)
                .unwrap()
        };
        click_while_draping(&mut h, near_pin);
        assert!(
            h.state()
                .viewport
                .drape
                .as_ref()
                .unwrap()
                .pin_positions()
                .is_empty(),
            "clicking the same spot again unpins it"
        );
    }

    #[test]
    fn strain_toggle_only_appears_while_draping() {
        let mut h = harness();
        add_block(&mut h);
        h.run();

        // No drape yet: nothing to color by strain.
        assert!(h.query_by_label("Strain").is_none());

        h.get_by_label("Drape").click();
        h.step();
        h.step();
        h.get_by_label("Strain").click();
        h.step();
        assert!(h.state().viewport.show_strain);

        h.get_by_label("Reset drape").click();
        h.run();
        assert!(!h.state().viewport.show_strain, "resetting turns it off");
        assert!(h.query_by_label("Strain").is_none());
    }

    #[test]
    fn right_drag_pans() {
        let mut h = harness();
        let centre = view_rect(&h).center();
        let before = h.state().viewport.camera;
        drag(&mut h, centre, vec2(80.0, 0.0), PointerButton::Secondary);
        let after = h.state().viewport.camera;
        assert_ne!(after.target, before.target);
        assert_eq!((after.yaw, after.pitch), (before.yaw, before.pitch));
    }

    #[test]
    fn view_buttons_set_the_camera() {
        let mut h = harness();
        h.get_by_label("Top").click();
        h.run();
        assert_eq!(h.state().viewport.camera.pitch, MAX_PITCH);

        h.get_by_label("Front").click();
        h.run();
        let cam = h.state().viewport.camera;
        assert_eq!((cam.yaw, cam.pitch), (0.0, 0.0));

        // Frame brings a far-away camera back to the scene.
        h.state_mut().viewport.camera.distance = 25.0;
        h.get_by_label("Frame").click();
        h.run();
        assert!(h.state().viewport.camera.distance < 6.0);
    }

    #[test]
    fn double_click_resets_the_view() {
        // Real frame times: the default step is longer than a double-click.
        let mut h = harness_with(Harness::builder().with_step_dt(1.0 / 60.0));
        let centre = view_rect(&h).center();
        {
            let cam = &mut h.state_mut().viewport.camera;
            cam.yaw = 2.5;
            cam.distance = 20.0;
        }
        h.hover_at(centre);
        h.run();
        for pressed in [true, false, true, false] {
            h.event(Event::PointerButton {
                pos: centre,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        h.run();
        let cam = h.state().viewport.camera;
        assert_eq!(cam.yaw, OrbitCamera::default().yaw);
        assert!(cam.distance < 6.0);
    }

    #[test]
    fn library_presets_switch_the_avatar() {
        let mut h = harness();
        assert_eq!(h.state().viewport.avatar.kind(), Some(AvatarKind::Female));
        for (label, kind, height) in [
            ("Male", AvatarKind::Male, 1.78),
            ("Kid", AvatarKind::Kid, 1.28),
            ("Female", AvatarKind::Female, 1.68),
        ] {
            h.get_by_label(label).click();
            h.run();
            let avatar = &h.state().viewport.avatar;
            assert_eq!(avatar.kind(), Some(kind));
            assert!((avatar.bounds.size().y - height).abs() < 1e-3, "{label}");
        }
    }

    #[test]
    fn sizing_panel_resizes_the_avatar() {
        let mut h = harness();
        h.get_by_label("Avatar size").click();
        h.run();
        h.get_by_label("Avatar: Female");
        for label in ["Height", "Chest", "Waist", "Hip"] {
            h.get_by_label(label);
        }

        let before = h.state().viewport.avatar.bounds;
        let generation = h.state().viewport.avatar.generation;
        let avatar = &mut h.state_mut().viewport.avatar;
        avatar.set_measurements(Measurements {
            height: 180.0,
            hip: 110.0,
            ..avatar.measurements
        });
        h.run();
        let avatar = &h.state().viewport.avatar;
        assert!((avatar.bounds.size().y - 1.8).abs() < 1e-3);
        assert!(avatar.bounds.size().x > before.size().x);
        assert!(avatar.generation > generation, "renderer re-uploads");

        h.get_by_label("Reset sizes").click();
        h.run();
        assert_eq!(
            h.state().viewport.avatar.measurements,
            AvatarKind::Female.default_measurements()
        );
    }

    #[test]
    fn clicking_a_pattern_in_3d_selects_it() {
        let mut h = harness();
        let id = add_block(&mut h);
        let rect = view_rect(&h);
        let s = h.state();
        let placed = placed_patterns(s);
        let centre = placed[0].mesh.bounds().unwrap().center();
        let on_pattern = s.viewport.screen_pos(rect, centre).unwrap();
        assert!(rect.contains(on_pattern));

        click_at(&mut h, on_pattern);
        assert_eq!(h.state().selection, Some(id));

        // Empty space clears it, as in the 2D window.
        click_at(&mut h, rect.left_bottom() + vec2(30.0, -30.0));
        assert_eq!(h.state().selection, None);
    }

    #[test]
    fn picking_respects_the_avatar_and_the_pattern_shape() {
        let mut h = harness();
        let id = add_block(&mut h);
        h.state_mut().viewport.camera.set_view(ViewPreset::Back);
        let rect = view_rect(&h);
        let s = h.state();
        let placed = placed_patterns(s);
        let centre = placed[0].mesh.bounds().unwrap().center();
        // From behind, the body hides the front pattern's centre...
        let behind = s.viewport.screen_pos(rect, centre).unwrap();
        assert_eq!(s.viewport.pick(rect, behind, &placed), None);
        // ...but not its edge, which sticks out past the body.
        let edge = centre + glam::Vec3::new(0.18, 0.0, 0.0);
        let edge = s.viewport.screen_pos(rect, edge).unwrap();
        assert_eq!(s.viewport.pick(rect, edge, &placed), Some(id));
    }

    #[test]
    fn avatar_file_loads_and_errors_are_shown() {
        let dir = std::env::temp_dir().join(format!("od_app_{}_avatar", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tall.obj");
        // A box 1.9 m tall, in centimetres.
        let mut obj = String::new();
        for i in 0..8 {
            let (x, y, z) = ((i & 1) * 40, ((i >> 1) & 1) * 190, ((i >> 2) & 1) * 30);
            obj += &format!("v {x} {y} {z}\n");
        }
        obj += "f 1 3 2\nf 2 3 4\nf 5 6 7\nf 6 8 7\nf 1 2 5\nf 2 6 5\n";
        obj += "f 3 7 4\nf 4 7 8\nf 1 5 3\nf 3 5 7\nf 2 4 6\nf 4 8 6\n";
        std::fs::write(&path, obj).unwrap();

        let mut h = harness();
        h.state_mut().viewport.load_avatar(&path);
        h.run();
        let avatar = &h.state().viewport.avatar;
        assert_eq!(avatar.name(), "tall.obj");
        assert_eq!(avatar.kind(), None);
        assert!((avatar.measurements.height - 190.0).abs() < 0.01);
        assert!((avatar.bounds.size().y - 1.9).abs() < 1e-3);

        // Loaded avatars scale with the height.
        let avatar = &mut h.state_mut().viewport.avatar;
        avatar.set_measurements(Measurements {
            height: 95.0,
            ..avatar.measurements
        });
        assert!((h.state().viewport.avatar.bounds.size().y - 0.95).abs() < 1e-3);

        h.state_mut().viewport.load_avatar(&dir.join("missing.obj"));
        h.run();
        assert!(
            h.state().viewport.show_sizing,
            "the panel opens to show the error"
        );
        assert!(h.query_by_label_contains("Can't read OBJ").is_some());
        assert_eq!(
            h.state().viewport.avatar.name(),
            "tall.obj",
            "kept the old one"
        );
    }

    // ---- The real GPU path, when there is any wgpu adapter ----

    use egui_wgpu::wgpu;

    /// Like kittest's own wgpu renderer, but with the depth buffer and
    /// multisampling main.rs asks eframe for, so the app's 3D pipelines
    /// run inside egui's render pass exactly as in the window.
    struct WindowLikeRenderer {
        rs: egui_wgpu::RenderState,
    }

    impl WindowLikeRenderer {
        fn new() -> Option<Self> {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            pollster_block_on(instance.request_adapter(&Default::default())).ok()?;
            let options = egui_wgpu::RendererOptions {
                msaa_samples: u32::from(od_render::MSAA_SAMPLES),
                depth_stencil_format: Some(od_render::DEPTH_FORMAT),
                ..egui_wgpu::RendererOptions::PREDICTABLE
            };
            let rs = egui_kittest::wgpu::create_render_state(
                egui_kittest::wgpu::default_wgpu_setup(),
                options,
            );
            Some(Self { rs })
        }
    }

    /// A minimal executor; wgpu's futures here resolve without waking.
    fn pollster_block_on<F: std::future::Future>(f: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let mut cx = Context::from_waker(Waker::noop());
        let mut f = std::pin::pin!(f);
        loop {
            if let Poll::Ready(out) = f.as_mut().poll(&mut cx) {
                return out;
            }
            std::thread::yield_now();
        }
    }

    impl egui_kittest::TestRenderer for WindowLikeRenderer {
        fn setup_eframe(&self, cc: &mut eframe::CreationContext<'_>, frame: &mut eframe::Frame) {
            cc.wgpu_render_state = Some(self.rs.clone());
            frame.wgpu_render_state = Some(self.rs.clone());
        }

        fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
            let mut renderer = self.rs.renderer.write();
            for (id, images) in delta.set.drain() {
                for image in images {
                    renderer.update_texture(&self.rs.device, &self.rs.queue, id, &image);
                }
            }
            for id in delta.free.drain() {
                renderer.free_texture(&id);
            }
        }

        fn render(
            &mut self,
            ctx: &egui::Context,
            output: &egui::FullOutput,
        ) -> Result<image::RgbaImage, String> {
            let (device, queue) = (&self.rs.device, &self.rs.queue);
            let size = ctx.content_rect().size() * ctx.pixels_per_point();
            let [w, h] = [size.x.round() as u32, size.y.round() as u32];
            let screen = egui_wgpu::ScreenDescriptor {
                pixels_per_point: ctx.pixels_per_point(),
                size_in_pixels: [w, h],
            };
            let tessellated = ctx.tessellate(output.shapes.clone(), ctx.pixels_per_point());
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut renderer = self.rs.renderer.write();
            let user_buffers =
                renderer.update_buffers(device, queue, &mut encoder, &tessellated, &screen);

            let texture = |samples, format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            use wgpu::TextureUsages as U;
            let samples = u32::from(od_render::MSAA_SAMPLES);
            let color = self.rs.target_format;
            let resolved = texture(1, color, U::RENDER_ATTACHMENT | U::COPY_SRC);
            let msaa = texture(samples, color, U::RENDER_ATTACHMENT);
            let depth = texture(samples, od_render::DEPTH_FORMAT, U::RENDER_ATTACHMENT);
            let (resolved_view, msaa_view, depth_view) = (
                resolved.create_view(&Default::default()),
                msaa.create_view(&Default::default()),
                depth.create_view(&Default::default()),
            );
            {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &msaa_view,
                            depth_slice: None,
                            resolve_target: Some(&resolved_view),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    })
                    .forget_lifetime();
                renderer.render(&mut pass, &tessellated, &screen);
            }

            let row = w * 4;
            let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(padded * h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                resolved.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(user_buffers.into_iter().chain([encoder.finish()]));
            readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| e.to_string())?;
            let data = readback
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("{e:?}"))?;
            let mut pixels = Vec::with_capacity((row * h) as usize);
            for r in data.chunks_exact(padded as usize) {
                pixels.extend_from_slice(&r[..row as usize]);
            }
            if color == wgpu::TextureFormat::Bgra8Unorm {
                for px in pixels.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
            }
            image::RgbaImage::from_raw(w, h, pixels).ok_or("bad image size".into())
        }
    }

    /// The whole app on a real (often software) GPU: the avatar is drawn,
    /// shaded and depth-tested, inside the 3D tab, in both themes.
    #[test]
    fn app_renders_the_scene_on_a_gpu() {
        let Some(renderer) = WindowLikeRenderer::new() else {
            eprintln!("no GPU adapter; skipping the full-app render");
            return;
        };
        let mut h = Harness::builder()
            .with_size(vec2(1440.0, 880.0))
            .renderer(renderer)
            .build_eframe(|cc| OpenDesignApp::new(cc));
        h.run();
        assert!(h.query_by_label(NO_GPU).is_none(), "the GPU path is in use");
        h.get_by_label("+ Front bodice").click();
        h.run();
        let view = h.get_by_label("3D garment view").rect();

        for (name, dark) in [("dark", true), ("light", false)] {
            h.ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            h.run();
            let image = h.render().expect("render");
            if let Ok(dir) = std::env::var("OD_SNAPSHOT_DIR") {
                image
                    .save(Path::new(&dir).join(format!("app_{name}.png")))
                    .unwrap();
            }
            let pal = theme::palette(dark);
            let px = |p: Pos2| {
                let c = image.get_pixel(p.x as u32, p.y as u32).0;
                Color32::from_rgb(c[0], c[1], c[2])
            };
            // Background in the view's top-right corner, below the hint.
            assert_eq!(
                px(view.right_top() + vec2(-5.0, 40.0)),
                pal.canvas_bg,
                "{name}"
            );
            // The avatar is lit: its pixels are near the avatar colour but
            // vary with the surface angle.
            let body: Vec<Color32> = (0..40)
                .map(|i| px(view.center() + vec2(0.0, -60.0 + i as f32 * 3.0)))
                .collect();
            let near_avatar = |c: &&Color32| {
                let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs();
                d(c.r(), pal.avatar.r()) < 70 && d(c.g(), pal.avatar.g()) < 70
            };
            assert!(
                body.iter().filter(near_avatar).count() > 10,
                "{name}: avatar not drawn: {body:?}"
            );
            let lightest = body.iter().map(|c| c.r()).max().unwrap();
            let darkest = body.iter().map(|c| c.r()).min().unwrap();
            assert!(lightest > darkest + 10, "{name}: shading varies");
        }

        // For a look at the sizing panel over the scene.
        if let Ok(dir) = std::env::var("OD_SNAPSHOT_DIR") {
            h.get_by_label("Male").click();
            h.get_by_label("Avatar size").click();
            h.run();
            let image = h.render().expect("render");
            image.save(Path::new(&dir).join("app_sizing.png")).unwrap();
        }
    }
}
