//! Project files (`.odproj`): a versioned JSON wrapper around a [`Document`].
//!
//! ```json
//! { "format": "opendesign-project", "version": 1, "document": { ... } }
//! ```

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::{Document, PatternId};

pub const EXTENSION: &str = "odproj";
pub const FORMAT: &str = "opendesign-project";
/// Bump when the document layout changes incompatibly, and add a migration.
pub const VERSION: u32 = 1;

#[derive(Debug)]
pub enum ProjectError {
    Io(io::Error),
    /// The file is not valid JSON or does not match the document layout.
    Malformed(serde_json::Error),
    /// Valid JSON, but not an OpenDesign project.
    NotAProject,
    /// Written by a newer OpenDesign.
    UnsupportedVersion(u32),
    DuplicateId(PatternId),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Io(e) => write!(f, "{e}"),
            ProjectError::Malformed(e) => write!(f, "the file is damaged ({e})"),
            ProjectError::NotAProject => write!(f, "this is not an OpenDesign project"),
            ProjectError::UnsupportedVersion(v) => write!(
                f,
                "it was saved by a newer OpenDesign (format version {v}, this version reads {VERSION})"
            ),
            ProjectError::DuplicateId(id) => {
                write!(f, "the file is damaged (pattern id {} is used twice)", id.0)
            }
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<io::Error> for ProjectError {
    fn from(e: io::Error) -> Self {
        ProjectError::Io(e)
    }
}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    version: u32,
    document: &'a Document,
}

/// Read first, so the format and version can be checked before the body.
#[derive(Deserialize)]
struct Header {
    format: String,
    version: u32,
}

#[derive(Deserialize)]
struct FileIn {
    document: Document,
}

pub fn to_json(doc: &Document) -> String {
    let file = FileOut {
        format: FORMAT,
        version: VERSION,
        document: doc,
    };
    serde_json::to_string_pretty(&file).expect("documents always serialize")
}

pub fn from_json(text: &str) -> Result<Document, ProjectError> {
    let header: Header = serde_json::from_str(text).map_err(|e| {
        // Valid JSON without our header is someone else's file.
        if e.is_data() {
            ProjectError::NotAProject
        } else {
            ProjectError::Malformed(e)
        }
    })?;
    if header.format != FORMAT {
        return Err(ProjectError::NotAProject);
    }
    if header.version > VERSION {
        return Err(ProjectError::UnsupportedVersion(header.version));
    }
    let mut doc = serde_json::from_str::<FileIn>(text)
        .map_err(ProjectError::Malformed)?
        .document;
    doc.validate().map_err(ProjectError::DuplicateId)?;
    Ok(doc)
}

/// Write via a temporary file and rename, so a crash mid-save never leaves
/// a half-written project behind.
pub fn save(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
    fs::write(&tmp, to_json(doc))?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Document, ProjectError> {
    from_json(&fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, Command};
    use crate::geometry::{rectangle, Vec2};

    fn sample() -> Document {
        let mut doc = Document::new("Shirt");
        for name in ["Front", "Back"] {
            let mut add = AddPattern::new(name, rectangle(Vec2::ZERO, Vec2::new(500.0, 600.0)));
            add.apply(&mut doc);
        }
        doc
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("od_core_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn json_round_trip_keeps_everything() {
        let doc = sample();
        assert_eq!(from_json(&to_json(&doc)).unwrap(), doc);
    }

    #[test]
    fn loaded_document_keeps_issuing_fresh_ids() {
        let mut doc = from_json(&to_json(&sample())).unwrap();
        let new_id = doc.allocate_id();
        assert!(doc.patterns.iter().all(|p| p.id != new_id));
    }

    #[test]
    fn stale_next_id_is_repaired() {
        let text = to_json(&sample()).replace("\"next_id\": 3", "\"next_id\": 1");
        let mut doc = from_json(&text).unwrap();
        assert_eq!(doc.allocate_id(), PatternId(3));
    }

    #[test]
    fn rejects_files_it_cannot_read() {
        let good = to_json(&sample());
        assert!(matches!(
            from_json("{ not json"),
            Err(ProjectError::Malformed(_))
        ));
        assert!(matches!(
            from_json("{\"hello\": 1}"),
            Err(ProjectError::NotAProject)
        ));
        assert!(matches!(
            from_json(&good.replace(FORMAT, "something-else")),
            Err(ProjectError::NotAProject)
        ));
        assert!(matches!(
            from_json(&good.replace("\"version\": 1", "\"version\": 99")),
            Err(ProjectError::UnsupportedVersion(99))
        ));
        let dup = good.replace("\"id\": 2", "\"id\": 1");
        assert!(matches!(
            from_json(&dup),
            Err(ProjectError::DuplicateId(PatternId(1)))
        ));
    }

    #[test]
    fn save_and_load_through_disk() {
        let path = temp_path("shirt.odproj");
        let doc = sample();
        save(&doc, &path).unwrap();
        assert_eq!(load(&path).unwrap(), doc);
        assert!(
            !path.with_extension("odproj.tmp").exists(),
            "temp file cleaned up"
        );

        // Saving again overwrites in place.
        let mut renamed = doc.clone();
        renamed.name = "Shirt v2".into();
        save(&renamed, &path).unwrap();
        assert_eq!(load(&path).unwrap().name, "Shirt v2");
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = load(&temp_path("does-not-exist.odproj")).unwrap_err();
        assert!(matches!(err, ProjectError::Io(ref e) if e.kind() == io::ErrorKind::NotFound));
    }
}
