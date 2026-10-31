//! Application shell: menu bar, toolbar, status bar and the docked workspace.

use egui::{Key, KeyboardShortcut, Modifiers};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use od_core::{
    AddPattern, AddTack, Axis, Batch, Clip, Command, DeletePattern, DeletePoint, Document,
    DuplicatePattern, History, Link, LinkedCopy, MirrorPattern, OutlinePoint, Paste, PatternFlag,
    PatternId, RemoveSewingLine, RemoveTack, RotatePattern, SetPatternFlag, SetPointKind,
    SewingLine, SewingLineId, TackId, TackPoint, TackTarget, Vec2,
};

use crate::files::{Discard, FileState, RECENT_KEY};
use crate::i18n::{self, Key as TKey, Locale};
use crate::panels::{
    self, pattern2d::Canvas2D, properties::PropertiesState, viewport3d::Viewport3D,
};
use crate::recovery::AutoSave;
use crate::selection::Selection;
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Viewport3D,
    Pattern2D,
    Library,
    Objects,
    Properties,
    History,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Viewport3D,
        Tab::Pattern2D,
        Tab::Library,
        Tab::Objects,
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
    Tack,
    Dart,
    Pleat,
    Measure,
}

impl Tool {
    /// Toolbar order, with button label, shortcut key and tooltip.
    const BUTTONS: [(Tool, &'static str, Key, &'static str); 13] = [
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
        (
            Tool::Tack,
            "Tack",
            Key::K,
            "Hold a point on a pattern to another pattern, or to the avatar in the 3D view (K)",
        ),
        (
            Tool::Dart,
            "Dart",
            Key::D,
            "Click an outline to cut a dart into it; drag inside a pattern for a diamond dart (D)",
        ),
        (
            Tool::Pleat,
            "Pleat",
            Key::Q,
            "Drag along a pattern where the first pleat folds; the rest step to the right (Q)",
        ),
        (
            Tool::Measure,
            "Measure",
            Key::M,
            "Click two outline points to measure between them; Shift+click to measure along the outline (M)",
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
            Tool::Tack => "Tack",
            Tool::Dart => "Dart",
            Tool::Pleat => "Pleat",
            Tool::Measure => "Measure",
        }
    }
}

/// Check sewing length: flag seams whose sides differ by more than a
/// percentage of the shorter one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeamCheck {
    pub enabled: bool,
    pub percent: f32,
}

impl Default for SeamCheck {
    fn default() -> Self {
        Self {
            enabled: true,
            percent: od_core::document::LENGTH_MISMATCH_TOLERANCE * 100.0,
        }
    }
}

impl SeamCheck {
    /// Should `line` be flagged?
    pub fn flags(&self, doc: &Document, line: &SewingLine) -> bool {
        self.enabled && line.is_mismatched_by(doc, self.percent / 100.0)
    }

    /// Seams to flag, in document order.
    pub fn flagged<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = &'a SewingLine> + 'a {
        doc.sewing_lines
            .iter()
            .filter(move |line| self.flags(doc, line))
    }
}

/// Everything the panels can see and change. Kept apart from the dock
/// layout so it can be lent to the tab viewer while the dock is drawn.
pub struct Session {
    pub doc: Document,
    pub history: History,
    pub selection: Selection,
    /// Patterns copied with Ctrl+C or Ctrl+X, and how many times they've
    /// been pasted since, so each paste lands a step further along.
    pub clipboard: Option<(Clip, u32)>,
    /// A selected seam or tack. Selecting one clears the others.
    pub selected_seam: Option<SewingLineId>,
    pub selected_tack: Option<TackId>,
    /// Where a tack being placed starts, in the 2D or 3D view.
    pub tack_start: Option<TackPoint>,
    pub seam_check: SeamCheck,
    pub tool: Tool,
    pub canvas: Canvas2D,
    pub viewport: Viewport3D,
    pub props: PropertiesState,
    /// Index into `doc.size_chart.names` the 2D canvas, properties and BOM
    /// previews currently show. Not persisted: a document always opens
    /// showing its base size.
    pub preview_size: usize,
    /// The pattern under the pointer in either view, highlighted in both.
    /// Set from what the views saw last frame (`hover_next`).
    pub hovered: Option<PatternId>,
    pub hover_next: Option<PatternId>,
    /// Fabric files saved or loaded, listed in the Library.
    pub fabrics: crate::fabric_shelf::FabricShelf,
    /// Seam allowance and particle distance new work gets (Preferences).
    pub defaults: crate::prefs::Defaults,
}

/// How far each paste lands from the last, in mm on both axes.
const PASTE_OFFSET: f32 = 40.0;

impl Session {
    pub fn execute(&mut self, command: impl Command + 'static) {
        self.history.execute(&mut self.doc, Box::new(command));
    }

    /// Add a straight-edged pattern and select it.
    pub fn add_pattern(&mut self, name: impl Into<String>, outline: Vec<Vec2>) {
        self.add_outline(
            name,
            outline.into_iter().map(OutlinePoint::corner).collect(),
        );
    }

    /// Add a pattern with corner and curve points and select it.
    /// It gets the seam allowance set in Preferences.
    pub fn add_outline(&mut self, name: impl Into<String>, outline: Vec<OutlinePoint>) {
        let mut pattern = od_core::Pattern::new(PatternId(0), name, outline);
        pattern.seam_allowance = self.defaults.seam_allowance;
        self.execute(AddPattern::from_pattern(pattern));
        self.selection.set(self.doc.patterns.last().map(|p| p.id));
    }

    pub fn undo(&mut self) {
        self.history.undo(&mut self.doc);
    }

    pub fn redo(&mut self) {
        self.history.redo(&mut self.doc);
    }

    /// The Tack tool picked `target`: start a tack there, or finish the
    /// one already started. Tacks start on a pattern, not the avatar.
    pub fn tack_to(&mut self, target: TackTarget) {
        match (self.tack_start.take(), target) {
            (None, TackTarget::Pattern(start)) => self.tack_start = Some(start),
            (None, _) => {}
            (Some(start), target) => {
                self.execute(AddTack::new(start, target));
                // A new tack goes last; select it so it shows in Properties.
                self.selected_tack = self.doc.tacks.last().map(|t| t.id);
                self.selected_seam = None;
                self.selection.set(None);
            }
        }
    }

    /// Delete the selected point while editing points, else the selected
    /// tack, seam or pattern.
    pub fn delete_selected(&mut self) {
        if let Some((id, index)) = self.canvas.selected_point.take() {
            // An outline needs three points; keep the point selected if so.
            if self.doc.pattern(id).is_some_and(|p| p.outline.len() > 3) {
                let (target, _) = self.edit_target(id);
                self.execute(DeletePoint::new(target, index));
            } else {
                self.canvas.selected_point = Some((id, index));
            }
            return;
        }
        if let Some(id) = self.selected_tack.take() {
            self.execute(RemoveTack::new(id));
            return;
        }
        if let Some(id) = self.selected_seam.take() {
            self.execute(RemoveSewingLine::new(id));
            return;
        }
        // Locked patterns stay, and stay selected.
        let ids = self.unlocked_selection();
        self.selection.set_all(
            self.selection
                .ids()
                .to_vec()
                .into_iter()
                .filter(|id| !ids.contains(id)),
        );
        self.for_each(ids, "Delete", "patterns", |id| {
            Box::new(DeletePattern::new(id))
        });
    }

    /// Where an edit to pattern `id`'s shape or markings goes: to the
    /// source of a linked copy (with the link, to map positions back with
    /// [`Link::to_source`]), else to `id` itself. Outline indices are the
    /// same on both.
    pub fn edit_target(&self, id: PatternId) -> (PatternId, Option<Link>) {
        match self.doc.pattern(id).and_then(|p| p.link) {
            Some(link) if self.doc.pattern(link.source).is_some() => (link.source, Some(link)),
            _ => (id, None),
        }
    }

    /// Add a linked copy of each selected pattern, mirrored beside it, and
    /// select the copies.
    pub fn linked_copy_selected(&mut self) {
        let ids = self.selection.ids().to_vec();
        let count = self.doc.patterns.len();
        self.for_each(ids, "Linked copy of", "patterns", |id| {
            Box::new(LinkedCopy::new(id, Some(Axis::Vertical)))
        });
        let copies: Vec<_> = self.doc.patterns[count..].iter().map(|p| p.id).collect();
        if !copies.is_empty() {
            self.selection.set_all(copies);
        }
    }

    /// The selected patterns that aren't locked: the ones edits act on.
    pub fn unlocked_selection(&self) -> Vec<PatternId> {
        self.selection
            .ids()
            .iter()
            .copied()
            .filter(|&id| self.doc.pattern(id).is_some_and(|p| !p.locked))
            .collect()
    }

    /// Hide or show, lock or unlock `ids`, as one undo step. Hidden
    /// patterns leave the selection.
    pub fn set_flag(&mut self, ids: Vec<PatternId>, flag: PatternFlag, value: bool) {
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|&id| self.doc.pattern(id).is_some_and(|p| flag.get(p) != value))
            .collect();
        let verb = match (flag, value) {
            (PatternFlag::Hidden, true) => "Hide",
            (PatternFlag::Hidden, false) => "Show",
            (PatternFlag::Locked, true) => "Lock",
            (PatternFlag::Locked, false) => "Unlock",
        };
        self.for_each(ids, verb, "patterns", |id| {
            Box::new(SetPatternFlag::new(id, flag, value))
        });
        self.fix_selection();
    }

    /// Run `command` on each of `ids` as one undo step: on its own for one
    /// pattern, else in a [`Batch`] labelled like "Delete 3 patterns".
    pub fn for_each(
        &mut self,
        ids: Vec<PatternId>,
        verb: &str,
        noun: &str,
        mut command: impl FnMut(PatternId) -> Box<dyn Command>,
    ) {
        let mut commands: Vec<_> = ids.into_iter().map(&mut command).collect();
        match commands.len() {
            0 => {}
            1 => self.history.execute(&mut self.doc, commands.remove(0)),
            n => self.execute(Batch::new(format!("{verb} {n} {noun}"), commands)),
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
            let (target, _) = self.edit_target(id);
            self.execute(SetPointKind::new(target, index, kind.toggled()));
        }
    }

    /// Flip each selected pattern in place.
    pub fn mirror_selected(&mut self, axis: Axis) {
        let ids = self.unlocked_selection();
        self.for_each(ids, "Flip", "patterns", |id| {
            Box::new(MirrorPattern::new(id, axis))
        });
    }

    /// Turn each selected, unlocked pattern `degrees` clockwise about its
    /// own centre, as one undo step.
    pub fn rotate_selected(&mut self, degrees: f32) {
        let ids = self.unlocked_selection();
        self.for_each(ids, "Rotate", "patterns", |id| {
            Box::new(RotatePattern::new(id, degrees))
        });
    }

    /// Line up the selected patterns (see [`od_core::Align`]) as one undo
    /// step. Locked ones count but stay put.
    pub fn align_selected(&mut self, how: od_core::Align) {
        if let Some(batch) = od_core::align::align(&self.doc, self.selection.ids(), how) {
            self.execute(batch);
        }
    }

    /// Copy the selected patterns, optionally mirrored, and select the
    /// copies.
    pub fn duplicate_selected(&mut self, mirror: Option<Axis>) {
        let ids = self.selection.ids().to_vec();
        let count = self.doc.patterns.len();
        self.for_each(ids, "Duplicate", "patterns", |id| {
            Box::new(DuplicatePattern::new(id, mirror))
        });
        // Copies go last, in the order their sources were selected.
        let copies: Vec<_> = self.doc.patterns[count..].iter().map(|p| p.id).collect();
        if !copies.is_empty() {
            self.selection.set_all(copies);
        }
    }

    /// Copy the selected patterns, with the seams and tacks between them.
    pub fn copy_selected(&mut self) {
        if let Some(clip) = Clip::copy(&self.doc, self.selection.ids()) {
            self.clipboard = Some((clip, 0));
        }
    }

    /// Copy the selected patterns, then delete them.
    pub fn cut_selected(&mut self) {
        let ids = self.unlocked_selection();
        if let Some(clip) = Clip::copy(&self.doc, &ids) {
            self.clipboard = Some((clip, 0));
            self.selection.clear();
            self.for_each(ids, "Cut", "patterns", |id| {
                Box::new(DeletePattern::new(id))
            });
        }
    }

    /// Paste the copied patterns, offset a step further each time, or
    /// mirrored beside the originals, and select them.
    pub fn paste(&mut self, mirror: Option<Axis>) {
        let Some((clip, pastes)) = &mut self.clipboard else {
            return;
        };
        *pastes += 1;
        let step = PASTE_OFFSET * *pastes as f32;
        let paste = Paste::new(clip.clone(), Vec2::new(step, step), mirror);
        // Pasted patterns go last.
        let count = self.doc.patterns.len();
        self.execute(paste);
        let ids: Vec<_> = self.doc.patterns[count..].iter().map(|p| p.id).collect();
        self.selection.set_all(ids);
        self.selected_seam = None;
        self.selected_tack = None;
    }

    /// Fit the selected patterns in both views.
    pub fn frame_selection(&mut self) {
        if self.selection.primary().is_some() {
            self.canvas.frame_selection = true;
            self.viewport.frame_selection = true;
        }
    }

    /// Select every shown pattern.
    pub fn select_all(&mut self) {
        let shown = self.doc.patterns.iter().filter(|p| !p.hidden);
        self.selection.set_all(shown.map(|p| p.id));
        self.selected_seam = None;
        self.selected_tack = None;
    }

    /// Drop selections that undo or redo made stale.
    pub fn fix_selection(&mut self) {
        self.selection.retain_existing(&self.doc);
        let hidden: Vec<_> = self
            .selection
            .ids()
            .iter()
            .copied()
            .filter(|&id| self.doc.pattern(id).is_some_and(|p| p.hidden))
            .collect();
        for id in hidden {
            self.selection.toggle(id);
        }
        if self
            .selected_seam
            .is_some_and(|id| self.doc.sewing_line(id).is_none())
        {
            self.selected_seam = None;
        }
        if self
            .selected_tack
            .is_some_and(|id| self.doc.tack(id).is_none())
        {
            self.selected_tack = None;
        }
        if self
            .tack_start
            .is_some_and(|t| self.doc.pattern(t.pattern).is_none())
        {
            self.tack_start = None;
        }
        if let Some((id, index)) = self.canvas.selected_point {
            let valid = self.selection.primary() == Some(id)
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
        self.selection.set(None);
        self.selected_seam = None;
        self.selected_tack = None;
        self.tack_start = None;
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
    show_prefs: bool,
    locale: Locale,
    autosave: AutoSave,
}

impl OpenDesignApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self {
            dock: restore_layout(cc.storage),
            session: Session {
                doc: Document::default(),
                history: History::new(),
                selection: Default::default(),
                clipboard: None,
                selected_seam: None,
                selected_tack: None,
                tack_start: None,
                seam_check: Default::default(),
                tool: Tool::Select,
                canvas: Canvas2D::default(),
                viewport: Viewport3D::new(cc.wgpu_render_state.as_ref()),
                props: PropertiesState::default(),
                preview_size: Document::default().size_chart.base,
                hovered: None,
                hover_next: None,
                fabrics: crate::fabric_shelf::FabricShelf::restore(cc.storage),
                defaults: Default::default(),
            },
            files: FileState::with_recent(
                cc.storage
                    .and_then(|s| eframe::get_value(s, RECENT_KEY))
                    .unwrap_or_default(),
            ),
            show_about: false,
            show_prefs: false,
            locale: cc
                .storage
                .and_then(|s| eframe::get_value(s, LOCALE_KEY))
                .unwrap_or_default(),
            autosave: {
                let mut autosave = AutoSave::new(AutoSave::default_dir());
                autosave.check();
                autosave
            },
        };
        crate::prefs::restore(cc.storage).apply(&mut app.session);
        app
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
        // Text fields have their own select all, cut, copy and paste.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing {
            if shortcut(Modifiers::COMMAND, Key::A) {
                s.select_all();
            } else if shortcut(Modifiers::COMMAND, Key::C) {
                s.copy_selected();
            } else if shortcut(Modifiers::COMMAND, Key::X) {
                s.cut_selected();
            } else if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::V) {
                s.paste(Some(Axis::Vertical));
            } else if shortcut(Modifiers::COMMAND, Key::V) {
                s.paste(None);
            }
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
        if ctx.input(|i| i.key_pressed(Key::F)) {
            s.frame_selection();
        }
        // Esc while selecting clears the selection; other tools use Esc to
        // cancel what they're drawing.
        if s.tool == Tool::Select && ctx.input(|i| i.key_pressed(Key::Escape)) {
            s.selection.clear();
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
                ui.menu_button("Import", |ui| {
                    if ui
                        .button("Patterns from DXF (AAMA)…")
                        .on_hover_text("Add the pieces in a DXF file to this document")
                        .clicked()
                    {
                        if let Some(err) = crate::import::import_dxf_dialog(s) {
                            files.error = Some(err);
                        }
                    }
                    if ui
                        .button("Garment model for reference (OBJ, glTF)…")
                        .on_hover_text(
                            "Show a finished garment see-through in the 3D view, to compare the drape with. It isn't simulated or saved.",
                        )
                        .clicked()
                    {
                        if let Some(err) = crate::import::import_reference_dialog(s) {
                            files.error = Some(err);
                        }
                    }
                    crate::import::reference_menu(ui, s);
                });
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
                    ui.separator();
                    let dark = ui.visuals().dark_mode;
                    ui.add_enabled_ui(s.viewport.has_gpu(), |ui| {
                        if ui
                            .button("Turntable frames…")
                            .on_hover_text(
                                "Render a full rotation around the current scene as numbered \
                                 PNGs in a folder you choose. Stitch them into a video \
                                 externally, e.g. ffmpeg -framerate 24 -i turntable_%03d.png \
                                 turntable.mp4",
                            )
                            .clicked()
                        {
                            if let Some(err) = crate::export::export_turntable(s, dark) {
                                files.error = Some(err);
                            }
                        }
                        ui.add_enabled_ui(!s.doc.patterns.is_empty(), |ui| {
                            if ui
                                .button("Drape animation frames…")
                                .on_hover_text(
                                    "Simulate the drape from scratch and render each step as \
                                     numbered PNGs in a folder you choose.",
                                )
                                .clicked()
                            {
                                if let Some(err) = crate::export::export_drape_animation(s, dark) {
                                    files.error = Some(err);
                                }
                            }
                        });
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
                let any = s.selection.primary().is_some();
                let item = |label, keys| egui::Button::new(label).shortcut_text(keys);
                if ui.add_enabled(any, item("Cut", "Ctrl+X")).clicked() {
                    s.cut_selected();
                }
                if ui.add_enabled(any, item("Copy", "Ctrl+C")).clicked() {
                    s.copy_selected();
                }
                let pasting = s.clipboard.is_some();
                if ui.add_enabled(pasting, item("Paste", "Ctrl+V")).clicked() {
                    s.paste(None);
                }
                if ui
                    .add_enabled(pasting, item("Paste mirrored", "Ctrl+Shift+V"))
                    .on_hover_text("Paste a mirror image beside the copied patterns")
                    .clicked()
                {
                    s.paste(Some(Axis::Vertical));
                }
                if ui
                    .add_enabled(!s.doc.patterns.is_empty(), item("Select all", "Ctrl+A"))
                    .clicked()
                {
                    s.select_all();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        s.selection.primary().is_some()
                            || s.selected_seam.is_some()
                            || s.selected_tack.is_some(),
                        egui::Button::new(tr(TKey::Delete)).shortcut_text("Del"),
                    )
                    .clicked()
                {
                    s.delete_selected();
                }
                ui.separator();
                if ui
                    .button("Preferences…")
                    .on_hover_text("Grid, snapping, defaults for new pieces, sewing check, theme")
                    .clicked()
                {
                    self.show_prefs = true;
                }
            });
            ui.menu_button(tr(TKey::MenuView), |ui| {
                ui.checkbox(&mut s.canvas.snap.grid, "Snap to grid");
                ui.checkbox(&mut s.canvas.snap.points, "Snap to points");
                ui.separator();
                ui.checkbox(&mut s.seam_check.enabled, "Check sewing length")
                    .on_hover_text("Show seams whose two sides differ in length in the warning colour");
                ui.add_enabled_ui(s.seam_check.enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Tolerance");
                        ui.add(
                            egui::DragValue::new(&mut s.seam_check.percent)
                                .range(0.0..=50.0)
                                .speed(0.1)
                                .suffix(" %"),
                        );
                    });
                });
                ui.separator();
                ui.checkbox(&mut s.canvas.measure.all_sizes, "Show all graded sizes")
                    .on_hover_text("Draw every graded size faintly around the size being previewed");
                if ui.button("Measurement chart").clicked() {
                    s.canvas.measure.chart_open = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Reset layout").clicked() {
                    self.dock = default_layout();
                }
                ui.menu_button("Workspace", |ui| {
                    for mode in crate::layouts::TaskMode::ALL {
                        if ui.button(mode.label()).on_hover_text(mode.tip()).clicked() {
                            self.dock = mode.layout();
                        }
                    }
                });
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let (s, files) = (&mut self.session, &self.files);
        ui.horizontal(|ui| {
            ui.label(format!("Tool: {}", s.tool.name()));
            ui.separator();
            let pattern = s.selection.primary().and_then(|id| s.doc.pattern(id));
            let selected = match (pattern, s.selected_seam, s.selected_tack) {
                (Some(p), _, _) => format!("Selected: {}", p.name),
                (None, Some(_), _) => "Selected: seam".to_owned(),
                (None, None, Some(_)) => "Selected: tack".to_owned(),
                (None, None, None) => "Nothing selected".to_owned(),
            };
            ui.label(selected);
            ui.separator();
            ui.label(format!("{} patterns", s.doc.patterns.len()));
            ui.separator();
            ui.label(format!("{} seams", s.doc.sewing_lines.len()));
            ui.separator();
            ui.label(format!("{} tacks", s.doc.tacks.len()));
            let flagged: Vec<SewingLineId> = s.seam_check.flagged(&s.doc).map(|l| l.id).collect();
            if let Some(&first) = flagged.first() {
                ui.separator();
                let text = match flagged.len() {
                    1 => "1 seam differs in length".to_owned(),
                    n => format!("{n} seams differ in length"),
                };
                let pal = theme::palette(ui.visuals().dark_mode);
                if ui
                    .link(egui::RichText::new(text).color(pal.warning))
                    .on_hover_text("Select the first one")
                    .clicked()
                {
                    s.selected_seam = Some(first);
                    s.selected_tack = None;
                    s.selection.set(None);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match &files.path {
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
        // A close that goes through is a normal quit: nothing unsaved is
        // kept for recovery.
        let closing = ctx.input(|i| i.viewport().close_requested());
        if closing && (self.files.quitting || !self.session.history.is_dirty()) {
            self.autosave.quit();
        }
        // While a dialog is up, the document must not change underneath it.
        if !self.files.is_blocking() && self.autosave.offer.is_none() {
            self.files.handle_dropped(&mut self.session, &ctx);
            if let Some(path) = crate::import::dropped_dxf(&ctx) {
                if let Err(err) = crate::import::import_dxf(&mut self.session, &path) {
                    self.files.error = Some(err);
                }
            }
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
        // What the views found under the pointer, highlighted next frame.
        let s = &mut self.session;
        if s.hovered != s.hover_next {
            ctx.request_repaint();
        }
        s.hovered = s.hover_next.take();

        egui::Window::new("About OpenDesign")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(&ctx, |ui| {
                ui.label("OpenDesign: open-source 3D garment design.");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            });

        egui::Window::new("Preferences")
            .open(&mut self.show_prefs)
            .default_pos(egui::pos2(ctx.content_rect().center().x - 170.0, 120.0))
            .collapsible(false)
            .resizable(false)
            .show(&ctx, |ui| crate::prefs::ui(ui, &mut self.session));

        panels::measure2d::chart_window(&ctx, &mut self.session);
        self.files.dialogs(&mut self.session, &ctx);
        self.autosave
            .dialog(&mut self.session, &mut self.files, &ctx);
        let now = ctx.input(|i| i.time);
        self.autosave.tick(&self.session, &self.files, &ctx, now);
        self.files.update_title(&self.session, &ctx);
    }

    // eframe also saves egui's memory, which holds the theme preference.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_KEY, &self.dock);
        eframe::set_value(storage, RECENT_KEY, &self.files.recent);
        eframe::set_value(storage, LOCALE_KEY, &self.locale);
        let fabrics = self.session.fabrics.paths();
        eframe::set_value(storage, crate::fabric_shelf::FABRICS_KEY, &fabrics);
        let prefs = crate::prefs::Preferences::capture(&self.session);
        eframe::set_value(storage, crate::prefs::PREFS_KEY, &prefs);
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
            Tab::Objects => "Objects",
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
            Tab::Objects => panels::objects::ui(ui, self),
            Tab::Properties => panels::properties::ui(ui, self),
            Tab::History => panels::history::ui(ui, self),
        }
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        // Canvases handle their own panning.
        let side_panel = matches!(
            tab,
            Tab::Library | Tab::Objects | Tab::Properties | Tab::History
        );
        [false, side_panel]
    }
}

/// Library on the left, 3D and 2D side by side, Properties/History on the right.
fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Viewport3D]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.8, vec![Tab::Properties, Tab::History]);
    // Fractions size the left/top child, which for split_left is the new node.
    let [center, _] = tree.split_left(center, 0.2, vec![Tab::Library, Tab::Objects]);
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

#[cfg(test)]
#[path = "app_workspace_tests.rs"]
mod workspace_tests;

#[cfg(test)]
#[path = "measure_tests.rs"]
mod measure_tests;

#[cfg(test)]
#[path = "pose_tests.rs"]
mod pose_tests;

/// Drive the real app headlessly: clicks and keys go through egui exactly as
/// they would from a mouse and keyboard, no window or GPU needed.
#[cfg(test)]
mod fabric_tests;

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
        // Seam allowance, grainline, particle distance and grading percent.
        assert_eq!(fields.len(), 4);
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

    use od_core::{SeamEdge, TackPoint, TackTarget};

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
                    end: 1.0,
                    forward: true,
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
                end: 1.0,
                forward: true,
            }]
        );
        assert_eq!(
            doc.sewing_lines[0].b,
            vec![SeamEdge {
                pattern: id1,
                start: 0.0,
                end: 1.0,
                forward: true,
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
            Some(TackPoint {
                pattern: id,
                at: p0
            })
        );
        let at = screen(&h, p2.x, p2.y);
        click_at(&mut h, at, Modifiers::NONE);

        let doc = h.state().session.doc.clone();
        assert_eq!(doc.tacks.len(), 1);
        assert_eq!(
            doc.tacks[0].a,
            TackPoint {
                pattern: id,
                at: p0
            }
        );
        assert_eq!(
            doc.tacks[0].b,
            TackTarget::Pattern(TackPoint {
                pattern: id,
                at: p2
            })
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
        h.state_mut().session.selection.set(Some(front));
        h.run();
        assert_eq!(
            h.state().session.doc.pattern(front).unwrap().fabric,
            od_core::Fabric::silk()
        );
    }

    // ---- Sewing and tacks ported from claude/phase5-sewing ----

    /// Two 200 × 150 panels, 100 mm apart, and the Sew tool. Outlines run
    /// (x0,0) (x0+200,0) (x0+200,150) (x0,150).
    fn sewing_harness() -> Harness<'static, OpenDesignApp> {
        let mut h = harness();
        {
            let s = &mut h.state_mut().session;
            s.add_pattern(
                "Left",
                od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(200.0, 150.0)),
            );
            s.add_pattern(
                "Right",
                od_core::geometry::rectangle(Vec2::new(300.0, 0.0), Vec2::new(500.0, 150.0)),
            );
            s.selection.set(None);
        }
        h.run();
        press(&mut h, Modifiers::NONE, Key::S);
        assert_eq!(h.state().session.tool, Tool::Sew);
        h
    }

    fn click_mm(h: &mut Harness<'_, OpenDesignApp>, x: f32, y: f32) {
        let at = screen(h, x, y);
        click_at(h, at, Modifiers::NONE);
    }

    fn drag_mm(h: &mut Harness<'_, OpenDesignApp>, from: (f32, f32), to: (f32, f32)) {
        let (from, to) = (screen(h, from.0, from.1), screen(h, to.0, to.1));
        drag(h, from, to);
    }

    /// Sew Left's right edge to Right's left edge, both picked near the top.
    fn sew_side_seam(h: &mut Harness<'_, OpenDesignApp>) -> od_core::SewingLineId {
        click_mm(h, 200.0, 25.0);
        press(h, Modifiers::NONE, Key::Tab);
        click_mm(h, 300.0, 25.0);
        press(h, Modifiers::NONE, Key::Enter);
        h.state().session.doc.sewing_lines[0].id
    }

    #[test]
    fn picking_edges_near_matching_ends_sews_them_untwisted() {
        let mut h = sewing_harness();
        let id = sew_side_seam(&mut h);
        let s = &h.state().session;
        let line = s.doc.sewing_line(id).unwrap();
        assert!(line.a[0].forward, "Left's right edge runs top-down");
        assert!(!line.b[0].forward, "Right's left edge runs bottom-up");
        let start = |side| s.doc.side_point(line, side, 0.0).unwrap().1;
        assert_eq!(start(od_core::Side::A), Vec2::new(200.0, 0.0));
        assert_eq!(start(od_core::Side::B), Vec2::new(300.0, 0.0));
    }

    #[test]
    fn free_sewing_can_cross_the_outlines_start() {
        let mut h = sewing_harness();
        // From 40 mm along Left's top edge, back round its top-left corner
        // (where the outline starts) and 40 mm down its left edge.
        drag_mm(&mut h, (40.0, 0.0), (0.0, 40.0));
        let sides = h.state().session.canvas.sew_pending_edges().unwrap();
        let edge = sides[0][0];
        let pattern = h.state().session.doc.patterns[0].clone();
        let length = pattern.segment_length(edge.start, edge.end, od_core::document::TOLERANCE);
        assert!((length - 80.0).abs() < 2.0, "{edge:?} is {length} mm");
        assert!(!edge.forward, "sewn from the top edge back round");
        assert!(pattern.edge_points(&edge)[0].distance(Vec2::new(40.0, 0.0)) < 1.0);
    }

    #[test]
    fn seams_can_be_selected_flipped_edited_and_deleted() {
        let mut h = sewing_harness();
        let id = sew_side_seam(&mut h);
        press(&mut h, Modifiers::NONE, Key::V);
        click_mm(&mut h, 250.0, 75.0);
        assert_eq!(h.state().session.selected_seam, None);
        click_mm(&mut h, 200.0, 75.0);
        assert_eq!(h.state().session.selected_seam, Some(id));
        assert_eq!(h.state().session.selection.primary(), None);
        h.get_by_label("Selected: seam");
        h.get_by_label("1:1");

        click(&mut h, "Flip side B");
        assert!(h.state().session.doc.sewing_line(id).unwrap().b[0].forward);
        assert_eq!(last_step(&h), "Flip seam");
        press(&mut h, Modifiers::COMMAND, Key::Z);

        // Drag side A's first end (Left's top-right corner) 50 mm down.
        drag_mm(&mut h, (200.0, 0.0), (204.0, 50.0));
        assert_eq!(last_step(&h), "Edit sewing");
        let s = &h.state().session;
        let line = s.doc.sewing_line(id).unwrap();
        let (a, b) = line.side_lengths(&s.doc).unwrap();
        assert!(
            (a - 100.0).abs() < 0.5 && (b - 150.0).abs() < 0.1,
            "{a} {b}"
        );
        assert_eq!(
            s.doc.patterns[0].outline[1].pos,
            Vec2::new(200.0, 0.0),
            "pattern stays"
        );

        press(&mut h, Modifiers::NONE, Key::Delete);
        assert!(h.state().session.doc.sewing_lines.is_empty());
        assert_eq!(h.state().session.doc.patterns.len(), 2);
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.sewing_lines.len(), 1);
    }

    #[test]
    fn the_length_check_can_be_tuned_and_jumped_to() {
        let mut h = sewing_harness();
        // 100 mm of Left's bottom edge against 70 mm of Right's.
        drag_mm(&mut h, (40.0, 150.0), (140.0, 150.0));
        press(&mut h, Modifiers::NONE, Key::Tab);
        drag_mm(&mut h, (320.0, 150.0), (390.0, 150.0));
        press(&mut h, Modifiers::NONE, Key::Enter);
        let id = h.state().session.doc.sewing_lines[0].id;

        h.get_by_label("1 seam differs in length").click();
        h.run();
        assert_eq!(h.state().session.selected_seam, Some(id));

        h.state_mut().session.seam_check.percent = 50.0;
        h.run();
        assert!(h.query_by_label("1 seam differs in length").is_none());
        h.state_mut().session.seam_check = Default::default();
        h.state_mut().session.seam_check.enabled = false;
        h.run();
        assert!(h.query_by_label("1 seam differs in length").is_none());
    }

    #[test]
    fn tack_tool_holds_any_pattern_point_to_another() {
        let mut h = sewing_harness();
        press(&mut h, Modifiers::NONE, Key::K);
        assert_eq!(h.state().session.tool, Tool::Tack);
        click_mm(&mut h, 250.0, 300.0);
        assert!(h.state().session.tack_start.is_none(), "off every pattern");
        click_mm(&mut h, 50.0, 50.0);
        assert!(h.state().session.tack_start.is_some());
        press(&mut h, Modifiers::NONE, Key::Escape);
        assert!(h.state().session.tack_start.is_none(), "Esc cancels");

        click_mm(&mut h, 50.0, 50.0);
        click_mm(&mut h, 320.0, 60.0);
        let s = &h.state().session;
        assert_eq!(s.doc.tacks.len(), 1);
        let tack = s.doc.tacks[0].clone();
        assert!(tack.a.at.distance(Vec2::new(50.0, 50.0)) < 1.0);
        let TackTarget::Pattern(b) = tack.b else {
            panic!("tacked to a pattern");
        };
        assert_eq!(b.pattern, s.doc.patterns[1].id);
        h.get_by_label("Selected: tack");
        h.get_by_label("Delete tack");

        press(&mut h, Modifiers::NONE, Key::V);
        click_mm(&mut h, 250.0, 75.0);
        assert_eq!(h.state().session.selected_tack, None);
        click_mm(&mut h, 250.0, 55.0);
        assert_eq!(h.state().session.selected_tack, Some(tack.id));
        press(&mut h, Modifiers::NONE, Key::Delete);
        assert!(h.state().session.doc.tacks.is_empty());
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.tacks, [tack]);
    }

    /// Bring `tab` to the front of its dock group.
    fn show_tab(h: &mut Harness<'_, OpenDesignApp>, tab: Tab) {
        let dock = &mut h.state_mut().dock;
        let found = dock.find_tab(&tab).unwrap();
        dock.set_active_tab(found).unwrap();
        h.run();
    }

    /// Two 100 × 80 mm rectangles side by side, not selected.
    fn two_pieces(h: &mut Harness<'_, OpenDesignApp>) -> [PatternId; 2] {
        let s = &mut h.state_mut().session;
        s.add_pattern(
            "Left",
            od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 80.0)),
        );
        s.add_pattern(
            "Right",
            od_core::geometry::rectangle(Vec2::new(150.0, 0.0), Vec2::new(250.0, 80.0)),
        );
        s.selection.clear();
        let ids = [s.doc.patterns[0].id, s.doc.patterns[1].id];
        h.run();
        ids
    }

    #[test]
    fn shift_click_selects_several_and_delete_removes_them_in_one_step() {
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        let at = screen(&h, 50.0, 40.0);
        click_at(&mut h, at, Modifiers::NONE);
        let at = screen(&h, 200.0, 40.0);
        click_at(&mut h, at, Modifiers::SHIFT);
        assert_eq!(h.state().session.selection.ids(), [left, right]);
        assert!(h.query_by_label("2 patterns selected").is_some());
        // Shift-clicking a selected piece takes it back out.
        let at = screen(&h, 50.0, 40.0);
        click_at(&mut h, at, Modifiers::SHIFT);
        assert_eq!(h.state().session.selection.ids(), [right]);
        let at = screen(&h, 50.0, 40.0);
        click_at(&mut h, at, Modifiers::SHIFT);

        press(&mut h, Modifiers::NONE, Key::Delete);
        assert_eq!(pattern_count(&h), 0);
        assert_eq!(last_step(&h), "Delete 2 patterns");
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(pattern_count(&h), 2);
    }

    #[test]
    fn a_box_selects_what_it_touches_and_dragging_moves_them_together() {
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        // From empty space below-left, across both pieces.
        let (from, to) = (screen(&h, -30.0, 120.0), screen(&h, 160.0, 40.0));
        drag(&mut h, from, to);
        assert_eq!(pattern_count(&h), 2, "the Select tool doesn't draw");
        let mut ids = h.state().session.selection.ids().to_vec();
        ids.sort_by_key(|id| id.0);
        assert_eq!(ids, [left, right]);

        let (from, to) = (screen(&h, 50.0, 40.0), screen(&h, 50.0, 140.0));

        drag(&mut h, from, to);
        assert_eq!(last_step(&h), "Move 2 patterns");
        let first = |h: &Harness<'_, OpenDesignApp>, id| {
            h.state().session.doc.pattern(id).unwrap().outline[0].pos
        };
        assert_near(first(&h, left), Vec2::new(0.0, 100.0));
        assert_near(first(&h, right), Vec2::new(150.0, 100.0));
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_near(first(&h, left), Vec2::ZERO);
        assert_near(first(&h, right), Vec2::new(150.0, 0.0));
    }

    #[test]
    fn select_all_duplicates_together_and_escape_clears() {
        let mut h = harness();
        two_pieces(&mut h);
        press(&mut h, Modifiers::COMMAND, Key::A);
        assert_eq!(h.state().session.selection.len(), 2);
        press(&mut h, Modifiers::COMMAND, Key::D);
        assert_eq!(pattern_count(&h), 4);
        assert_eq!(last_step(&h), "Duplicate 2 patterns");
        // The copies are what's selected now.
        let s = &h.state().session;
        let copies: Vec<_> = s.doc.patterns[2..].iter().map(|p| p.id).collect();
        assert_eq!(s.selection.ids(), copies);
        press(&mut h, Modifiers::NONE, Key::Escape);
        assert_eq!(h.state().session.selection.len(), 0);
    }

    #[test]
    fn copied_pieces_paste_as_new_sewn_pieces_and_cut_removes_them() {
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        {
            let s = &mut h.state_mut().session;
            let edge = |pattern, start: f32| od_core::SeamEdge {
                pattern,
                start,
                end: start + 1.0,
                forward: true,
            };
            s.execute(od_core::AddSewingLine::new(
                vec![edge(left, 1.0)],
                vec![edge(right, 3.0)],
            ));
        }
        press(&mut h, Modifiers::COMMAND, Key::A);
        press(&mut h, Modifiers::COMMAND, Key::C);
        assert_eq!(
            h.state().session.tool,
            Tool::Select,
            "C alone isn't pressed"
        );
        press(&mut h, Modifiers::COMMAND, Key::V);
        assert_eq!(last_step(&h), "Paste 2 patterns");
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 4);
        assert_eq!(s.doc.sewing_lines.len(), 2, "the seam is pasted too");
        let pasted: Vec<_> = s.doc.patterns[2..].iter().map(|p| p.id).collect();
        assert_eq!(s.selection.ids(), pasted);
        assert_near(
            s.doc.pattern(pasted[0]).unwrap().outline[0].pos,
            Vec2::new(40.0, 40.0),
        );
        // A second paste lands a step further along.
        press(&mut h, Modifiers::COMMAND, Key::V);
        let s = &h.state().session;
        assert_near(s.doc.patterns[4].outline[0].pos, Vec2::new(80.0, 80.0));

        // Cutting the selection removes it in one step, and pastes back.
        press(&mut h, Modifiers::COMMAND, Key::X);
        assert_eq!(pattern_count(&h), 4);
        assert_eq!(last_step(&h), "Cut 2 patterns");
        press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::V);
        assert_eq!(pattern_count(&h), 6);
        assert_eq!(last_step(&h), "Paste 2 patterns mirrored");
    }

    #[test]
    fn hidden_pieces_drop_out_and_locked_ones_stay_put() {
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        show_tab(&mut h, Tab::Objects);
        // Hide the left piece from the Objects list.
        h.get_all_by_label("Show").next().unwrap().click();
        h.run();
        assert_eq!(last_step(&h), "Hide pattern");
        let s = &h.state().session;
        assert!(s.doc.pattern(left).unwrap().hidden);
        let shown: Vec<_> = crate::panels::viewport3d::placements(s)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(shown, [right], "a hidden piece isn't placed in 3D");
        // Clicking where it was picks nothing, and select all skips it.
        let at = screen(&h, 50.0, 40.0);
        click_at(&mut h, at, Modifiers::NONE);
        assert_eq!(h.state().session.selection.len(), 0);
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert!(!h.state().session.doc.pattern(left).unwrap().hidden);

        // Lock the right piece: dragging it does nothing, and Delete
        // removes only the unlocked piece.
        h.get_all_by_label("Lock").nth(1).unwrap().click();
        h.run();
        assert!(h.state().session.doc.pattern(right).unwrap().locked);
        let (from, to) = (screen(&h, 200.0, 40.0), screen(&h, 200.0, 140.0));
        drag(&mut h, from, to);
        let first = h.state().session.doc.pattern(right).unwrap().outline[0].pos;
        assert_near(first, Vec2::new(150.0, 0.0));
        press(&mut h, Modifiers::COMMAND, Key::A);
        press(&mut h, Modifiers::NONE, Key::Delete);
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 1);
        assert_eq!(s.doc.patterns[0].id, right);
        assert_eq!(s.selection.ids(), [right]);
    }

    #[test]
    fn pieces_turn_with_the_handle_or_properties_buttons() {
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        let at = screen(&h, 50.0, 40.0);
        click_at(&mut h, at, Modifiers::NONE);
        // The handle sits above the piece's top edge, at its centre: drag
        // it round to the right of the centre for a quarter turn.
        let (centre, handle) = {
            let s = &h.state().session;
            let (c, handle) = crate::panels::pattern2d::rotate_handle_for_tests(s).unwrap();
            (c, handle)
        };
        let reach = centre.y - handle.y;
        let (from, to) = (
            screen(&h, handle.x, handle.y),
            screen(&h, centre.x + reach, centre.y),
        );
        h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
        drag(&mut h, from, to);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        assert_eq!(last_step(&h), "Rotate pattern");
        let p = h.state().session.doc.pattern(left).unwrap();
        assert_eq!(p.grain_angle, 90.0);
        let (min, max) = p.bounds().unwrap();
        assert_near(max - min, Vec2::new(80.0, 100.0));

        // Properties turns it back.
        h.get_by_label("-90°").click();
        h.run();
        let p = h.state().session.doc.pattern(left).unwrap();
        assert_eq!(p.grain_angle, 0.0);
        assert_near(p.outline[0].pos, Vec2::ZERO);
    }

    #[test]
    fn right_click_menus_act_on_the_clicked_piece() {
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        let at = screen(&h, 200.0, 40.0);
        h.hover_at(at);
        h.run();
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.run();
        assert_eq!(h.state().session.selection.ids(), [right]);
        h.get_by_label("Hide").click();
        h.run();
        let s = &h.state().session;
        assert!(s.doc.pattern(right).unwrap().hidden);
        assert!(!s.doc.pattern(left).unwrap().hidden);
    }

    #[test]
    fn hovering_a_piece_highlights_it_and_f_frames_the_selection() {
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        let at = screen(&h, 50.0, 40.0);
        h.hover_at(at);
        h.run();
        assert_eq!(h.state().session.hovered, Some(left));

        click_at(&mut h, at, Modifiers::NONE);
        press(&mut h, Modifiers::NONE, Key::F);
        // The 2D view centres the piece.
        let centre = h.state().session.canvas.screen_pos(Vec2::new(50.0, 40.0));
        let canvas = h.get_by_label("2D pattern canvas").rect().center();
        assert!(centre.distance(canvas) < 2.0, "{centre:?} vs {canvas:?}");
        // The 3D camera looks at it.
        let s = &h.state().session;
        let placed = crate::panels::viewport3d::placed_patterns(s);
        let piece = placed.iter().find(|p| p.key == left).unwrap();
        let target = s.viewport.camera.target;
        let middle = piece.mesh.bounds().unwrap().center();
        assert!(target.distance(middle) < 0.01, "{target} vs {middle}");
    }

    #[test]
    fn dart_tool_cuts_outline_darts_and_draws_diamond_darts() {
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        press(&mut h, Modifiers::NONE, Key::D);
        assert_eq!(h.state().session.tool, Tool::Dart);
        h.state_mut().session.canvas.dart_depth = 40.0;
        // A click on Left's top edge: a V with its legs sewn.
        let at = screen(&h, 50.0, 0.0);
        click_at(&mut h, at, Modifiers::NONE);
        let s = &h.state().session;
        let p = s.doc.pattern(left).unwrap();
        assert_eq!(p.outline.len(), 7);
        assert!(p
            .positions()
            .any(|q| q.distance(Vec2::new(50.0, 40.0)) < 1e-3));
        assert_eq!(s.doc.sewing_lines.len(), 1);
        assert_eq!(s.history.undo_label().as_deref(), Some("Add dart"));

        // A drag inside Right: a diamond dart, tip to tip.
        h.state_mut().session.canvas.snap.grid = false;
        drag_mm(&mut h, (200.0, 10.0), (200.0, 70.0));
        let p = h.state().session.doc.pattern(right).unwrap();
        assert_eq!(p.darts.len(), 1);
        let (start, end) = (p.darts[0].start, p.darts[0].end);
        assert!(start.distance(Vec2::new(200.0, 10.0)) < 1.0, "{start:?}");
        assert!(end.distance(Vec2::new(200.0, 70.0)) < 1.0, "{end:?}");

        // Each is one undo step.
        press(&mut h, Modifiers::COMMAND, Key::Z);
        press(&mut h, Modifiers::COMMAND, Key::Z);
        let s = &h.state().session;
        assert!(s.doc.pattern(right).unwrap().darts.is_empty());
        assert_eq!(s.doc.pattern(left).unwrap().outline.len(), 4);
        assert!(s.doc.sewing_lines.is_empty());
    }

    #[test]
    fn pleat_tool_lays_fold_lines_across_the_piece() {
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        press(&mut h, Modifiers::NONE, Key::Q);
        assert_eq!(h.state().session.tool, Tool::Pleat);
        h.state_mut().session.canvas.snap.grid = false;
        // Upwards, so the knife pleat's lines step to the right.
        drag_mm(&mut h, (20.0, 70.0), (20.0, 10.0));
        let s = &h.state().session;
        let lines = &s.doc.pattern(left).unwrap().internal_lines;
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].fold.is_some_and(|f| f.angle < 0.0),
            "a mountain first"
        );
        assert!(
            lines[1].fold.is_some_and(|f| f.angle > 0.0),
            "then a valley"
        );
        assert_eq!(lines[2].fold, None, "and where it lands");
        // Each runs from the top edge to the bottom one.
        for line in lines {
            let ys: Vec<f32> = line.points.iter().map(|p| p.y).collect();
            assert!(ys.iter().any(|y| y.abs() < 0.5) && ys.iter().any(|y| (y - 80.0).abs() < 0.5));
        }
        // The top, where the drag started, is tacked: both folds onto the
        // line they land on.
        assert_eq!(s.doc.tacks.len(), 2, "{:?}", s.doc.tacks);
        let place = lines[2].points[0];
        for tack in &s.doc.tacks {
            let od_core::TackTarget::Pattern(b) = tack.b else {
                panic!("{tack:?}");
            };
            assert_eq!((tack.a.pattern, b.pattern), (left, left));
            assert_eq!(b.at, place);
            assert!((tack.a.at.y - 80.0).abs() < 0.5, "{tack:?}");
        }
        assert_eq!(s.history.undo_label().as_deref(), Some("Add pleats"));
        press(&mut h, Modifiers::COMMAND, Key::Z);
        let s = &h.state().session;
        assert!(s.doc.pattern(left).unwrap().internal_lines.is_empty());
        assert!(s.doc.tacks.is_empty());

        // Without tacking, only the lines.
        h.state_mut().session.canvas.pleat.tack = false;
        drag_mm(&mut h, (20.0, 70.0), (20.0, 10.0));
        let s = &h.state().session;
        assert_eq!(s.doc.pattern(left).unwrap().internal_lines.len(), 3);
        assert!(s.doc.tacks.is_empty());
    }

    /// Screenshots of the dart, pleat and linked-copy tools for a look by
    /// eye, written to `$OD_SHOTS` (default: a temporary folder). Needs a
    /// wgpu adapter; a software one (lavapipe) is enough.
    #[test]
    #[ignore = "writes screenshots; run with --ignored"]
    fn screenshots_of_darts_pleats_and_linked_copies() {
        let dir = std::env::var_os("OD_SHOTS")
            .map_or_else(|| std::env::temp_dir().join("od_shots"), Into::into);
        std::fs::create_dir_all(&dir).unwrap();
        let shot = |h: &mut Harness<'_, OpenDesignApp>, name: &str| {
            h.run();
            let image = h.render().expect("a wgpu adapter to render with");
            image.save(dir.join(name)).unwrap();
        };
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 880.0))
            .wgpu()
            .build_eframe(|cc| OpenDesignApp::new(cc));
        // The 3D view draws with a depth buffer the test renderer's pass
        // doesn't have; these shots are of the 2D window and Properties.
        let dock = &mut h.state_mut().dock;
        if let Some(found) = dock.find_tab(&Tab::Viewport3D) {
            dock.remove_tab(found);
        }
        h.run();
        let [left, right] = two_pieces(&mut h);
        h.state_mut().session.canvas.snap.grid = false;
        press(&mut h, Modifiers::COMMAND, Key::A);
        press(&mut h, Modifiers::NONE, Key::F);
        h.state_mut().session.selection.clear();
        h.run();

        // Darts: an outline dart in Right's top edge, a diamond inside it,
        // and the diamond tool mid-drag.
        press(&mut h, Modifiers::NONE, Key::D);
        h.state_mut().session.canvas.dart_depth = 35.0;
        let at = screen(&h, 200.0, 0.0);
        click_at(&mut h, at, Modifiers::NONE);
        drag_mm(&mut h, (175.0, 25.0), (175.0, 75.0));
        let (from, to) = (screen(&h, 225.0, 25.0), screen(&h, 225.0, 70.0));
        h.drag_at(from);
        h.run();
        h.hover_at(to);
        shot(&mut h, "dart-tool.png");
        h.drop_at(to);
        h.run();

        // Pleats across Left, then its Properties: the fold lines with
        // their angle and strength, and the pleat tool's settings.
        press(&mut h, Modifiers::NONE, Key::Q);
        drag_mm(&mut h, (15.0, 75.0), (15.0, 5.0));
        h.state_mut().session.selection.set(Some(left));
        show_tab(&mut h, Tab::Properties);
        shot(&mut h, "pleats.png");

        // Right's Properties: its darts, editable.
        h.state_mut().session.tool = Tool::Select;
        h.state_mut().session.selection.set(Some(right));
        shot(&mut h, "darts-properties.png");

        // A linked mirrored copy of Left, selected.
        h.state_mut().session.selection.set(Some(left));
        h.state_mut().session.linked_copy_selected();
        h.run();
        press(&mut h, Modifiers::COMMAND, Key::A);
        press(&mut h, Modifiers::NONE, Key::F);
        let copy = h.state().session.doc.patterns.last().unwrap().id;
        h.state_mut().session.selection.set(Some(copy));
        shot(&mut h, "linked-copy.png");
        eprintln!("screenshots in {}", dir.display());
    }

    #[test]
    fn diamond_darts_are_resized_in_properties() {
        use od_core::{AddDart, Dart};
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        let dart = Dart {
            start: Vec2::new(50.0, 20.0),
            end: Vec2::new(50.0, 60.0),
            width: 10.0,
        };
        {
            let s = &mut h.state_mut().session;
            s.execute(AddDart::new(left, dart));
            s.selection.set(Some(left));
        }
        show_tab(&mut h, Tab::Properties);
        let field = |h: &Harness<'_, OpenDesignApp>, suffix: &str| {
            h.get_all_by_role(Role::SpinButton)
                .find(|n| {
                    n.accesskit_node()
                        .value()
                        .is_some_and(|v| v.contains(suffix))
                })
                .unwrap_or_else(|| panic!("no {suffix} field"))
                .rect()
                .center()
        };
        let wide = field(&h, "mm wide");
        drag(&mut h, wide, wide + egui::vec2(20.0, 0.0));
        let got = h.state().session.doc.pattern(left).unwrap().darts[0];
        assert!(got.width > 10.0, "{got:?}");
        assert_eq!((got.start, got.end), (dart.start, dart.end));
        assert_eq!(
            h.state().session.history.undo_label().as_deref(),
            Some("Edit dart")
        );

        // Longer, about the same middle.
        let long = field(&h, "mm long");
        drag(&mut h, long, long + egui::vec2(20.0, 0.0));
        let got = h.state().session.doc.pattern(left).unwrap().darts[0];
        let length = got.start.distance(got.end);
        assert!(length > 40.0, "{got:?}");
        assert!(got.start.lerp(got.end, 0.5).distance(Vec2::new(50.0, 40.0)) < 1e-3);
        press(&mut h, Modifiers::COMMAND, Key::Z);
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.pattern(left).unwrap().darts[0], dart);
    }

    #[test]
    fn fold_strength_is_set_in_properties() {
        use od_core::{AddInternalLine, Fold, SetFold};
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        {
            let s = &mut h.state_mut().session;
            s.execute(AddInternalLine::new(
                left,
                vec![Vec2::new(10.0, 40.0), Vec2::new(90.0, 40.0)],
            ));
            s.execute(SetFold::new(left, 0, Some(Fold::valley())));
            s.selection.set(Some(left));
        }
        show_tab(&mut h, Tab::Properties);
        let field = h
            .get_all_by_role(Role::SpinButton)
            .find(|n| {
                n.accesskit_node()
                    .value()
                    .is_some_and(|v| v.contains("% firm"))
            })
            .expect("a fold strength field")
            .rect()
            .center();
        drag(&mut h, field, field + egui::vec2(30.0, 0.0));
        let s = &h.state().session;
        let fold = s.doc.pattern(left).unwrap().internal_lines[0].fold.unwrap();
        assert!(fold.strength > 0.6, "{fold:?}");
        assert_eq!(fold.angle, Fold::DEFAULT_ANGLE);
        // One undo step for the whole drag.
        press(&mut h, Modifiers::COMMAND, Key::Z);
        let fold = h.state().session.doc.pattern(left).unwrap().internal_lines[0].fold;
        assert_eq!(fold, Some(Fold::valley()));
    }

    #[test]
    fn cut_and_sew_then_merge_from_properties() {
        use od_core::{AddInternalLine, AddSewingLine, SeamEdge};
        let mut h = harness();
        let [left, right] = two_pieces(&mut h);
        {
            let s = &mut h.state_mut().session;
            s.execute(AddInternalLine::new(
                left,
                vec![Vec2::new(-10.0, 40.0), Vec2::new(110.0, 40.0)],
            ));
            s.selection.set(Some(left));
        }
        show_tab(&mut h, Tab::Properties);
        click(&mut h, "Cut and sew");
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 3);
        assert_eq!(s.doc.sewing_lines.len(), 1);
        assert_eq!(s.selection.len(), 2, "both halves selected");
        assert_eq!(
            s.history.undo_label().as_deref(),
            Some("Cut and sew pattern")
        );
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.doc.patterns.len(), 2);

        // Sew Left's right side to Right's left side, then merge them.
        let edge = |pattern, start: f32, forward| SeamEdge {
            pattern,
            start,
            end: start + 1.0,
            forward,
        };
        {
            let s = &mut h.state_mut().session;
            s.execute(AddSewingLine::new(
                vec![edge(left, 1.0, true)],
                vec![edge(right, 3.0, false)],
            ));
            s.selected_seam = s.doc.sewing_lines.last().map(|l| l.id);
            s.selection.clear();
        }
        h.run();
        click(&mut h, "Merge pieces");
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 1);
        let merged = s.doc.pattern(left).unwrap();
        assert!((merged.area() - 16_000.0).abs() < 1.0, "{}", merged.area());
        assert!(s.doc.sewing_lines.is_empty());
    }

    #[test]
    fn a_linked_copy_passes_its_edits_to_the_source() {
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        h.state_mut().session.selection.set(Some(left));
        show_tab(&mut h, Tab::Properties);
        click(&mut h, "Linked mirrored copy");
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 3);
        let copy = s.selection.primary().unwrap();
        assert_eq!(s.doc.pattern(copy).unwrap().link.unwrap().source, left);
        assert!(h.query_by_label("Unlink").is_some());

        // Point 0 picked on the copy: C curves it, Delete removes it, and
        // both happen to the source too.
        {
            let s = &mut h.state_mut().session;
            s.tool = Tool::EditPoints;
            s.canvas.selected_point = Some((copy, 0));
        }
        h.run();
        press(&mut h, Modifiers::NONE, Key::C);
        let kind = |h: &Harness<'_, OpenDesignApp>, id| {
            h.state().session.doc.pattern(id).unwrap().outline[0].kind
        };
        assert_eq!(kind(&h, left), od_core::PointKind::Curve);
        assert_eq!(kind(&h, copy), od_core::PointKind::Curve);
        press(&mut h, Modifiers::NONE, Key::Delete);
        let s = &h.state().session;
        assert_eq!(s.doc.pattern(left).unwrap().outline.len(), 3);
        assert_eq!(s.doc.pattern(copy).unwrap().outline.len(), 3);
        // Undo puts both back.
        press(&mut h, Modifiers::COMMAND, Key::Z);
        let s = &h.state().session;
        assert_eq!(s.doc.pattern(copy).unwrap().outline.len(), 4);

        // Fabric and seam allowance picked on the copy are the source's too.
        h.state_mut().session.tool = Tool::Select;
        h.run();
        click(&mut h, "Denim");
        let s = &h.state().session;
        for id in [left, copy] {
            assert_eq!(s.doc.pattern(id).unwrap().fabric.name, "Denim");
        }
        let allowance = h
            .get_all_by_role(Role::SpinButton)
            .next()
            .unwrap()
            .rect()
            .center();
        drag(&mut h, allowance, allowance + egui::vec2(30.0, 0.0));
        let s = &h.state().session;
        let (a, b) = (
            s.doc.pattern(left).unwrap().seam_allowance,
            s.doc.pattern(copy).unwrap().seam_allowance,
        );
        assert!(a > 10.0 && a == b, "{a} {b}");
    }

    #[test]
    fn round_corner_from_properties_is_one_undo_step() {
        let mut h = harness();
        let [left, _] = two_pieces(&mut h);
        {
            let s = &mut h.state_mut().session;
            s.selection.set(Some(left));
            s.tool = Tool::EditPoints;
            s.canvas.selected_point = Some((left, 1));
        }
        show_tab(&mut h, Tab::Properties);
        click(&mut h, "Round corner");
        let s = &h.state().session;
        let p = s.doc.pattern(left).unwrap();
        assert_eq!(p.outline.len(), 6);
        assert_eq!(p.outline[2].kind, od_core::PointKind::Curve);
        assert_eq!(s.history.undo_label().as_deref(), Some("Round corner"));
        press(&mut h, Modifiers::COMMAND, Key::Z);
        let p = h.state().session.doc.pattern(left).unwrap();
        assert_eq!(p.outline.len(), 4);
    }

    // ---- Import (CLO3D gap Phase 5) ----

    #[test]
    fn dropping_a_dxf_adds_its_pieces_beside_the_others_as_one_step() {
        let dir = temp_dir("dxf_drop");
        let path = dir.join("pieces.dxf");
        let mut source = Document::new("source");
        for (name, x) in [("Front", 0.0), ("Back", 200.0)] {
            let outline =
                od_core::geometry::rectangle(Vec2::new(x, 0.0), Vec2::new(x + 120.0, 300.0));
            AddPattern::new(name, outline).apply(&mut source);
        }
        od_core::dxf::export(&source, &path).unwrap();

        let mut h = harness();
        click(&mut h, "+ Sleeve");
        let sleeve_right = h.state().session.doc.patterns[0].bounds().unwrap().1.x;
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(DroppedPath(path)));
        h.run();
        let s = &h.state().session;
        assert_eq!(s.doc.patterns.len(), 3);
        assert_eq!(s.doc.patterns[1].name, "Front");
        assert_eq!(s.doc.patterns[2].name, "Back");
        let front = s.doc.patterns[1].bounds().unwrap();
        assert!(front.0.x > sleeve_right, "placed beside the sleeve");
        assert!((front.1.x - front.0.x - 120.0).abs() < 0.01);
        assert_eq!(s.selection.len(), 2, "the imported pieces are selected");
        assert_eq!(s.history.undo_label().as_deref(), Some("Import pieces.dxf"));
        assert!(s.history.is_dirty());
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(pattern_count(&h), 1, "one undo removes the whole import");
    }

    #[test]
    fn a_bad_dxf_shows_an_error_and_the_menu_offers_import() {
        let path = temp_dir("dxf_bad").join("broken.dxf");
        std::fs::write(&path, "this is not a dxf").unwrap();
        let mut h = harness();
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(DroppedPath(path)));
        h.run();
        h.get_by_label("Something went wrong");
        assert_eq!(pattern_count(&h), 0);
        click(&mut h, "OK");
        click(&mut h, "File");
        click(&mut h, "Import ⏵");
        h.get_by_label("Patterns from DXF (AAMA)…");
    }

    // ---- Auto-save and recovery (CLO3D gap Phase 5) ----

    use crate::recovery;

    /// A harness whose auto-save uses its own folder, writing every frame.
    fn autosaving(test: &str) -> (Harness<'static, OpenDesignApp>, PathBuf) {
        let dir = temp_dir(test).join("data");
        let mut h = harness();
        let mut autosave = AutoSave::new(Some(dir.clone()));
        autosave.interval = 0.0;
        h.state_mut().autosave = autosave;
        (h, dir)
    }

    #[test]
    fn unsaved_work_is_auto_saved_and_cleared_by_saving() {
        let (mut h, dir) = autosaving("autosave");
        h.run();
        assert!(recovery::find(&dir).is_none(), "nothing unsaved yet");
        click(&mut h, "+ Sleeve");
        h.run();
        let found = recovery::find(&dir).expect("written while unsaved");
        assert_eq!(found.doc, h.state().session.doc);
        assert_eq!(found.source, None);

        let project = dir.parent().unwrap().join("mine.odproj");
        {
            let app = h.state_mut();
            assert!(app.files.write(&mut app.session, &project));
        }
        h.run();
        assert!(recovery::find(&dir).is_none(), "a clean save clears it");
        assert!(project.exists(), "the user's file is separate");

        // Changes after a save point back at the project.
        click(&mut h, "+ Sleeve");
        h.run();
        let found = recovery::find(&dir).unwrap();
        assert_eq!(found.source.as_deref(), Some(project.as_path()));
        assert_eq!(od_core::project::load(&project).unwrap().patterns.len(), 1);
        // Undo back to the saved state is clean again.
        press(&mut h, Modifiers::COMMAND, Key::Z);
        h.run();
        assert!(recovery::find(&dir).is_none());
    }

    #[test]
    fn closing_normally_drops_the_recovery_file() {
        let (mut h, dir) = autosaving("autosave_quit");
        click(&mut h, "+ Sleeve");
        h.run();
        assert!(recovery::find(&dir).is_some());
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
        click(&mut h, "Cancel");
        assert!(recovery::find(&dir).is_some(), "still open, still unsaved");
        request_close(&mut h);
        click(&mut h, "Don't save");
        // The window closes on the next close request eframe sends.
        request_close(&mut h);
        assert!(recovery::find(&dir).is_none());
    }

    /// A harness that starts up finding `doc` left in its recovery folder.
    fn recovering(
        test: &str,
        doc: &Document,
        source: Option<&Path>,
    ) -> (Harness<'static, OpenDesignApp>, PathBuf) {
        let (mut h, dir) = autosaving(test);
        recovery::write(&dir, doc, source).unwrap();
        h.state_mut().autosave.check();
        h.run();
        (h, dir)
    }

    fn crashed_doc() -> Document {
        let mut doc = Document::new("Crashed");
        let outline = od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(200.0, 90.0));
        AddPattern::new("Cuff", outline).apply(&mut doc);
        doc
    }

    #[test]
    fn recovered_work_is_offered_and_restored_as_unsaved() {
        let project = temp_dir("recover_restore").join("shirt.odproj");
        od_core::project::save(&Document::new("shirt"), &project).unwrap();
        // The project was saved before the crash copy was written.
        let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        std::fs::File::options()
            .write(true)
            .open(&project)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        let (mut h, _dir) = recovering("recover_restore_app", &crashed_doc(), Some(&project));
        h.get_by_label("Recover unsaved work?");
        // The document can't be changed while the offer is up.
        press(&mut h, Modifiers::COMMAND, Key::N);
        click(&mut h, "Restore");
        let app = h.state();
        assert_eq!(app.session.doc, crashed_doc());
        assert!(app.session.history.is_dirty(), "restored work is unsaved");
        assert_eq!(app.files.path.as_deref(), Some(project.as_path()));
        assert!(h.query_by_label("Recover unsaved work?").is_none());
    }

    #[test]
    fn discarding_recovered_work_deletes_it() {
        let (mut h, dir) = recovering("recover_discard", &crashed_doc(), None);
        click(&mut h, "Discard");
        assert!(h.state().session.doc.patterns.is_empty());
        assert!(recovery::find(&dir).is_none());
        assert!(h.query_by_label("Recover unsaved work?").is_none());
    }

    #[test]
    fn a_garment_model_is_shown_for_reference_and_can_be_removed() {
        let dir = temp_dir("reference_obj");
        let path = dir.join("shirt.obj");
        // A triangle in centimetres at chest height.
        std::fs::write(&path, "v 0 120 0\nv 40 120 0\nv 0 150 10\nf 1 2 3\n").unwrap();
        let mut h = harness();
        crate::import::import_reference(&mut h.state_mut().session, &path).unwrap();
        h.run();
        {
            let s = &h.state().session;
            let r = s.viewport.reference.as_ref().unwrap();
            assert!(r.shown);
            let b = r.mesh.bounds().unwrap();
            assert!((b.min.y - 1.2).abs() < 1e-4, "kept in place, in metres");
            assert!(!s.history.is_dirty(), "not part of the document");
        }
        click(&mut h, "File");
        click(&mut h, "Import ⏵");
        click(&mut h, "Show shirt.obj");
        assert!(!h.state().session.viewport.reference.as_ref().unwrap().shown);
        if h.query_by_label("Remove reference garment").is_none() {
            // The click closed the menu.
            click(&mut h, "File");
            click(&mut h, "Import ⏵");
        }
        click(&mut h, "Remove reference garment");
        assert!(h.state().session.viewport.reference.is_none());

        let bad = dir.join("broken.obj");
        std::fs::write(&bad, "v 0 0 0\n").unwrap();
        let err = crate::import::import_reference(&mut h.state_mut().session, &bad);
        assert!(err.unwrap_err().contains("no triangles"));
    }

    /// Like the darts and pleats screenshots above, for Phase 5: pieces
    /// imported from DXF, the Import menu and the recovery offer.
    #[test]
    #[ignore = "writes screenshots; run with --ignored"]
    fn screenshots_of_imports_and_recovery() {
        let dir = std::env::var_os("OD_SHOTS")
            .map_or_else(|| std::env::temp_dir().join("od_shots"), Into::into);
        std::fs::create_dir_all(&dir).unwrap();
        let shot = |h: &mut Harness<'_, OpenDesignApp>, name: &str| {
            h.run();
            let image = h.render().expect("a wgpu adapter to render with");
            image.save(dir.join(name)).unwrap();
        };
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 880.0))
            .wgpu()
            .build_eframe(|cc| OpenDesignApp::new(cc));
        let dock = &mut h.state_mut().dock;
        if let Some(found) = dock.find_tab(&Tab::Viewport3D) {
            dock.remove_tab(found);
        }
        h.run();

        // A sleeve exported to DXF and dropped back in beside the original.
        click(&mut h, "+ Sleeve");
        let files = temp_dir("shots_import");
        let dxf = files.join("sleeve.dxf");
        od_core::dxf::export(&h.state().session.doc, &dxf).unwrap();
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(DroppedPath(dxf)));
        h.run();
        press(&mut h, Modifiers::COMMAND, Key::A);
        press(&mut h, Modifiers::NONE, Key::F);
        show_tab(&mut h, Tab::Properties);
        shot(&mut h, "dxf-import.png");

        click(&mut h, "File");
        click(&mut h, "Import ⏵");
        shot(&mut h, "import-menu.png");
        press(&mut h, Modifiers::NONE, Key::Escape);

        // The offer to restore auto-saved work at start-up.
        let data = files.join("data");
        let source = files.join("shirt.odproj");
        recovery::write(&data, &crashed_doc(), Some(&source)).unwrap();
        h.state_mut().autosave = AutoSave::new(Some(data));
        h.state_mut().autosave.check();
        shot(&mut h, "recovery-offer.png");
        eprintln!("screenshots in {}", dir.display());
    }
}
