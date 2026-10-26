//! Application shell: menu bar, toolbar, status bar and the docked workspace.

use egui::{Key, KeyboardShortcut, Modifiers};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use od_core::{AddPattern, Command, DeletePattern, Document, History, PatternId, Vec2};

use crate::panels::{
    self, pattern2d::Canvas2D, properties::PropertiesState, viewport3d::Viewport3D,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Viewport3D,
    Pattern2D,
    Library,
    Properties,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rectangle,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select / Move",
            Tool::Rectangle => "Rectangle",
        }
    }
}

/// Everything the panels can see and change. Kept apart from the dock
/// layout so it can be lent to the tab viewer while the dock is drawn.
pub struct Session {
    pub doc: Document,
    pub history: History,
    pub selection: Option<PatternId>,
    pub tool: Tool,
    pub canvas: Canvas2D,
    pub viewport: Viewport3D,
    pub props: PropertiesState,
}

impl Session {
    pub fn execute(&mut self, command: impl Command + 'static) {
        self.history.execute(&mut self.doc, Box::new(command));
    }

    /// Add a pattern and select it.
    pub fn add_pattern(&mut self, name: impl Into<String>, outline: Vec<Vec2>) {
        self.execute(AddPattern::new(name, outline));
        self.selection = self.doc.patterns.last().map(|p| p.id);
    }

    pub fn undo(&mut self) {
        self.history.undo(&mut self.doc);
    }

    pub fn redo(&mut self) {
        self.history.redo(&mut self.doc);
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selection.take() {
            self.execute(DeletePattern::new(id));
        }
    }

    pub fn new_document(&mut self) {
        self.doc = Document::default();
        self.history.clear();
        self.selection = None;
    }
}

pub struct OpenDesignApp {
    dock: DockState<Tab>,
    session: Session,
    show_about: bool,
}

impl OpenDesignApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            dock: default_layout(),
            session: Session {
                doc: Document::default(),
                history: History::new(),
                selection: None,
                tool: Tool::Select,
                canvas: Canvas2D::default(),
                viewport: Viewport3D::default(),
                props: PropertiesState::default(),
            },
            show_about: false,
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let s = &mut self.session;
        let redo_shift = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        // Check Ctrl+Shift+Z before Ctrl+Z so the shorter one doesn't swallow it.
        if ctx.input_mut(|i| i.consume_shortcut(&redo_shift) || i.consume_shortcut(&redo)) {
            s.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
            s.undo();
        }

        // Single-key shortcuts must not fire while typing in a text field.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        ctx.input(|i| {
            if i.key_pressed(Key::V) {
                s.tool = Tool::Select;
            }
            if i.key_pressed(Key::R) {
                s.tool = Tool::Rectangle;
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
            s.delete_selected();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            let s = &mut self.session;
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    s.new_document();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = s
                    .history
                    .undo_label()
                    .map_or("Undo".into(), |l| format!("Undo {l}"));
                if ui
                    .add_enabled(
                        s.history.can_undo(),
                        egui::Button::new(undo).shortcut_text("Ctrl+Z"),
                    )
                    .clicked()
                {
                    s.undo();
                }
                let redo = s
                    .history
                    .redo_label()
                    .map_or("Redo".into(), |l| format!("Redo {l}"));
                if ui
                    .add_enabled(
                        s.history.can_redo(),
                        egui::Button::new(redo).shortcut_text("Ctrl+Y"),
                    )
                    .clicked()
                {
                    s.redo();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        s.selection.is_some(),
                        egui::Button::new("Delete").shortcut_text("Del"),
                    )
                    .clicked()
                {
                    s.delete_selected();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Reset layout").clicked() {
                    self.dock = default_layout();
                }
                ui.separator();
                egui::widgets::global_theme_preference_buttons(ui);
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About OpenDesign").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.session;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut s.tool, Tool::Select, "Select")
                .on_hover_text("Select and move patterns (V)");
            ui.selectable_value(&mut s.tool, Tool::Rectangle, "Rectangle")
                .on_hover_text("Drag in the 2D window to draw a rectangle pattern (R)");
            ui.separator();
            if ui
                .add_enabled(s.history.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                s.undo();
            }
            if ui
                .add_enabled(s.history.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                s.redo();
            }
            ui.separator();
            for (label, phase) in [
                ("Sew", "Phase 5"),
                ("Pin", "Phase 6"),
                ("▶ Simulate", "Phase 6"),
            ] {
                ui.add_enabled(false, egui::Button::new(label))
                    .on_disabled_hover_text(format!("Coming in {phase}"));
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let s = &self.session;
        ui.horizontal(|ui| {
            ui.label(format!("Tool: {}", s.tool.name()));
            ui.separator();
            let selected = s
                .selection
                .and_then(|id| s.doc.pattern(id))
                .map_or("Nothing selected".to_owned(), |p| {
                    format!("Selected: {}", p.name)
                });
            ui.label(selected);
            ui.separator();
            ui.label(format!("{} patterns", s.doc.patterns.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(&s.doc.name);
            });
        });
    }
}

impl eframe::App for OpenDesignApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);

        // Undo can remove the selected pattern; drop a stale selection.
        let s = &mut self.session;
        if s.selection.is_some_and(|id| s.doc.pattern(id).is_none()) {
            s.selection = None;
        }

        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                DockArea::new(&mut self.dock)
                    .style(Style::from_egui(ui.style()))
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut self.session);
            });

        egui::Window::new("About OpenDesign")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(&ctx, |ui| {
                ui.label("OpenDesign: open-source 3D garment design.");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            });
    }
}

impl TabViewer for Session {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Viewport3D => "3D Garment",
            Tab::Pattern2D => "2D Pattern",
            Tab::Library => "Library",
            Tab::Properties => "Properties",
            Tab::History => "History",
        }
        .into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Viewport3D => panels::viewport3d::ui(ui, self),
            Tab::Pattern2D => panels::pattern2d::ui(ui, self),
            Tab::Library => panels::library::ui(ui, self),
            Tab::Properties => panels::properties::ui(ui, self),
            Tab::History => panels::history::ui(ui, self),
        }
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        // Canvases handle their own panning.
        let side_panel = matches!(tab, Tab::Library | Tab::Properties | Tab::History);
        [false, side_panel]
    }
}

/// Library on the left, 3D and 2D side by side, Properties/History on the right.
fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Viewport3D]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.8, vec![Tab::Properties, Tab::History]);
    // Fractions size the left/top child, which for split_left is the new node.
    let [center, _] = tree.split_left(center, 0.2, vec![Tab::Library]);
    tree.split_right(center, 0.5, vec![Tab::Pattern2D]);
    dock
}

/// Drive the real app headlessly: clicks and keys go through egui exactly as
/// they would from a mouse and keyboard, no window or GPU needed.
#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{kittest::Queryable, Harness};

    fn harness() -> Harness<'static, OpenDesignApp> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 880.0))
            .build_eframe(|cc| OpenDesignApp::new(cc));
        h.run();
        h
    }

    fn pattern_count(h: &Harness<'_, OpenDesignApp>) -> usize {
        h.state().session.doc.patterns.len()
    }

    #[test]
    fn shell_shows_menus_and_all_panels() {
        let h = harness();
        for label in ["File", "Edit", "View", "Help"] {
            h.get_by_label(label);
        }
        // One landmark per visible tab (tab titles aren't accessibility nodes).
        h.get_by_label("3D garment view");
        h.get_by_label("2D pattern canvas");
        h.get_by_label("+ Front bodice"); // Library
        h.get_by_label("Nothing selected."); // Properties
        h.get_by_label("Nothing selected"); // status bar

        // The two views should share the middle, with the side panels narrower.
        let view_3d = h.get_by_label("3D garment view").rect().width();
        let view_2d = h.get_by_label("2D pattern canvas").rect().width();
        let library = h.get_by_label("+ Front bodice").rect().left();
        assert!(
            view_3d > 350.0 && view_2d > 350.0,
            "3D {view_3d}, 2D {view_2d}"
        );
        assert!(library < 50.0, "library should hug the left edge");
    }

    #[test]
    fn library_block_supports_undo_and_redo() {
        let mut h = harness();
        h.get_by_label("+ Front bodice").click();
        h.run();
        assert_eq!(pattern_count(&h), 1);
        h.get_by_label("Selected: Front bodice");

        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(pattern_count(&h), 0);
        h.get_by_label("Nothing selected");

        h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        h.run();
        assert_eq!(pattern_count(&h), 1);
    }

    #[test]
    fn delete_key_removes_selection_and_undo_restores_it() {
        let mut h = harness();
        h.get_by_label("+ Sleeve").click();
        h.run();
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(pattern_count(&h), 0);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(pattern_count(&h), 1);
    }

    #[test]
    fn rectangle_tool_draws_a_pattern() {
        let mut h = harness();
        h.get_by_label("Rectangle").click();
        h.run();

        let start = h.get_by_label("2D pattern canvas").rect().center();
        let end = start + egui::vec2(150.0, 90.0);
        h.drag_at(start);
        h.run();
        for t in [0.25, 0.5, 0.75, 1.0] {
            h.hover_at(start + (end - start) * t);
            h.run();
        }
        h.drop_at(end);
        h.run();

        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 1);
        let (min, max) = s.doc.patterns[0].bounds().unwrap();
        let zoom = s.canvas.zoom();
        assert!(
            (max.x - min.x - 150.0 / zoom).abs() < 1.0,
            "width {}",
            max.x - min.x
        );
        assert!(
            (max.y - min.y - 90.0 / zoom).abs() < 1.0,
            "height {}",
            max.y - min.y
        );
        h.get_by_label("Selected: Pattern 1");
    }

    #[test]
    fn select_tool_moves_pattern_as_one_undo_step() {
        let mut h = harness();
        h.get_by_label("+ Back bodice").click();
        h.run();
        let before = h.state().session.doc.patterns[0].outline.clone();

        h.get_by_label("Select").click();
        h.run();
        // Grab the pattern by its centre, wherever the canvas drew it.
        let s = &h.state().session;
        let (min, max) = s.doc.patterns[0].bounds().unwrap();
        let start = s.canvas.screen_pos((min + max) * 0.5);
        let end = start + egui::vec2(60.0, 30.0);
        h.drag_at(start);
        h.run();
        for t in [0.5, 1.0] {
            h.hover_at(start + (end - start) * t);
            h.run();
        }
        h.drop_at(end);
        h.run();

        let after = &h.state().session.doc.patterns[0].outline;
        assert_ne!(&before, after, "pattern should have moved");
        assert_eq!(
            h.state().session.history.done_labels().last().unwrap(),
            "Move pattern"
        );
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(h.state().session.doc.patterns[0].outline, before);
    }
}
