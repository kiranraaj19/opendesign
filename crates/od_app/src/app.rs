//! Application shell: menu bar, toolbar, status bar and the docked workspace.

use egui::{Key, KeyboardShortcut, Modifiers};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use od_core::{
    AddPattern, Axis, Command, DeletePattern, DeletePoint, Document, DuplicatePattern, History,
    MirrorPattern, OutlinePoint, PatternId, SetPointKind, Vec2,
};

use crate::files::{Discard, FileState, RECENT_KEY};
use crate::i18n::{self, Key as TKey, Locale};
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

/// Storage key for the UI language preference (see `crate::i18n`).
const LOCALE_KEY: &str = "locale_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rectangle,
    Pen,
    EditPoints,
    InternalLine,
    Notch,
    Sew,
    Topstitch,
    Trim,
}

impl Tool {
    /// Toolbar order, with button label, shortcut key and tooltip.
    const BUTTONS: [(Tool, &'static str, Key, &'static str); 9] = [
        (
            Tool::Select,
            "Select",
            Key::V,
            "Select and move patterns (V)",
        ),
        (
            Tool::Rectangle,
            "Rectangle",
            Key::R,
            "Drag in the 2D window to draw a rectangle pattern (R)",
        ),
        (
            Tool::Pen,
            "Pen",
            Key::P,
            "Click points to draft a pattern outline; Shift+click for a curve point (P)",
        ),
        (
            Tool::EditPoints,
            "Edit points",
            Key::E,
            "Drag, add, delete and curve outline points (E)",
        ),
        (
            Tool::InternalLine,
            "Internal line",
            Key::L,
            "Draw a line inside a pattern (L)",
        ),
        (
            Tool::Notch,
            "Notch",
            Key::N,
            "Add or remove notches on an outline (N)",
        ),
        (
            Tool::Sew,
            "Sew",
            Key::S,
            "Click two edges to sew them together; click a sewn edge to unsew it (S)",
        ),
        (
            Tool::Topstitch,
            "Topstitch",
            Key::T,
            "Draw a topstitch line inside a pattern (T)",
        ),
        (
            Tool::Trim,
            "Trim",
            Key::B,
            "Click to place a button; Shift+click for a snap; click a trim to remove it (B)",
        ),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select / Move",
            Tool::Rectangle => "Rectangle",
            Tool::Pen => "Pen",
            Tool::EditPoints => "Edit points",
            Tool::InternalLine => "Internal line",
            Tool::Notch => "Notch",
            Tool::Sew => "Sew",
            Tool::Topstitch => "Topstitch",
            Tool::Trim => "Trim",
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
    /// Index into `doc.size_chart.names` the 2D canvas, properties and BOM
    /// previews currently show. Not persisted: a document always opens
    /// showing its base size.
    pub preview_size: usize,
}

impl Session {
    pub fn execute(&mut self, command: impl Command + 'static) {
        self.history.execute(&mut self.doc, Box::new(command));
    }

    /// Add a straight-edged pattern and select it.
    pub fn add_pattern(&mut self, name: impl Into<String>, outline: Vec<Vec2>) {
        self.execute(AddPattern::new(name, outline));
        self.selection = self.doc.patterns.last().map(|p| p.id);
    }

    /// Add a pattern with corner and curve points and select it.
    pub fn add_outline(&mut self, name: impl Into<String>, outline: Vec<OutlinePoint>) {
        self.execute(AddPattern::with_outline(name, outline));
        self.selection = self.doc.patterns.last().map(|p| p.id);
    }

    pub fn undo(&mut self) {
        self.history.undo(&mut self.doc);
    }

    pub fn redo(&mut self) {
        self.history.redo(&mut self.doc);
    }

    /// Delete the selected point while editing points, else the pattern.
    pub fn delete_selected(&mut self) {
        if let Some((id, index)) = self.canvas.selected_point.take() {
            // An outline needs three points; keep the point selected if so.
            if self.doc.pattern(id).is_some_and(|p| p.outline.len() > 3) {
                self.execute(DeletePoint::new(id, index));
            } else {
                self.canvas.selected_point = Some((id, index));
            }
            return;
        }
        if let Some(id) = self.selection.take() {
            self.execute(DeletePattern::new(id));
        }
    }

    /// Switch the selected point between corner and curve.
    pub fn toggle_selected_point(&mut self) {
        let Some((id, index)) = self.canvas.selected_point else {
            return;
        };
        if let Some(kind) = self
            .doc
            .pattern(id)
            .and_then(|p| p.outline.get(index))
            .map(|p| p.kind)
        {
            self.execute(SetPointKind::new(id, index, kind.toggled()));
        }
    }

    /// Flip the selected pattern in place.
    pub fn mirror_selected(&mut self, axis: Axis) {
        if let Some(id) = self.selection {
            self.execute(MirrorPattern::new(id, axis));
        }
    }

    /// Copy the selected pattern, optionally mirrored, and select the copy.
    pub fn duplicate_selected(&mut self, mirror: Option<Axis>) {
        if let Some(id) = self.selection {
            self.execute(DuplicatePattern::new(id, mirror));
            self.selection = self.doc.patterns.last().map(|p| p.id);
        }
    }

    /// Drop selections that undo or redo made stale.
    fn fix_selection(&mut self) {
        if self
            .selection
            .is_some_and(|id| self.doc.pattern(id).is_none())
        {
            self.selection = None;
        }
        if let Some((id, index)) = self.canvas.selected_point {
            let valid = self.selection == Some(id)
                && self
                    .doc
                    .pattern(id)
                    .is_some_and(|p| index < p.outline.len());
            if !valid {
                self.canvas.selected_point = None;
            }
        }
    }

    /// Replace the document, e.g. after File → New or Open.
    pub fn load_document(&mut self, doc: Document) {
        self.preview_size = doc.size_chart.base;
        self.doc = doc;
        self.history.clear();
        self.selection = None;
        self.props = PropertiesState::default();
    }

    /// How many sizes [`Session::preview_size`] is from the document's base
    /// size (see [`od_core::SizeChart::steps`]), for grading a pattern or
    /// computing its BOM at the size currently being previewed.
    pub fn preview_steps(&self) -> i32 {
        self.doc.size_chart.steps(self.preview_size)
    }
}

pub struct OpenDesignApp {
    dock: DockState<Tab>,
    session: Session,
    files: FileState,
    show_about: bool,
    locale: Locale,
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
                preview_size: Document::default().size_chart.base,
            },
            files: FileState::with_recent(
                cc.storage
                    .and_then(|s| eframe::get_value(s, RECENT_KEY))
                    .unwrap_or_default(),
            ),
            show_about: false,
            locale: cc
                .storage
                .and_then(|s| eframe::get_value(s, LOCALE_KEY))
                .unwrap_or_default(),
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
        if shortcut(Modifiers::COMMAND, Key::D) {
            s.duplicate_selected(None);
        }

        // Single-key shortcuts must not fire while typing in a text field.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        ctx.input(|i| {
            for (tool, _, key, _) in Tool::BUTTONS {
                if i.key_pressed(key) {
                    s.tool = tool;
                }
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
            s.delete_selected();
        }
        if ctx.input(|i| i.key_pressed(Key::C)) {
            s.toggle_selected_point();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let locale = self.locale;
        let tr = |key| i18n::t(locale, key);
        egui::MenuBar::new().ui(ui, |ui| {
            let (s, files) = (&mut self.session, &mut self.files);
            ui.menu_button(tr(TKey::MenuFile), |ui| {
                let ctx = ui.ctx().clone();
                let item = |label, keys| egui::Button::new(label).shortcut_text(keys);
                if ui.add(item(tr(TKey::New), "Ctrl+N")).clicked() {
                    files.request(s, &ctx, Discard::New);
                }
                if ui.add(item(tr(TKey::Open), "Ctrl+O")).clicked() {
                    files.request(s, &ctx, Discard::Open(None));
                }
                ui.add_enabled_ui(!files.recent.is_empty(), |ui| {
                    ui.menu_button(tr(TKey::OpenRecent), |ui| {
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
                        if ui.button(tr(TKey::ClearRecent)).clicked() {
                            files.recent.clear();
                        }
                        if let Some(path) = chosen {
                            files.request(s, &ctx, Discard::Open(Some(path)));
                        }
                    });
                });
                ui.separator();
                if ui.add(item(tr(TKey::Save), "Ctrl+S")).clicked() {
                    files.save(s);
                }
                if ui.add(item(tr(TKey::SaveAs), "Ctrl+Shift+S")).clicked() {
                    files.save_as(s);
                }
                ui.separator();
                ui.menu_button("Export", |ui| {
                    ui.add_enabled_ui(crate::export::garment_mesh(s).is_some(), |ui| {
                        if ui
                            .button("Draped garment as OBJ…")
                            .on_hover_text(
                                "Drape the garment first (3D garment view) to export its mesh.",
                            )
                            .clicked()
                        {
                            if let Some(err) = crate::export::export_obj(s) {
                                files.error = Some(err);
                            }
                        }
                        if ui
                            .button("Draped garment as glTF (.glb)…")
                            .on_hover_text(
                                "Drape the garment first (3D garment view) to export its mesh.",
                            )
                            .clicked()
                        {
                            if let Some(err) = crate::export::export_gltf(s) {
                                files.error = Some(err);
                            }
                        }
                    });
                    ui.add_enabled_ui(!s.doc.patterns.is_empty(), |ui| {
                        if ui.button("Patterns as DXF (AAMA)…").clicked() {
                            if let Some(err) = crate::export::export_dxf(s) {
                                files.error = Some(err);
                            }
                        }
                        if ui
                            .button("Patterns and BOM as PDF…")
                            .on_hover_text(
                                "Traces each piece to scale plus a bill-of-materials summary, at the size picked in Properties.",
                            )
                            .clicked()
                        {
                            if let Some(err) = crate::export::export_pdf(s) {
                                files.error = Some(err);
                            }
                        }
                    });
                });
                ui.separator();
                if ui.add(item(tr(TKey::Quit), "Ctrl+Q")).clicked() {
                    files.request(s, &ctx, Discard::Quit);
                }
            });
            ui.menu_button(tr(TKey::MenuEdit), |ui| {
                let undo = s
                    .history
                    .undo_label()
                    .map_or(tr(TKey::Undo).to_owned(), |l| {
                        format!("{} {l}", tr(TKey::Undo))
                    });
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
                    .map_or(tr(TKey::Redo).to_owned(), |l| {
                        format!("{} {l}", tr(TKey::Redo))
                    });
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
                        egui::Button::new(tr(TKey::Delete)).shortcut_text("Del"),
                    )
                    .clicked()
                {
                    s.delete_selected();
                }
            });
            ui.menu_button(tr(TKey::MenuView), |ui| {
                ui.checkbox(&mut s.canvas.snap.grid, "Snap to grid");
                ui.checkbox(&mut s.canvas.snap.points, "Snap to points");
                ui.separator();
                if ui.button("Reset layout").clicked() {
                    self.dock = default_layout();
                }
                ui.separator();
                egui::widgets::global_theme_preference_buttons(ui);
                ui.separator();
                ui.menu_button(tr(TKey::Language), |ui| {
                    for candidate in Locale::ALL {
                        if ui
                            .selectable_label(self.locale == candidate, candidate.display_name())
                            .clicked()
                        {
                            self.locale = candidate;
                        }
                    }
                });
            });
            ui.menu_button(tr(TKey::MenuHelp), |ui| {
                if ui.button("About OpenDesign").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.session;
        ui.horizontal(|ui| {
            for (tool, label, _, tip) in Tool::BUTTONS {
                ui.selectable_value(&mut s.tool, tool, label)
                    .on_hover_text(tip);
            }
            ui.separator();
            let snap = &mut s.canvas.snap;
            ui.toggle_value(&mut snap.grid, "Grid snap")
                .on_hover_text("Snap drafted points to the grid");
            ui.toggle_value(&mut snap.points, "Point snap")
                .on_hover_text("Snap drafted points to existing points");
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
            for (label, phase) in [("Pin", "Phase 6"), ("▶ Simulate", "Phase 6")] {
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

        // Undo can remove the selected pattern or point.
        self.session.fix_selection();

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
        eframe::set_value(storage, LOCALE_KEY, &self.locale);
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
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

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

    // ---- Localisation (Phase 13) ----

    #[test]
    fn language_picker_switches_menu_labels() {
        let mut h = harness();
        assert_eq!(h.state().locale, Locale::En);

        h.get_by_label("View").click();
        h.run();
        h.get_by_label("Language ⏵").click();
        h.run();
        h.get_by_label("Français").click();
        h.run();
        assert_eq!(h.state().locale, Locale::Fr);

        // Close the (now French) View menu, then the File menu label
        // itself should read "Fichier" — proving the switch reaches the
        // top-level menu bar, not just the picker's own submenu.
        h.run();
        assert!(h.query_by_label("Fichier").is_some());
        assert!(h.query_by_label("File").is_none());

        // Switch back: English is what every other UI test in this
        // module relies on via literal labels like "File"/"Save".
        h.get_by_label("Affichage").click();
        h.run();
        h.get_by_label("Langue ⏵").click();
        h.run();
        h.get_by_label("English").click();
        h.run();
        assert_eq!(h.state().locale, Locale::En);
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
    fn properties_panel_assigns_a_fabric() {
        let mut h = harness();
        h.get_by_label("+ Sleeve").click();
        h.run();
        let id = h.state().session.doc.patterns[0].id;
        assert_eq!(
            h.state().session.doc.pattern(id).unwrap().fabric.name,
            "Cotton"
        );

        click(&mut h, "Denim");
        assert_eq!(
            h.state().session.doc.pattern(id).unwrap().fabric.name,
            "Denim"
        );
        assert_eq!(last_step(&h), "Set fabric: Denim");

        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(
            h.state().session.doc.pattern(id).unwrap().fabric.name,
            "Cotton"
        );
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

    // ---- 2D drafting (Phase 3) ----

    use crate::panels::pattern2d::Snap;
    use egui::accesskit::Role;
    use od_core::PointKind::{Corner, Curve};

    /// Screen position of a document point (mm) in the 2D canvas.
    fn screen(h: &Harness<'_, OpenDesignApp>, x: f32, y: f32) -> egui::Pos2 {
        h.state().session.canvas.screen_pos(Vec2::new(x, y))
    }

    fn mouse(h: &Harness<'_, OpenDesignApp>, pos: egui::Pos2, pressed: bool, mods: Modifiers) {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: mods,
        });
    }

    /// Press and release the primary button at `pos`, holding `mods`.
    fn click_at(h: &mut Harness<'_, OpenDesignApp>, pos: egui::Pos2, mods: Modifiers) {
        h.hover_at(pos);
        h.run();
        h.event(egui::Event::ModifiersChanged(mods));
        for pressed in [true, false] {
            mouse(h, pos, pressed, mods);
            h.run();
        }
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    }

    fn double_click_at(h: &mut Harness<'_, OpenDesignApp>, pos: egui::Pos2) {
        // The harness delivers one pointer event per quarter-second step,
        // slower than a double click, so allow a slower one here. Earlier
        // clicks must first age, or this would count as a triple click.
        let set_delay = |h: &mut Harness<'_, OpenDesignApp>, secs| {
            h.ctx
                .options_mut(|o| o.input_options.max_double_click_delay = secs);
        };
        set_delay(h, 1.0);
        h.run_steps(12);
        h.hover_at(pos);
        h.run();
        for pressed in [true, false, true, false] {
            mouse(h, pos, pressed, Modifiers::NONE);
            h.step();
        }
        h.run();
        set_delay(h, 0.3);
    }

    fn drag(h: &mut Harness<'_, OpenDesignApp>, from: egui::Pos2, to: egui::Pos2) {
        h.drag_at(from);
        h.run();
        for t in [0.25, 0.5, 0.75, 1.0] {
            h.hover_at(from + (to - from) * t);
            h.run();
        }
        h.drop_at(to);
        h.run();
    }

    fn draft(h: &Harness<'_, OpenDesignApp>) -> Vec<Vec2> {
        h.state().session.canvas.draft_points()
    }

    fn last_step(h: &Harness<'_, OpenDesignApp>) -> String {
        h.state().session.history.undo_label().unwrap_or_default()
    }

    fn assert_near(a: Vec2, b: Vec2) {
        assert!(a.distance(b) < 0.05, "{a:?} vs {b:?}");
    }

    #[test]
    fn pen_tool_drafts_a_curved_outline_closed_on_its_first_point() {
        let mut h = harness();
        click(&mut h, "Pen");
        // Slightly off the grid points: grid snapping puts them back.
        for (x, y) in [(0.0, 0.0), (200.0, 0.0), (200.0, 150.0)] {
            let at = screen(&h, x, y) + egui::vec2(4.0, -3.0);
            click_at(&mut h, at, Modifiers::NONE);
        }
        // Shift+click places a curve point.
        let at = screen(&h, 0.0, 150.0);
        click_at(&mut h, at, Modifiers::SHIFT);
        assert_eq!(draft(&h).len(), 4);
        assert_eq!(pattern_count(&h), 0, "not closed yet");

        let first = screen(&h, 0.0, 0.0) + egui::vec2(3.0, 3.0);
        click_at(&mut h, first, Modifiers::NONE);
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 1);
        let outline = &s.doc.patterns[0].outline;
        let kinds: Vec<_> = outline.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [Corner, Corner, Corner, Curve]);
        assert_eq!(outline[1].pos, Vec2::new(200.0, 0.0));
        assert!(s.canvas.draft_points().is_empty());
        // The curve rounds off one corner of the 200 × 150 box.
        let area = s.doc.patterns[0].area();
        assert!(area < 30_000.0 && area > 20_000.0, "area {area}");
        assert_eq!(s.history.done_labels(), ["Add Pattern 1"]);
        h.get_by_label("Selected: Pattern 1");

        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(pattern_count(&h), 0, "the whole outline is one step");
    }

    #[test]
    fn pen_tool_escape_cancels_and_enter_closes() {
        let mut h = harness();
        press(&mut h, Modifiers::NONE, Key::P);
        assert_eq!(h.state().session.tool, Tool::Pen);
        let triangle = [(0.0, 0.0), (300.0, 0.0), (150.0, 200.0)];
        for (x, y) in triangle {
            let at = screen(&h, x, y);
            click_at(&mut h, at, Modifiers::NONE);
        }
        press(&mut h, Modifiers::NONE, Key::Escape);
        assert!(draft(&h).is_empty());
        assert_eq!(pattern_count(&h), 0);

        for (x, y) in triangle {
            let at = screen(&h, x, y);
            click_at(&mut h, at, Modifiers::NONE);
        }
        press(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(pattern_count(&h), 1);
        assert_eq!(h.state().session.doc.patterns[0].outline.len(), 3);
    }

    #[test]
    fn snapping_to_grid_and_points_can_be_switched_off() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        let corner = h.state().session.doc.patterns[0].outline[0].pos;
        click(&mut h, "Pen");

        // Grid: 50 mm at this zoom.
        assert_eq!(h.state().session.canvas.grid_step(), 50.0);
        let at = screen(&h, 50.0, 100.0) + egui::vec2(5.0, -4.0);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(draft(&h)[0], Vec2::new(50.0, 100.0));

        // Points win over the grid: land exactly on the skirt's corner.
        let at = screen(&h, corner.x, corner.y) + egui::vec2(5.0, 5.0);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(draft(&h)[1], corner);

        // Both off: the click lands where it is.
        click(&mut h, "Grid snap");
        click(&mut h, "Point snap");
        assert_eq!(
            h.state().session.canvas.snap,
            Snap {
                grid: false,
                points: false
            }
        );
        let at = screen(&h, 50.0, 300.0) + egui::vec2(5.0, 5.0);
        click_at(&mut h, at, Modifiers::NONE);
        let zoom = h.state().session.canvas.zoom();
        assert_near(
            draft(&h)[2],
            Vec2::new(50.0 + 5.0 / zoom, 300.0 + 5.0 / zoom),
        );

        // The View menu has the same switches.
        click(&mut h, "View");
        click(&mut h, "Snap to grid");
        assert!(h.state().session.canvas.snap.grid);
        press(&mut h, Modifiers::NONE, Key::Escape);

        // Dragged points snap too.
        press(&mut h, Modifiers::NONE, Key::Escape);
        click(&mut h, "Edit points");
        let p2 = h.state().session.doc.patterns[0].outline[2].pos;
        let from = screen(&h, p2.x, p2.y);
        let to = screen(&h, 300.0, 400.0) + egui::vec2(4.0, -3.0);
        drag(&mut h, from, to);
        assert_eq!(
            h.state().session.doc.patterns[0].outline[2].pos,
            Vec2::new(300.0, 400.0)
        );
    }

    #[test]
    fn dragging_a_point_is_one_undo_step() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        press(&mut h, Modifiers::NONE, Key::E);
        h.state_mut().session.canvas.snap = Snap {
            grid: false,
            points: false,
        };
        let before = h.state().session.doc.patterns[0].outline.clone();
        let from = screen(&h, before[2].pos.x, before[2].pos.y);
        drag(&mut h, from, from + egui::vec2(60.0, 30.0));

        let s = &h.state().session;
        let after = &s.doc.patterns[0].outline;
        let zoom = s.canvas.zoom();
        assert_near(
            after[2].pos,
            before[2].pos + Vec2::new(60.0 / zoom, 30.0 / zoom),
        );
        assert_eq!(after[0], before[0], "other points stay put");
        assert_eq!(s.history.done_labels(), ["Add Skirt panel", "Move point"]);
        assert_eq!(s.canvas.selected_point, Some((s.doc.patterns[0].id, 2)));

        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.patterns[0].outline, before);
    }

    #[test]
    fn edit_points_inserts_curves_and_deletes_points() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        click(&mut h, "Edit points");
        let original = h.state().session.doc.patterns[0].outline.clone();
        let id = h.state().session.doc.patterns[0].id;
        // The top edge runs from point 0 to point 1; its middle is x = 0.
        let top = original[0].pos.lerp(original[1].pos, 0.5);

        let at = screen(&h, top.x, top.y);
        double_click_at(&mut h, at);
        let outline = &h.state().session.doc.patterns[0].outline;
        assert_eq!(outline.len(), 5);
        assert_near(outline[1].pos, top);
        assert_eq!(outline[1].kind, Corner);
        assert_eq!(last_step(&h), "Insert point");
        assert_eq!(h.state().session.canvas.selected_point, Some((id, 1)));

        press(&mut h, Modifiers::NONE, Key::C);
        assert_eq!(h.state().session.doc.patterns[0].outline[1].kind, Curve);
        // Double-clicking the point switches it back.
        double_click_at(&mut h, at);
        assert_eq!(h.state().session.doc.patterns[0].outline[1].kind, Corner);
        assert_eq!(last_step(&h), "Change point type");

        press(&mut h, Modifiers::NONE, Key::Delete);
        assert_eq!(
            pattern_count(&h),
            1,
            "Delete removes the point, not the pattern"
        );
        assert_eq!(h.state().session.doc.patterns[0].outline.len(), 4);
        assert_eq!(last_step(&h), "Delete point");

        for _ in 0..4 {
            press(&mut h, Modifiers::COMMAND, Key::Z);
        }
        assert_eq!(h.state().session.doc.patterns[0].outline, original);
    }

    #[test]
    fn library_blocks_are_curved_tshirt_pieces() {
        let mut h = harness();
        for block in ["+ Front bodice", "+ Back bodice", "+ Sleeve"] {
            click(&mut h, block);
        }
        let doc = &h.state().session.doc;
        for p in &doc.patterns {
            assert!(
                p.outline.iter().any(|q| q.kind == Curve),
                "{} has curves",
                p.name
            );
            // Concave necklines and armholes still fill exactly.
            let poly = p.polyline(od_core::document::TOLERANCE);
            let tris = od_core::geometry::triangulate(&poly);
            let covered: f32 = tris
                .iter()
                .map(|t| od_core::geometry::area(&t.map(|i| poly[i as usize])))
                .sum();
            assert!((covered - p.area()).abs() < p.area() * 1e-3, "{}", p.name);
        }
        // The front neckline dips lower than the back's.
        let neck_depth = |p: &od_core::Pattern| {
            let (min, max) = p.bounds().unwrap();
            let centre = (min.x + max.x) * 0.5;
            p.polyline(0.1)
                .iter()
                .filter(|v| (v.x - centre).abs() < 2.0 && v.y < min.y + 200.0)
                .map(|v| v.y - min.y)
                .fold(0.0_f32, f32::max)
        };
        let (front, back) = (&doc.patterns[0], &doc.patterns[1]);
        assert!(neck_depth(front) > 80.0, "front {}", neck_depth(front));
        assert!(neck_depth(back) < 30.0, "back {}", neck_depth(back));
    }

    /// Draft half a T-shirt front by hand, mark it up, mirror it into a
    /// whole front, then save and reload it.
    #[test]
    fn draft_a_tshirt_front_end_to_end() {
        let path = temp_dir("tshirt").join("tshirt.odproj");
        let mut h = harness();

        click(&mut h, "Pen");
        // Left half, centre front on x = 0: (x, y, curve point?).
        let half = [
            (0.0, 100.0, false), // centre front neck
            (-50.0, 100.0, true),
            (-100.0, 0.0, false),   // neck point
            (-200.0, 50.0, false),  // shoulder
            (-200.0, 200.0, true),  // armhole
            (-250.0, 200.0, false), // underarm
            (-250.0, 650.0, false),
            (0.0, 650.0, false),
        ];
        for (x, y, curve) in half {
            let mods = if curve {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            let at = screen(&h, x, y);
            click_at(&mut h, at, mods);
        }
        let at = screen(&h, 0.0, 100.0);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(pattern_count(&h), 1);
        let id = h.state().session.doc.patterns[0].id;

        // A notch halfway down the side seam.
        click(&mut h, "Notch");
        let at = screen(&h, -250.0, 400.0);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(h.state().session.doc.patterns[0].notches.len(), 1);

        // An internal line, e.g. a pocket placement.
        click(&mut h, "Internal line");
        for (x, y) in [(-150.0, 300.0), (-100.0, 450.0)] {
            let at = screen(&h, x, y);
            click_at(&mut h, at, Modifiers::NONE);
        }
        press(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(
            h.state().session.doc.patterns[0].internal_lines[0].points,
            [Vec2::new(-150.0, 300.0), Vec2::new(-100.0, 450.0)]
        );

        // Seam allowance and grainline from Properties: one step per drag.
        let fields: Vec<egui::Pos2> = h
            .get_all_by_role(Role::SpinButton)
            .map(|n| n.rect().center())
            .collect();
        // Seam allowance, grainline, and the new grading-percent field.
        assert_eq!(fields.len(), 3);
        drag(&mut h, fields[0], fields[0] + egui::vec2(40.0, 0.0));
        assert_eq!(last_step(&h), "Set seam allowance");
        let allowance = h.state().session.doc.patterns[0].seam_allowance;
        assert!(allowance > 15.0, "allowance {allowance}");
        drag(&mut h, fields[1], fields[1] + egui::vec2(30.0, 0.0));
        assert_eq!(last_step(&h), "Set grainline");
        assert!(h.state().session.doc.patterns[0].grain_angle > 5.0);

        // Unfold along centre front.
        click(&mut h, "Mirrored copy");
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 2);
        let (left, right) = (&s.doc.patterns[0], &s.doc.patterns[1]);
        assert_eq!(right.name, "Pattern 1 (mirrored)");
        let (min, max) = right.bounds().unwrap();
        assert_near(Vec2::new(min.x, max.x), Vec2::new(0.0, 250.0));
        assert!((left.area() - right.area()).abs() < 1.0);
        assert_eq!(right.notches, left.notches);

        let labels = s.history.done_labels();
        assert_eq!(
            labels,
            [
                "Add Pattern 1",
                "Add notch",
                "Add internal line",
                "Set seam allowance",
                "Set grainline",
                "Duplicate mirrored",
            ]
        );

        // Save, reload, and get exactly the same document.
        h.state_mut().files.path = Some(path.clone());
        press(&mut h, Modifiers::COMMAND, Key::S);
        let loaded = od_core::project::load(&path).unwrap();
        assert_eq!(loaded, h.state().session.doc);
        assert_eq!(loaded.pattern(id).unwrap().outline[1].kind, Curve);
    }

    #[test]
    fn notch_tool_toggles_notches() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        click(&mut h, "Notch");
        let top = {
            let o = &h.state().session.doc.patterns[0].outline;
            o[0].pos.lerp(o[1].pos, 0.5)
        };
        let at = screen(&h, top.x, top.y) + egui::vec2(0.0, 3.0);
        click_at(&mut h, at, Modifiers::NONE);
        let notches = h.state().session.doc.patterns[0].notches.clone();
        assert_eq!(notches.len(), 1);
        // The skirt's outline starts at its top-left corner, 300 mm wide.
        assert!((notches[0].at - 150.0).abs() < 0.5, "at {}", notches[0].at);

        click_at(&mut h, at, Modifiers::NONE);
        assert!(h.state().session.doc.patterns[0].notches.is_empty());
        assert_eq!(last_step(&h), "Remove notch");
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.patterns[0].notches, notches);
    }

    #[test]
    fn trim_tool_places_a_button_and_a_snap_and_removes_them() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        click(&mut h, "Trim");
        let outline = h.state().session.doc.patterns[0].outline.clone();
        let inside = (outline[0].pos + outline[2].pos) * 0.5;

        // A plain click places a button.
        let at = screen(&h, inside.x, inside.y);
        click_at(&mut h, at, Modifiers::NONE);
        let trims = h.state().session.doc.patterns[0].trims.clone();
        assert_eq!(trims.len(), 1);
        assert_eq!(trims[0].kind, od_core::TrimKind::Button);

        // Shift+click elsewhere places a snap.
        let other = screen(&h, inside.x + 20.0, inside.y + 20.0);
        click_at(&mut h, other, Modifiers::SHIFT);
        let trims = h.state().session.doc.patterns[0].trims.clone();
        assert_eq!(trims.len(), 2);
        assert_eq!(trims[1].kind, od_core::TrimKind::Snap);

        // Clicking an existing trim removes it.
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(h.state().session.doc.patterns[0].trims.len(), 1);
        assert_eq!(last_step(&h), "Remove trim");
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.patterns[0].trims.len(), 2);
    }

    #[test]
    fn topstitch_tool_draws_a_line() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        click(&mut h, "Topstitch");
        let outline = h.state().session.doc.patterns[0].outline.clone();
        let inside = (outline[0].pos + outline[2].pos) * 0.5;
        // Far enough apart (default grid snap is 50 mm here) that the two
        // points don't snap onto the same grid cell.
        for (dx, dy) in [(-100.0, 0.0), (100.0, 0.0)] {
            let at = screen(&h, inside.x + dx, inside.y + dy);
            click_at(&mut h, at, Modifiers::NONE);
        }
        press(&mut h, Modifiers::NONE, Key::Enter);
        let lines = &h.state().session.doc.patterns[0].topstitches;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].points.len(), 2);
        assert_eq!(last_step(&h), "Add topstitch");
    }

    #[test]
    fn flips_and_duplicate_undo_exactly() {
        let mut h = harness();
        click(&mut h, "+ Sleeve");
        let before = h.state().session.doc.clone();
        click(&mut h, "Flip left-right");
        click(&mut h, "Flip up-down");
        let flipped = &h.state().session.doc.patterns[0];
        assert_ne!(flipped.outline, before.patterns[0].outline);
        assert_eq!(flipped.bounds(), before.patterns[0].bounds());
        press(&mut h, Modifiers::COMMAND, Key::D);
        assert_eq!(pattern_count(&h), 2);
        h.get_by_label("Selected: Sleeve (copy)");
        for _ in 0..3 {
            press(&mut h, Modifiers::COMMAND, Key::Z);
        }
        assert_eq!(h.state().session.doc.patterns, before.patterns);
    }

    // ---- Sewing (Phase 5) ----

    use od_core::{SeamEdge, SeamPoint};

    /// Midpoint of outline edge `i -> i + 1` of `patterns[index]`, in doc
    /// space.
    fn edge_mid(h: &Harness<'_, OpenDesignApp>, index: usize, i: usize) -> Vec2 {
        let o = &h.state().session.doc.patterns[index].outline;
        o[i].pos.lerp(o[(i + 1) % o.len()].pos, 0.5)
    }

    #[test]
    fn sew_tool_builds_and_unsews_a_two_edge_seam() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        click(&mut h, "+ Skirt panel");
        let (id0, id1) = (
            h.state().session.doc.patterns[0].id,
            h.state().session.doc.patterns[1].id,
        );
        click(&mut h, "Sew");

        // Pick the top edge (index 0 -> 1) of each panel for the two sides.
        let top0 = edge_mid(&h, 0, 0);
        let at = screen(&h, top0.x, top0.y);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(
            h.state().session.canvas.sew_pending_edges(),
            Some([
                vec![SeamEdge {
                    pattern: id0,
                    start: 0.0,
                    end: 1.0
                }],
                vec![]
            ])
        );

        press(&mut h, Modifiers::NONE, Key::Tab);
        let top1 = edge_mid(&h, 1, 0);
        let at = screen(&h, top1.x, top1.y);
        click_at(&mut h, at, Modifiers::NONE);
        press(&mut h, Modifiers::NONE, Key::Enter);

        let doc = h.state().session.doc.clone();
        assert_eq!(doc.sewing_lines.len(), 1);
        assert_eq!(
            doc.sewing_lines[0].a,
            vec![SeamEdge {
                pattern: id0,
                start: 0.0,
                end: 1.0
            }]
        );
        assert_eq!(
            doc.sewing_lines[0].b,
            vec![SeamEdge {
                pattern: id1,
                start: 0.0,
                end: 1.0
            }]
        );
        assert_eq!(last_step(&h), "Sew edges");
        assert_eq!(h.state().session.canvas.sew_pending_edges(), None);

        // Clicking a sewn edge again, with nothing pending, unsews it.
        let at = screen(&h, top0.x, top0.y);
        click_at(&mut h, at, Modifiers::NONE);
        assert!(h.state().session.doc.sewing_lines.is_empty());
        assert_eq!(last_step(&h), "Unsew edges");

        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.sewing_lines, doc.sewing_lines);
    }

    #[test]
    fn sew_tool_drag_picks_a_free_sub_range() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        let id = h.state().session.doc.patterns[0].id;
        click(&mut h, "Sew");

        // A quarter to three-quarters of the way along the 300 mm top edge.
        let (o0, o1) = {
            let o = &h.state().session.doc.patterns[0].outline;
            (o[0].pos, o[1].pos)
        };
        let (from, to) = (o0.lerp(o1, 0.25), o0.lerp(o1, 0.75));
        let (from_px, to_px) = (screen(&h, from.x, from.y), screen(&h, to.x, to.y));
        drag(&mut h, from_px, to_px);

        let sides = h.state().session.canvas.sew_pending_edges();
        let Some([side0, _]) = sides else {
            panic!("expected a pending edge after dragging, got {sides:?}");
        };
        assert_eq!(side0.len(), 1);
        let edge = side0[0];
        assert_eq!(edge.pattern, id);
        // Free sewing: not the whole 0..1 corner span.
        assert!((edge.start - 0.25).abs() < 0.05, "start {}", edge.start);
        assert!((edge.end - 0.75).abs() < 0.05, "end {}", edge.end);
        let length = h.state().session.doc.pattern(id).unwrap().segment_length(
            edge.start,
            edge.end,
            od_core::document::TOLERANCE,
        );
        // Half of the 300 mm edge.
        assert!((length - 150.0).abs() < 5.0, "length {length}");
    }

    #[test]
    fn sew_tool_builds_a_one_to_many_seam() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        click(&mut h, "+ Skirt panel");
        let id1 = h.state().session.doc.patterns[1].id;
        click(&mut h, "Sew");

        let top0 = edge_mid(&h, 0, 0);
        let at = screen(&h, top0.x, top0.y);
        click_at(&mut h, at, Modifiers::NONE);
        press(&mut h, Modifiers::NONE, Key::Tab);
        let top1 = edge_mid(&h, 1, 0);
        let at = screen(&h, top1.x, top1.y);
        click_at(&mut h, at, Modifiers::NONE);
        let bottom1 = edge_mid(&h, 1, 2);
        let at = screen(&h, bottom1.x, bottom1.y);
        click_at(&mut h, at, Modifiers::NONE);
        press(&mut h, Modifiers::NONE, Key::Enter);

        let doc = h.state().session.doc.clone();
        assert_eq!(doc.sewing_lines.len(), 1);
        let seam = &doc.sewing_lines[0];
        assert_eq!(seam.a.len(), 1);
        assert_eq!(seam.b.len(), 2);
        assert!(seam.b.iter().all(|e| e.pattern == id1));
        assert_ne!(seam.b[0].start, seam.b[1].start);
    }

    #[test]
    fn sew_tool_tacks_two_points_and_toggles() {
        let mut h = harness();
        click(&mut h, "+ Skirt panel");
        let id = h.state().session.doc.patterns[0].id;
        click(&mut h, "Sew");

        let (p0, p2) = {
            let o = &h.state().session.doc.patterns[0].outline;
            (o[0].pos, o[2].pos)
        };
        let at = screen(&h, p0.x, p0.y);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(
            h.state().session.canvas.sew_pending_point(),
            Some(SeamPoint {
                pattern: id,
                index: 0
            })
        );
        let at = screen(&h, p2.x, p2.y);
        click_at(&mut h, at, Modifiers::NONE);

        let doc = h.state().session.doc.clone();
        assert_eq!(doc.tacks.len(), 1);
        assert_eq!(
            doc.tacks[0].a,
            SeamPoint {
                pattern: id,
                index: 0
            }
        );
        assert_eq!(
            doc.tacks[0].b,
            SeamPoint {
                pattern: id,
                index: 2
            }
        );
        assert_eq!(last_step(&h), "Add tack");

        // Clicking a tacked point again, with nothing pending, untacks it.
        let at = screen(&h, p0.x, p0.y);
        click_at(&mut h, at, Modifiers::NONE);
        assert!(h.state().session.doc.tacks.is_empty());
        assert_eq!(last_step(&h), "Remove tack");

        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.tacks, doc.tacks);
    }

    // ---- Library: asset browser (Phase 8) ----

    #[test]
    fn library_lists_avatars_fabrics_and_trims() {
        let mut h = harness();
        // Avatars (also covered by viewport3d's own avatar-switching tests).
        for label in ["Female", "Male", "Kid"] {
            h.get_by_label(label);
        }
        // Fabrics: a swatch legend; assignment itself lives in Properties
        // (see `properties_panel_assigns_a_fabric`), so there's no
        // per-fabric widget here to avoid a second control for one action.
        h.get_by_label("Select a pattern to assign a fabric.");
        // Trims remain a placeholder past this phase, collapsed by default.
        click(&mut h, "Trims");
        assert!(h.get_by_label("Buttons").accesskit_node().is_disabled());
    }

    #[test]
    fn picking_a_fabric_assigns_it_to_the_selected_pattern() {
        let mut h = harness();
        click(&mut h, "+ Front bodice");
        let id = h.state().session.doc.patterns[0].id;
        assert_eq!(
            h.state().session.doc.pattern(id).unwrap().fabric,
            od_core::Fabric::default()
        );

        click(&mut h, "Denim");
        assert_eq!(
            h.state().session.doc.pattern(id).unwrap().fabric,
            od_core::Fabric::denim()
        );

        // Undo restores the previous (default) fabric.
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(
            h.state().session.doc.pattern(id).unwrap().fabric,
            od_core::Fabric::default()
        );
    }

    #[test]
    fn switching_the_selected_pattern_shows_its_own_fabric() {
        let mut h = harness();
        click(&mut h, "+ Front bodice");
        let front = h.state().session.doc.patterns[0].id;
        click(&mut h, "Silk");

        click(&mut h, "+ Back bodice");
        let back = h.state().session.doc.patterns[1].id;
        assert_ne!(front, back);
        assert_eq!(
            h.state().session.doc.pattern(back).unwrap().fabric,
            od_core::Fabric::default()
        );

        // Re-selecting the first pattern (2D canvas) shows it still has
        // its fabric, unaffected by the second pattern's selection.
        h.state_mut().session.selection = Some(front);
        h.run();
        assert_eq!(
            h.state().session.doc.pattern(front).unwrap().fabric,
            od_core::Fabric::silk()
        );
    }
}
