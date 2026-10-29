//! Auto-save and crash recovery.
//!
//! While the document has unsaved changes, it is written every
//! [`INTERVAL`] seconds to a recovery file in the app's data folder (next
//! to eframe's own settings), never to the user's project file. A clean
//! state (saved, undone back to the save, or a new or opened document)
//! deletes it, and so does quitting normally. So a recovery file only
//! survives a crash or a killed process; on the next start, if it is
//! newer than the project it belongs to, the user is offered it back.
//!
//! One recovery slot is shared by every running copy of the app.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use od_core::project;
use od_core::Document;

use crate::app::Session;
use crate::files::FileState;

/// Seconds between auto-saves while there are unsaved changes.
pub const INTERVAL: f64 = 60.0;

const DOCUMENT_FILE: &str = "recovery.odproj";
/// Holds the path of the project the recovered document came from, or
/// nothing if it was never saved.
const SOURCE_FILE: &str = "recovery.path";

/// A recovery file found at start-up, waiting for the user to restore or
/// discard it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub doc: Document,
    /// The project file it belongs to, if it had been saved.
    pub source: Option<PathBuf>,
    pub written: SystemTime,
}

/// The recovery folder and when it was last written.
pub struct AutoSave {
    /// `None` turns auto-save off (no data folder, or in tests).
    dir: Option<PathBuf>,
    /// Seconds between writes, [`INTERVAL`] but for tests.
    pub interval: f64,
    /// The document as last written, to skip writing it again unchanged.
    written: Option<Document>,
    /// This session's recovery file (or a restored one) is on disk.
    on_disk: bool,
    /// `egui` time of the last write, or of the first unsaved change since.
    since: Option<f64>,
    /// Waiting for an answer to "Recover unsaved work?".
    pub offer: Option<Found>,
}

impl AutoSave {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            interval: INTERVAL,
            written: None,
            on_disk: false,
            since: None,
            offer: None,
        }
    }

    /// The app's data folder, where eframe keeps its settings. Off in
    /// tests, so they never touch the real one.
    pub fn default_dir() -> Option<PathBuf> {
        if cfg!(test) {
            None
        } else {
            eframe::storage_dir("OpenDesign")
        }
    }

    /// Look for a recovery file left by a session that didn't quit
    /// normally, and offer it if it is newer than its project. A stale one
    /// (older than the project, or unreadable) is deleted.
    pub fn check(&mut self) {
        let Some(dir) = &self.dir else {
            return;
        };
        match find(dir) {
            Some(found) if is_newer(&found) => self.offer = Some(found),
            _ => remove(dir),
        }
    }

    /// Call once per frame: write the document if it has had unsaved
    /// changes for [`Self::interval`] seconds since the last write, or delete the
    /// recovery file once there's nothing unsaved. `now` is egui's time in
    /// seconds.
    pub fn tick(&mut self, s: &Session, files: &FileState, ctx: &egui::Context, now: f64) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        // Until the user answers, the old recovery file must stay.
        if self.offer.is_some() {
            return;
        }
        if !s.history.is_dirty() {
            if self.on_disk {
                remove(&dir);
                self.on_disk = false;
            }
            self.written = None;
            self.since = None;
            return;
        }
        let since = *self.since.get_or_insert(now);
        let due = since + self.interval;
        if now < due {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(due - now));
            return;
        }
        self.since = Some(now);
        if self.written.as_ref() == Some(&s.doc) {
            return;
        }
        // A failed write is retried next interval; there's nowhere useful
        // to report it without interrupting the user.
        if write(&dir, &s.doc, files.path.as_deref()).is_ok() {
            self.written = Some(s.doc.clone());
            self.on_disk = true;
        }
    }

    /// The app is quitting normally: whatever was unsaved was let go on
    /// purpose. Auto-save stays off from here on.
    pub fn quit(&mut self) {
        if let Some(dir) = self.dir.take() {
            remove(&dir);
        }
    }

    /// Restore the offered document as unsaved changes to its project.
    pub fn restore(&mut self, s: &mut Session, files: &mut FileState) {
        let Some(found) = self.offer.take() else {
            return;
        };
        s.load_document(found.doc);
        s.history.mark_unsaved();
        files.path = found.source.filter(|p| p.exists());
        // It stays on disk until this session overwrites or clears it.
        self.written = None;
        self.on_disk = true;
        self.since = None;
    }

    pub fn discard(&mut self) {
        self.offer = None;
        if let Some(dir) = &self.dir {
            remove(dir);
        }
    }

    /// The "Recover unsaved work?" dialog. Call once per frame.
    pub fn dialog(&mut self, s: &mut Session, files: &mut FileState, ctx: &egui::Context) {
        let Some(found) = &self.offer else {
            return;
        };
        let mut restore = None;
        egui::Modal::new(egui::Id::new("recover_unsaved_work")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Recover unsaved work?");
            let from = match &found.source {
                Some(path) => format!(
                    "OpenDesign didn't close normally while \"{}\" ({}) had unsaved changes.",
                    found.doc.name,
                    path.display()
                ),
                None => format!(
                    "OpenDesign didn't close normally while \"{}\" had unsaved changes.",
                    found.doc.name
                ),
            };
            ui.label(from);
            let pieces = match found.doc.patterns.len() {
                1 => "1 piece".to_owned(),
                n => format!("{n} pieces"),
            };
            ui.label(format!(
                "An auto-saved copy from {} with {pieces} can be restored.",
                ago(found.written)
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Restore").clicked() {
                    restore = Some(true);
                }
                if ui
                    .button("Discard")
                    .on_hover_text("Delete the auto-saved copy")
                    .clicked()
                {
                    restore = Some(false);
                }
            });
        });
        match restore {
            Some(true) => self.restore(s, files),
            Some(false) => self.discard(),
            None => {}
        }
    }
}

/// Write `doc` and where it came from to the recovery folder.
pub fn write(dir: &Path, doc: &Document, source: Option<&Path>) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let source = source.map_or(String::new(), |p| p.display().to_string());
    fs::write(dir.join(SOURCE_FILE), source)?;
    project::save(doc, &dir.join(DOCUMENT_FILE)).map_err(|e| match e {
        project::ProjectError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    })
}

/// The recovery file in `dir`, if there is a readable one.
pub fn find(dir: &Path) -> Option<Found> {
    let path = dir.join(DOCUMENT_FILE);
    let written = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    let doc = project::load(&path).ok()?;
    let source = fs::read_to_string(dir.join(SOURCE_FILE))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    Some(Found {
        doc,
        source,
        written,
    })
}

/// True if `found` holds work its project doesn't: the project was never
/// saved, is gone, or was last written before the recovery file.
fn is_newer(found: &Found) -> bool {
    let saved = found
        .source
        .as_deref()
        .and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    saved.is_none_or(|saved| found.written > saved)
}

/// Delete the recovery files in `dir`, if any.
pub fn remove(dir: &Path) {
    let _ = fs::remove_file(dir.join(DOCUMENT_FILE));
    let _ = fs::remove_file(dir.join(SOURCE_FILE));
}

/// "3 minutes ago" and the like.
fn ago(time: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(time)
        .map_or(0, |d| d.as_secs());
    match secs {
        0..=59 => "less than a minute ago".to_owned(),
        60..=119 => "a minute ago".to_owned(),
        120..=3599 => format!("{} minutes ago", secs / 60),
        3600..=7199 => "an hour ago".to_owned(),
        7200..=86399 => format!("{} hours ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use od_core::{AddPattern, Command, Vec2};
    use std::time::Duration;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("od_recovery_{}_{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn doc() -> Document {
        let mut doc = Document::new("Shirt");
        let outline = od_core::geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 50.0));
        AddPattern::new("Panel", outline).apply(&mut doc);
        doc
    }

    fn set_modified(path: &Path, time: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn written_recovery_is_found_with_its_source() {
        let dir = temp_dir("found");
        let project = dir.join("shirt.odproj");
        write(&dir, &doc(), Some(&project)).unwrap();
        let found = find(&dir).unwrap();
        assert_eq!(found.doc, doc());
        assert_eq!(found.source.as_deref(), Some(project.as_path()));

        write(&dir, &doc(), None).unwrap();
        assert_eq!(find(&dir).unwrap().source, None);

        remove(&dir);
        assert!(find(&dir).is_none());
    }

    #[test]
    fn only_recovery_newer_than_its_project_is_offered() {
        let dir = temp_dir("newer");
        let project = dir.join("shirt.odproj");
        project::save(&doc(), &project).unwrap();
        write(&dir, &doc(), Some(&project)).unwrap();
        let now = SystemTime::now();
        set_modified(&project, now - Duration::from_secs(600));
        set_modified(&dir.join(DOCUMENT_FILE), now);

        let mut auto = AutoSave::new(Some(dir.clone()));
        auto.check();
        assert!(auto.offer.is_some(), "newer than the project");

        // The project was saved after the crash copy: it's stale.
        set_modified(&project, now + Duration::from_secs(60));
        let mut auto = AutoSave::new(Some(dir.clone()));
        auto.check();
        assert!(auto.offer.is_none());
        assert!(find(&dir).is_none(), "stale recovery is deleted");
    }

    #[test]
    fn recovery_of_a_never_saved_document_is_always_offered() {
        let dir = temp_dir("unsaved");
        write(&dir, &doc(), None).unwrap();
        let mut auto = AutoSave::new(Some(dir.clone()));
        auto.check();
        assert_eq!(auto.offer.as_ref().unwrap().doc, doc());
        auto.discard();
        assert!(auto.offer.is_none());
        assert!(find(&dir).is_none());
    }

    #[test]
    fn unreadable_recovery_is_dropped() {
        let dir = temp_dir("broken");
        fs::write(dir.join(DOCUMENT_FILE), "{ not a project").unwrap();
        let mut auto = AutoSave::new(Some(dir.clone()));
        auto.check();
        assert!(auto.offer.is_none());
        assert!(!dir.join(DOCUMENT_FILE).exists());
    }

    #[test]
    fn no_folder_means_no_auto_save() {
        let mut auto = AutoSave::new(None);
        auto.check();
        auto.quit();
        assert!(auto.offer.is_none());
        assert!(
            AutoSave::default_dir().is_none(),
            "tests never use the real folder"
        );
    }

    #[test]
    fn describes_how_long_ago() {
        let now = SystemTime::now();
        assert_eq!(ago(now), "less than a minute ago");
        assert_eq!(ago(now - Duration::from_secs(300)), "5 minutes ago");
        assert_eq!(ago(now - Duration::from_secs(3 * 3600)), "3 hours ago");
    }
}
