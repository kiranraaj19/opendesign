//! Project files: open/save, recent files, and asking before unsaved work is
//! thrown away.

use std::path::{Path, PathBuf};

use egui::ViewportCommand;
use od_core::project::{self, ProjectError};
use od_core::Document;

use crate::app::Session;

/// Storage key for the recent-files list.
pub const RECENT_KEY: &str = "recent_files_v1";
const MAX_RECENT: usize = 8;

/// An action that replaces the current document, so may lose changes.
#[derive(Debug, Clone, PartialEq)]
pub enum Discard {
    New,
    /// Open this file, or ask which one with a file dialog.
    Open(Option<PathBuf>),
    Quit,
}

enum Choice {
    Save,
    DontSave,
    Cancel,
}

#[derive(Default)]
pub struct FileState {
    /// Where the current document lives; `None` until first saved.
    pub path: Option<PathBuf>,
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    /// Waiting for an answer to "Save changes?".
    pub pending: Option<Discard>,
    /// Last file error, shown until dismissed.
    pub error: Option<String>,
    /// Set once quitting is confirmed, so the window close goes through.
    pub quitting: bool,
    last_title: String,
}

impl FileState {
    pub fn with_recent(recent: Vec<PathBuf>) -> Self {
        Self {
            recent,
            ..Self::default()
        }
    }

    /// A dialog is open; the rest of the app should ignore shortcuts.
    pub fn is_blocking(&self) -> bool {
        self.pending.is_some() || self.error.is_some()
    }

    fn remember(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(MAX_RECENT);
    }

    /// Run `action` now, or first ask whether to save unsaved changes.
    pub fn request(&mut self, s: &mut Session, ctx: &egui::Context, action: Discard) {
        if s.history.is_dirty() {
            self.pending = Some(action);
        } else {
            self.perform(s, ctx, action);
        }
    }

    fn perform(&mut self, s: &mut Session, ctx: &egui::Context, action: Discard) {
        match action {
            Discard::New => {
                s.load_document(Document::default());
                self.path = None;
            }
            Discard::Open(path) => {
                if let Some(path) = path.or_else(pick_open) {
                    self.open(s, &path);
                }
            }
            Discard::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    /// Save to the current file, or ask where if there isn't one yet.
    pub fn save(&mut self, s: &mut Session) -> bool {
        match self.path.clone() {
            Some(path) => self.write(s, &path),
            None => self.save_as(s),
        }
    }

    pub fn save_as(&mut self, s: &mut Session) -> bool {
        match pick_save(&s.doc.name) {
            Some(path) => self.write(s, &path),
            None => false,
        }
    }

    /// Save to `path` and make it the current file.
    pub fn write(&mut self, s: &mut Session, path: &Path) -> bool {
        // The document takes the file's name, like most desktop apps.
        let previous_name = std::mem::replace(&mut s.doc.name, file_stem(path));
        match project::save(&s.doc, path) {
            Ok(()) => {
                s.history.mark_saved();
                self.path = Some(path.to_owned());
                self.remember(path);
                true
            }
            Err(e) => {
                s.doc.name = previous_name;
                self.error = Some(format!("Couldn't save {}:\n{e}", path.display()));
                false
            }
        }
    }

    /// Replace the current document with the one in `path`.
    pub fn open(&mut self, s: &mut Session, path: &Path) -> bool {
        match project::load(path) {
            Ok(mut doc) => {
                doc.name = file_stem(path);
                s.load_document(doc);
                self.path = Some(path.to_owned());
                self.remember(path);
                true
            }
            Err(e) => {
                if matches!(&e, ProjectError::Io(io) if io.kind() == std::io::ErrorKind::NotFound) {
                    self.recent.retain(|p| p != path);
                }
                self.error = Some(format!("Couldn't open {}:\n{e}", path.display()));
                false
            }
        }
    }

    /// Stop the window closing while there are unsaved changes, and ask.
    pub fn intercept_close(&mut self, s: &Session, ctx: &egui::Context) {
        let requested = ctx.input(|i| i.viewport().close_requested());
        if requested && !self.quitting && s.history.is_dirty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending = Some(Discard::Quit);
        }
    }

    /// Open a project file dropped onto the window.
    pub fn handle_dropped(&mut self, s: &mut Session, ctx: &egui::Context) {
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path())
                .find(|p| has_project_extension(p))
                .map(Path::to_owned)
        });
        if let Some(path) = dropped {
            self.request(s, ctx, Discard::Open(Some(path)));
        }
    }

    pub fn update_title(&mut self, s: &Session, ctx: &egui::Context) {
        let title = window_title(s);
        if title != self.last_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    /// The "Save changes?" and error dialogs. Call once per frame.
    pub fn dialogs(&mut self, s: &mut Session, ctx: &egui::Context) {
        if let Some(action) = self.pending.clone() {
            let mut choice = None;
            let modal = egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.heading("Save changes?");
                ui.label(format!(
                    "\"{}\" has unsaved changes. Save them before continuing?",
                    s.doc.name
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(Choice::Save);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(Choice::DontSave);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });
            if choice.is_none() && modal.should_close() {
                choice = Some(Choice::Cancel);
            }
            match choice {
                Some(Choice::Save) => {
                    self.pending = None;
                    // Only continue if the save worked (not cancelled or failed).
                    if self.save(s) {
                        self.perform(s, ctx, action);
                    }
                }
                Some(Choice::DontSave) => {
                    self.pending = None;
                    self.perform(s, ctx, action);
                }
                Some(Choice::Cancel) => self.pending = None,
                None => {}
            }
        }

        if let Some(message) = &self.error {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::new("file_error")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.heading("Something went wrong");
                ui.label(message);
                ui.add_space(8.0);
                close = ui.button("OK").clicked();
            });
            if close || modal.should_close() {
                self.error = None;
            }
        }
    }
}

pub fn window_title(s: &Session) -> String {
    let marker = if s.history.is_dirty() { " *" } else { "" };
    format!("{}{marker} - OpenDesign", s.doc.name)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
}

fn has_project_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(project::EXTENSION))
}

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("OpenDesign project", &[project::EXTENSION])
}

fn pick_open() -> Option<PathBuf> {
    dialog().pick_file()
}

fn pick_save(name: &str) -> Option<PathBuf> {
    let path = dialog()
        .set_file_name(format!("{name}.{}", project::EXTENSION))
        .save_file()?;
    Some(if has_project_extension(&path) {
        path
    } else {
        path.with_extension(project::EXTENSION)
    })
}
