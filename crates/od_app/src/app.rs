//! Application shell: menu bar, toolbar, status bar and the docked workspace.

use egui::{Key, KeyboardShortcut, Modifiers};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use od_core::{AddPattern, Command, DeletePattern, Document, History, PatternId, Vec2};

use crate::files::{Discard, FileState, RECENT_KEY};
use crate::panels::{
    self, pattern2d::Canvas2D, properties::PropertiesState, viewport3d::Viewport3D,
};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Viewport3D,
    Pattern2D,
    Library,
    Properties,
    History,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Viewport3D,
        Tab::Pattern2D,
        Tab::Library,
        Tab::Properties,
        Tab::History,
    ];
}

/// Storage key for the dock layout. Bump the suffix if `Tab` changes shape.
const LAYOUT_KEY: &str = "dock_layout_v1";

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

    /// Replace the document, e.g. after File → New or Open.
    pub fn load_document(&mut self, doc: Document) {
        self.doc = doc;
        self.history.clear();
        self.selection = None;
        self.props = PropertiesState::default();
    }
}

pub struct OpenDesignApp {
    dock: DockState<Tab>,
    session: Session,
    files: FileState,
    show_about: bool,
}

impl OpenDesignApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        Self {
            dock: restore_layout(cc.storage),
            session: Session {
                doc: Document::default(),
                history: History::new(),
                selection: None,
                tool: Tool::Select,
                canvas: Canvas2D::default(),
                viewport: Viewport3D::new(cc.wgpu_render_state.as_ref()),
                props: PropertiesState::default(),
            },
            files: FileState::with_recent(
                cc.storage
                    .and_then(|s| eframe::get_value(s, RECENT_KEY))
                    .unwrap_or_default(),
            ),
            show_about: false,
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (s, files) = (&mut self.session, &mut self.files);
        let shortcut =
            |mods, key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)));
        // Longer chords first, so Ctrl+S doesn't swallow Ctrl+Shift+S.
        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            files.save_as(s);
        } else if shortcut(Modifiers::COMMAND, Key::S) {
            files.save(s);
        } else if shortcut(Modifiers::COMMAND, Key::N) {
            files.request(s, ctx, Discard::New);
        } else if shortcut(Modifiers::COMMAND, Key::O) {
            files.request(s, ctx, Discard::Open(None));
        } else if shortcut(Modifiers::COMMAND, Key::Q) {
            files.request(s, ctx, Discard::Quit);
        }

        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || shortcut(Modifiers::COMMAND, Key::Y)
        {
            s.redo();
        } else if shortcut(Modifiers::COMMAND, Key::Z) {
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
            let (s, files) = (&mut self.session, &mut self.files);
            ui.menu_button("File", |ui| {
                let ctx = ui.ctx().clone();
                let item = |label, keys| egui::Button::new(label).shortcut_text(keys);
                if ui.add(item("New", "Ctrl+N")).clicked() {
                    files.request(s, &ctx, Discard::New);
                }
                if ui.add(item("Open…", "Ctrl+O")).clicked() {
                    files.request(s, &ctx, Discard::Open(None));
                }
                ui.add_enabled_ui(!files.recent.is_empty(), |ui| {
                    ui.menu_button("Open Recent", |ui| {
                        let mut chosen = None;
                        for path in &files.recent {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui
                                .button(name)
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                chosen = Some(path.clone());
                            }
                        }
                        ui.separator();
                        if ui.button("Clear recent").clicked() {
                            files.recent.clear();
                        }
                        if let Some(path) = chosen {
                            files.request(s, &ctx, Discard::Open(Some(path)));
                        }
                    });
                });
                ui.separator();
                if ui.add(item("Save", "Ctrl+S")).clicked() {
                    files.save(s);
                }
                if ui.add(item("Save As…", "Ctrl+Shift+S")).clicked() {
                    files.save_as(s);
                }
                ui.separator();
                if ui.add(item("Quit", "Ctrl+Q")).clicked() {
                    files.request(s, &ctx, Discard::Quit);
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
                match &self.files.path {
                    Some(path) => ui.weak(path.display().to_string()),
                    None => ui.weak("Not saved yet"),
                };
                if s.history.is_dirty() {
                    ui.label("Unsaved changes");
                }
            });
        });
    }
}

impl eframe::App for OpenDesignApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.files.intercept_close(&self.session, &ctx);
        // While a dialog is up, the document must not change underneath it.
        if !self.files.is_blocking() {
            self.files.handle_dropped(&mut self.session, &ctx);
            self.handle_shortcuts(&ctx);
        }

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

        self.files.dialogs(&mut self.session, &ctx);
        self.files.update_title(&self.session, &ctx);
    }

    // eframe also saves egui's memory, which holds the theme preference.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_KEY, &self.dock);
        eframe::set_value(storage, RECENT_KEY, &self.files.recent);
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

/// The layout saved last session, or the default if there is none or it is
/// missing a tab (e.g. saved by an older version).
fn restore_layout(storage: Option<&dyn eframe::Storage>) -> DockState<Tab> {
    storage
        .and_then(|s| eframe::get_value::<DockState<Tab>>(s, LAYOUT_KEY))
        .filter(|dock| Tab::ALL.iter().all(|tab| dock.find_tab(tab).is_some()))
        .unwrap_or_else(default_layout)
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

    /// In-memory stand-in for eframe's on-disk storage.
    #[derive(Default)]
    struct MemStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    /// Where every tab sits, for comparing layouts.
    fn positions(dock: &DockState<Tab>) -> Vec<Option<egui_dock::TabPath>> {
        Tab::ALL.iter().map(|tab| dock.find_tab(tab)).collect()
    }

    /// All tabs stacked in one node: clearly different from the default.
    fn stacked_layout() -> DockState<Tab> {
        DockState::new(Tab::ALL.to_vec())
    }

    #[test]
    fn saved_layout_is_restored_next_launch() {
        let mut h = harness();
        h.state_mut().dock = stacked_layout();
        let mut storage = MemStorage::default();
        eframe::App::save(h.state_mut(), &mut storage);

        let restored = restore_layout(Some(&storage));
        assert_eq!(positions(&restored), positions(&stacked_layout()));
    }

    #[test]
    fn unusable_saved_layout_falls_back_to_default() {
        let default = positions(&default_layout());
        assert_eq!(positions(&restore_layout(None)), default);

        let mut storage = MemStorage::default();
        eframe::Storage::set_string(&mut storage, LAYOUT_KEY, "not a layout".into());
        assert_eq!(positions(&restore_layout(Some(&storage))), default);

        // A layout from an older version without the History tab.
        let old = DockState::new(vec![Tab::Viewport3D, Tab::Pattern2D, Tab::Library]);
        eframe::set_value(&mut storage, LAYOUT_KEY, &old);
        assert_eq!(positions(&restore_layout(Some(&storage))), default);
    }

    #[test]
    fn view_menu_resets_layout() {
        let mut h = harness();
        h.state_mut().dock = stacked_layout();
        h.run();
        h.get_by_label("View").click();
        h.run();
        h.get_by_label("Reset layout").click();
        h.run();
        assert_eq!(positions(&h.state().dock), positions(&default_layout()));
    }

    // ---- Files (Phase 2) ----

    use crate::files::window_title;
    use std::path::{Path, PathBuf};

    /// A fresh, empty directory for one test.
    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("od_app_{}_{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn click(h: &mut Harness<'_, OpenDesignApp>, label: &str) {
        h.get_by_label(label).click();
        h.run();
    }

    fn press(h: &mut Harness<'_, OpenDesignApp>, mods: Modifiers, key: Key) {
        h.key_press_modifiers(mods, key);
        h.run();
    }

    fn open_recent(h: &mut Harness<'_, OpenDesignApp>, file_name: &str) {
        click(h, "File");
        click(h, "Open Recent ⏵");
        click(h, file_name);
    }

    fn dirty(h: &Harness<'_, OpenDesignApp>) -> bool {
        h.state().session.history.is_dirty()
    }

    #[test]
    fn save_close_reopen_keeps_document_and_editing_continues() {
        let path = temp_dir("reopen").join("shirt.odproj");

        // First session: make something and save it.
        let mut first = harness();
        click(&mut first, "+ Front bodice");
        click(&mut first, "+ Sleeve");
        assert!(dirty(&first));
        first.state_mut().files.path = Some(path.clone());
        press(&mut first, Modifiers::COMMAND, Key::S);
        assert!(path.exists());
        assert!(!dirty(&first));
        assert_eq!(window_title(&first.state().session), "shirt - OpenDesign");
        let saved = first.state().session.doc.clone();
        drop(first);

        // Second session: reopen it from Open Recent.
        let mut second = harness();
        second.state_mut().files.recent = vec![path.clone()];
        open_recent(&mut second, "shirt.odproj");
        assert_eq!(second.state().session.doc, saved);
        assert_eq!(second.state().files.path.as_deref(), Some(path.as_path()));
        assert!(!dirty(&second));

        // Editing and undo carry on normally.
        click(&mut second, "+ Skirt panel");
        assert_eq!(pattern_count(&second), 3);
        assert_eq!(
            window_title(&second.state().session),
            "shirt * - OpenDesign"
        );
        press(&mut second, Modifiers::COMMAND, Key::Z);
        assert_eq!(pattern_count(&second), 2);
        assert!(!dirty(&second), "undo back to the saved state");
    }

    #[test]
    fn new_with_unsaved_changes_asks_first() {
        let mut h = harness();
        click(&mut h, "+ Back bodice");

        press(&mut h, Modifiers::COMMAND, Key::N);
        h.get_by_label("Save changes?");
        click(&mut h, "Cancel");
        assert!(h.query_by_label("Save changes?").is_none());
        assert_eq!(pattern_count(&h), 1, "cancel keeps the work");

        press(&mut h, Modifiers::COMMAND, Key::N);
        click(&mut h, "Don't save");
        assert_eq!(pattern_count(&h), 0);
        assert!(!dirty(&h));
    }

    #[test]
    fn save_in_the_prompt_saves_then_continues() {
        let path = temp_dir("prompt_save").join("coat.odproj");
        let mut h = harness();
        h.state_mut().files.path = Some(path.clone());
        click(&mut h, "+ Front bodice");

        press(&mut h, Modifiers::COMMAND, Key::N);
        click(&mut h, "Save");
        assert_eq!(od_core::project::load(&path).unwrap().patterns.len(), 1);
        assert_eq!(pattern_count(&h), 0, "then the new document");
        assert_eq!(h.state().files.path, None);
    }

    #[test]
    fn unchanged_document_is_replaced_without_asking() {
        let mut h = harness();
        press(&mut h, Modifiers::COMMAND, Key::N);
        assert!(h.query_by_label("Save changes?").is_none());
    }

    #[test]
    fn closing_the_window_with_unsaved_changes_asks_first() {
        let mut h = harness();
        click(&mut h, "+ Sleeve");
        let request_close = |h: &mut Harness<'_, OpenDesignApp>| {
            h.input_mut()
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
            h.run();
        };

        request_close(&mut h);
        h.get_by_label("Save changes?");
        click(&mut h, "Cancel");
        assert!(!h.state().files.quitting, "cancel keeps the window open");

        request_close(&mut h);
        click(&mut h, "Don't save");
        assert!(h.state().files.quitting);
    }

    #[test]
    fn missing_recent_file_shows_error_and_is_forgotten() {
        let gone = temp_dir("missing").join("gone.odproj");
        let mut h = harness();
        h.state_mut().files.recent = vec![gone];
        open_recent(&mut h, "gone.odproj");
        h.get_by_label("Something went wrong");
        assert!(h.state().files.recent.is_empty());
        click(&mut h, "OK");
        assert!(h.query_by_label("Something went wrong").is_none());
    }

    #[test]
    fn damaged_file_shows_error_and_keeps_current_work() {
        let path = temp_dir("damaged").join("broken.odproj");
        std::fs::write(&path, "{ this is not a project").unwrap();
        let mut h = harness();
        click(&mut h, "+ Sleeve");
        h.state_mut().files.recent = vec![path];
        open_recent(&mut h, "broken.odproj");
        click(&mut h, "Don't save");
        h.get_by_label("Something went wrong");
        assert_eq!(
            pattern_count(&h),
            1,
            "failed open leaves the document alone"
        );
    }

    #[derive(Debug)]
    struct DroppedPath(PathBuf);

    impl egui::DroppedFile for DroppedPath {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn dropping_a_project_file_opens_it() {
        let path = temp_dir("drop").join("dropped.odproj");
        let mut doc = Document::new("dropped");
        let outline = od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(300.0, 80.0));
        AddPattern::new("Yoke", outline).apply(&mut doc);
        od_core::project::save(&doc, &path).unwrap();

        let mut h = harness();
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(DroppedPath(path.clone())));
        h.run();
        assert_eq!(h.state().session.doc.patterns[0].name, "Yoke");
        assert_eq!(h.state().files.path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn recent_files_are_saved_for_next_launch() {
        let mut h = harness();
        let recent = vec![PathBuf::from("a.odproj"), PathBuf::from("b.odproj")];
        h.state_mut().files.recent = recent.clone();
        let mut storage = MemStorage::default();
        eframe::App::save(h.state_mut(), &mut storage);
        let stored: Vec<PathBuf> = eframe::get_value(&storage, RECENT_KEY).unwrap();
        assert_eq!(stored, recent);
    }

    #[test]
    fn theme_switch_uses_tokens() {
        let mut h = harness();
        h.get_by_label("View").click();
        h.run();
        h.get_by_label("☀ Light").click();
        h.run();
        assert_eq!(
            h.ctx.global_style().visuals.panel_fill,
            theme::LIGHT.panel_bg
        );

        h.get_by_label("View").click();
        h.run();
        h.get_by_label("🌙 Dark").click();
        h.run();
        assert_eq!(
            h.ctx.global_style().visuals.panel_fill,
            theme::DARK.panel_bg
        );
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
