//! Project files (`.odproj`): a versioned JSON wrapper around a [`Document`].
//!
//! ```json
//! { "format": "opendesign-project", "version": 4, "document": { ... } }
//! ```
//!
//! Older versions are upgraded on load by the `migrate_*` steps below, so a
//! file is always saved in the current version.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::{Document, PatternId};

pub const EXTENSION: &str = "odproj";
pub const FORMAT: &str = "opendesign-project";
/// Bump when the document layout changes incompatibly, and add a migration.
///
/// - 1: outlines are plain point lists.
/// - 2: outline points carry a kind (corner or curve); patterns gain seam
///   allowance, grainline, notches and internal lines.
/// - 3: the document gains sewing lines.
/// - 4: the document gains tacks.
pub const VERSION: u32 = 4;

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

/// Upgrade a version 1 document to version 2 in place.
fn migrate_v1(doc: &mut serde_json::Value) {
    use serde_json::{json, Value};
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        if let Some(outline) = pattern.get_mut("outline").and_then(Value::as_array_mut) {
            for point in outline.iter_mut().filter_map(Value::as_object_mut) {
                point.insert("kind".into(), json!("corner"));
            }
        }
        // Old patterns had no allowance; keep them looking as they did.
        pattern.insert("seam_allowance".into(), json!(0.0));
        pattern.insert("grain_angle".into(), json!(0.0));
        pattern.insert("notches".into(), json!([]));
        pattern.insert("internal_lines".into(), json!([]));
    }
}

/// Upgrade a version 2 document to version 3 in place: nothing was sewn yet.
fn migrate_v2(doc: &mut serde_json::Value) {
    use serde_json::json;
    let Some(object) = doc.as_object_mut() else {
        return; // leave it for deserialisation to report
    };
    object.insert("sewing_lines".into(), json!([]));
    object.insert("next_sewing_id".into(), json!(1));
}

/// Upgrade a version 3 document to version 4 in place: nothing was tacked
/// yet.
fn migrate_v3(doc: &mut serde_json::Value) {
    use serde_json::json;
    let Some(object) = doc.as_object_mut() else {
        return; // leave it for deserialisation to report
    };
    object.insert("tacks".into(), json!([]));
    object.insert("next_tack_id".into(), json!(1));
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
    let mut file: serde_json::Value =
        serde_json::from_str(text).map_err(ProjectError::Malformed)?;
    let mut body = file
        .get_mut("document")
        .map(serde_json::Value::take)
        .unwrap_or_default();
    if header.version < 2 {
        migrate_v1(&mut body);
    }
    if header.version < 3 {
        migrate_v2(&mut body);
    }
    if header.version < 4 {
        migrate_v3(&mut body);
    }
    let mut doc: Document = serde_json::from_value(body).map_err(ProjectError::Malformed)?;
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
            from_json(&good.replace("\"version\": 4", "\"version\": 99")),
            Err(ProjectError::UnsupportedVersion(99))
        ));
        let dup = good.replace("\"id\": 2", "\"id\": 1");
        assert!(matches!(
            from_json(&dup),
            Err(ProjectError::DuplicateId(PatternId(1)))
        ));
    }

    #[test]
    fn curves_and_markings_round_trip() {
        use crate::command::{AddInternalLine, AddNotch, SetPointKind};
        use crate::document::PointKind;
        let mut doc = sample();
        let id = doc.patterns[0].id;
        SetPointKind::new(id, 1, PointKind::Curve).apply(&mut doc);
        AddNotch::new(id, 250.0).apply(&mut doc);
        AddInternalLine::new(id, vec![Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)]).apply(&mut doc);
        doc.patterns[0].grain_angle = 45.0;
        doc.patterns[0].seam_allowance = 7.5;

        let text = to_json(&doc);
        assert!(text.contains("\"version\": 4"));
        assert!(text.contains("\"kind\": \"curve\""));
        assert_eq!(from_json(&text).unwrap(), doc);
    }

    /// A file exactly as Phase 2 (format version 1) wrote it.
    const V1_FILE: &str = r#"{
  "format": "opendesign-project",
  "version": 1,
  "document": {
    "name": "Old shirt",
    "patterns": [
      {
        "id": 4,
        "name": "Front",
        "outline": [
          { "x": 0.0, "y": 0.0 },
          { "x": 500.0, "y": 0.0 },
          { "x": 500.0, "y": 600.0 },
          { "x": 0.0, "y": 600.0 }
        ]
      }
    ],
    "next_id": 5
  }
}"#;

    #[test]
    fn version_1_files_load_as_corner_outlines() {
        use crate::document::{OutlinePoint, Pattern};
        let mut doc = from_json(V1_FILE).unwrap();
        assert_eq!(doc.name, "Old shirt");
        let expected = Pattern {
            seam_allowance: 0.0,
            ..Pattern::new(
                PatternId(4),
                "Front",
                rectangle(Vec2::ZERO, Vec2::new(500.0, 600.0))
                    .into_iter()
                    .map(OutlinePoint::corner)
                    .collect(),
            )
        };
        assert_eq!(doc.patterns, [expected]);
        assert_eq!(doc.patterns[0].area(), 300_000.0);
        assert_eq!(doc.allocate_id(), PatternId(5));

        // Saving writes the current version, which loads back the same.
        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 4"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    /// A file exactly as Phase 3/4 (format version 2) wrote it: no sewing.
    const V2_FILE: &str = r#"{
  "format": "opendesign-project",
  "version": 2,
  "document": {
    "name": "Old shirt",
    "patterns": [
      {
        "id": 4,
        "name": "Front",
        "outline": [
          { "x": 0.0, "y": 0.0, "kind": "corner" },
          { "x": 500.0, "y": 0.0, "kind": "corner" },
          { "x": 500.0, "y": 600.0, "kind": "corner" },
          { "x": 0.0, "y": 600.0, "kind": "corner" }
        ],
        "seam_allowance": 10.0,
        "grain_angle": 0.0,
        "notches": [],
        "internal_lines": []
      }
    ],
    "next_id": 5
  }
}"#;

    #[test]
    fn version_2_files_load_with_no_sewing_lines() {
        let mut doc = from_json(V2_FILE).unwrap();
        assert_eq!(doc.name, "Old shirt");
        assert!(doc.sewing_lines.is_empty());

        // A seam added after loading gets a fresh id, not one colliding
        // with anything the old file might have implied.
        let id = doc.patterns[0].id;
        let mut sew = crate::command::AddSewingLine::new(
            crate::document::SeamEdge {
                pattern: id,
                start: 0,
                end: 1,
            },
            crate::document::SeamEdge {
                pattern: id,
                start: 2,
                end: 3,
            },
        );
        sew.apply(&mut doc);
        assert_eq!(doc.sewing_lines.len(), 1);

        // Saving writes the current version, which loads back the same.
        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 4"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    /// A file exactly as the first Phase 5 commit (format version 3) wrote
    /// it: sewing, but no tacks.
    const V3_FILE: &str = r#"{
  "format": "opendesign-project",
  "version": 3,
  "document": {
    "name": "Old shirt",
    "patterns": [
      {
        "id": 4,
        "name": "Front",
        "outline": [
          { "x": 0.0, "y": 0.0, "kind": "corner" },
          { "x": 500.0, "y": 0.0, "kind": "corner" },
          { "x": 500.0, "y": 600.0, "kind": "corner" },
          { "x": 0.0, "y": 600.0, "kind": "corner" }
        ],
        "seam_allowance": 10.0,
        "grain_angle": 0.0,
        "notches": [],
        "internal_lines": []
      }
    ],
    "sewing_lines": [],
    "next_id": 5,
    "next_sewing_id": 1
  }
}"#;

    #[test]
    fn version_3_files_load_with_no_tacks() {
        let mut doc = from_json(V3_FILE).unwrap();
        assert!(doc.tacks.is_empty());

        let id = doc.patterns[0].id;
        let mut tack = crate::command::AddTack::new(
            crate::document::SeamPoint {
                pattern: id,
                index: 0,
            },
            crate::document::SeamPoint {
                pattern: id,
                index: 2,
            },
        );
        tack.apply(&mut doc);
        assert_eq!(doc.tacks.len(), 1);

        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 4"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    #[test]
    fn version_1_problems_are_still_reported() {
        let extra = V1_FILE.replace("\"next_id\": 5", "\"next_id\": 5, \"extra\": 1");
        assert!(from_json(&extra).is_ok(), "unknown fields are ignored");
        let broken = V1_FILE.replace("\"x\": 500.0", "\"x\": \"wide\"");
        assert!(matches!(
            from_json(&broken),
            Err(ProjectError::Malformed(_))
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
