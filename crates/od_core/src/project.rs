//! Project files (`.odproj`): a versioned JSON wrapper around a [`Document`].
//!
//! ```json
//! { "format": "opendesign-project", "version": 11, "document": { ... } }
//! ```
//!
//! Older versions are upgraded on load by the `migrate_*` steps below, so a
//! file is always saved in the current version.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::{Document, Invalid, PatternId, SewingLineId, TackId};

pub const EXTENSION: &str = "odproj";
pub const FORMAT: &str = "opendesign-project";
/// Bump when the document layout changes incompatibly, and add a migration.
///
/// - 1: outlines are plain point lists.
/// - 2: outline points carry a kind (corner or curve); patterns gain seam
///   allowance, grainline, notches and internal lines.
/// - 3: the document gains sewing lines.
/// - 4: the document gains tacks.
/// - 5: a sewing line's two sides are each a group of edges (one-to-many
///   seams), not one edge each; edge parameters are fractional (free
///   sewing), not whole outline indices.
/// - 6: patterns gain a fabric (mass, stiffness, friction, colour/weave).
/// - 7: patterns gain topstitch lines and point trims (buttons, snaps).
/// - 8: the document gains a size chart; patterns gain a grading rule.
/// - 9: seam edges gain a sewing direction; tacks hold a point anywhere on
///   a pattern (not an outline point index) to another pattern or the
///   avatar.
/// - 10: patterns gain an optional particle distance for simulation; a
///   tack can hold a point on the avatar's surface (a triangle and
///   barycentric weights) that follows the body when it is resized.
/// - 11: patterns can be hidden or locked, and keep a 3D arrangement.
pub const VERSION: u32 = 12;

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
    /// A seam sews a pattern that isn't in the file, or is malformed.
    BadSewingLine(SewingLineId),
    /// A tack holds a pattern that isn't in the file.
    BadTack(TackId),
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
            ProjectError::BadSewingLine(id) => {
                write!(f, "the file is damaged (seam {} sews a missing pattern)", id.0)
            }
            ProjectError::BadTack(id) => {
                write!(f, "the file is damaged (tack {} holds a missing pattern)", id.0)
            }
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<Invalid> for ProjectError {
    fn from(e: Invalid) -> Self {
        match e {
            Invalid::DuplicatePattern(id) => ProjectError::DuplicateId(id),
            Invalid::BadSewingLine(id) => ProjectError::BadSewingLine(id),
            Invalid::BadTack(id) => ProjectError::BadTack(id),
        }
    }
}

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

/// Upgrade a version 4 document to version 5 in place: each sewing line's
/// `a` and `b` become one-edge groups (whole-number outline indices read
/// back fine as the new fractional parameters, with no change needed).
fn migrate_v4(doc: &mut serde_json::Value) {
    use serde_json::{json, Value};
    let Some(lines) = doc.get_mut("sewing_lines").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for line in lines.iter_mut().filter_map(Value::as_object_mut) {
        for side in ["a", "b"] {
            if let Some(edge) = line.remove(side) {
                line.insert(side.into(), json!([edge]));
            }
        }
    }
}

/// Upgrade a version 5 document to version 6 in place: every pattern gets
/// the default fabric (plain cotton), since nothing had one yet.
fn migrate_v5(doc: &mut serde_json::Value) {
    use serde_json::Value;
    let default_fabric =
        serde_json::to_value(crate::document::Fabric::default()).expect("fabric serializes");
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        pattern.insert("fabric".into(), default_fabric.clone());
    }
}

/// Upgrade a version 6 document to version 7 in place: no pattern had
/// topstitches or trims yet.
fn migrate_v6(doc: &mut serde_json::Value) {
    use serde_json::{json, Value};
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        pattern.insert("topstitches".into(), json!([]));
        pattern.insert("trims".into(), json!([]));
    }
}

/// Upgrade a version 7 document to version 8 in place: the standard S/M/L/XL
/// chart, and no pattern had a grading rule yet (same outline at every size
/// until one is set).
fn migrate_v7(doc: &mut serde_json::Value) {
    use serde_json::{json, Value};
    let default_chart =
        serde_json::to_value(crate::document::SizeChart::default()).expect("chart serializes");
    if let Some(object) = doc.as_object_mut() {
        object.insert("size_chart".into(), default_chart);
    }
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        pattern.insert("grading".into(), json!({ "point_deltas": [] }));
    }
}

/// Upgrade a version 8 document to version 9 in place. Every seam edge was
/// sewn forwards. A tack held two outline points by index; it now holds
/// their positions, and its second end becomes a pattern target. A tack
/// whose point no longer exists is dropped.
fn migrate_v8(doc: &mut serde_json::Value) {
    use serde_json::{json, Value};
    if let Some(lines) = doc.get_mut("sewing_lines").and_then(Value::as_array_mut) {
        for line in lines.iter_mut().filter_map(Value::as_object_mut) {
            for side in ["a", "b"] {
                let edges = line.get_mut(side).and_then(Value::as_array_mut);
                for edge in edges.into_iter().flatten().filter_map(Value::as_object_mut) {
                    edge.insert("forward".into(), json!(true));
                }
            }
        }
    }
    let outline_point = |doc: &Value, point: &Value| -> Option<Value> {
        let pattern = point.get("pattern")?.as_u64()?;
        let index = point.get("index")?.as_u64()? as usize;
        let found = doc
            .get("patterns")?
            .as_array()?
            .iter()
            .find(|p| p.get("id").and_then(Value::as_u64) == Some(pattern))?;
        let pos = found.get("outline")?.as_array()?.get(index)?;
        Some(json!({ "pattern": pattern, "at": { "x": pos.get("x")?, "y": pos.get("y")? } }))
    };
    let Some(tacks) = doc.get("tacks").and_then(Value::as_array).cloned() else {
        return; // leave it for deserialisation to report
    };
    let upgraded: Vec<Value> = tacks
        .iter()
        .filter_map(|tack| {
            let a = outline_point(doc, tack.get("a")?)?;
            let b = outline_point(doc, tack.get("b")?)?;
            Some(json!({ "id": tack.get("id")?, "a": a, "b": { "pattern": b } }))
        })
        .collect();
    if let Some(object) = doc.as_object_mut() {
        object.insert("tacks".into(), Value::Array(upgraded));
    }
}

/// Upgrade a version 9 document to version 10 in place: every pattern
/// simulates at the default particle distance. Tacks keep their format.
fn migrate_v9(doc: &mut serde_json::Value) {
    use serde_json::Value;
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        pattern.insert("particle_distance".into(), Value::Null);
    }
}

/// Upgrade a version 10 document to version 11 in place: every pattern is
/// shown, unlocked and placed automatically.
fn migrate_v10(doc: &mut serde_json::Value) {
    use serde_json::Value;
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        pattern.insert("hidden".into(), Value::Bool(false));
        pattern.insert("locked".into(), Value::Bool(false));
        pattern.insert("arrangement".into(), Value::Null);
    }
}

/// Upgrade a version 11 document to version 12 in place: no pattern has
/// darts inside it.
fn migrate_v11(doc: &mut serde_json::Value) {
    use serde_json::Value;
    let Some(patterns) = doc.get_mut("patterns").and_then(Value::as_array_mut) else {
        return; // leave it for deserialisation to report
    };
    for pattern in patterns.iter_mut().filter_map(Value::as_object_mut) {
        pattern.insert("darts".into(), Value::Array(Vec::new()));
    }
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
    if header.version < 5 {
        migrate_v4(&mut body);
    }
    if header.version < 6 {
        migrate_v5(&mut body);
    }
    if header.version < 7 {
        migrate_v6(&mut body);
    }
    if header.version < 8 {
        migrate_v7(&mut body);
    }
    if header.version < 9 {
        migrate_v8(&mut body);
    }
    if header.version < 10 {
        migrate_v9(&mut body);
    }
    if header.version < 11 {
        migrate_v10(&mut body);
    }
    if header.version < 12 {
        migrate_v11(&mut body);
    }
    let mut doc: Document = serde_json::from_value(body).map_err(ProjectError::Malformed)?;
    doc.validate()?;
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
            from_json(&good.replace("\"version\": 11", "\"version\": 99")),
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
        assert!(text.contains("\"version\": 11"));
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
        assert!(resaved.contains("\"version\": 11"));
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
            vec![crate::document::SeamEdge {
                pattern: id,
                start: 0.0,
                end: 1.0,
                forward: true,
            }],
            vec![crate::document::SeamEdge {
                pattern: id,
                start: 2.0,
                end: 3.0,
                forward: true,
            }],
        );
        sew.apply(&mut doc);
        assert_eq!(doc.sewing_lines.len(), 1);

        // Saving writes the current version, which loads back the same.
        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 11"));
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
            crate::document::TackPoint {
                pattern: id,
                at: Vec2::ZERO,
            },
            crate::document::TackTarget::Avatar([0.0, 1.0, 0.1]),
        );
        tack.apply(&mut doc);
        assert_eq!(doc.tacks.len(), 1);

        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 11"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    /// A file exactly as the tacks commit (format version 4) wrote it: one
    /// sewing line with a single edge on each side, in the old shape.
    const V4_FILE: &str = r#"{
  "format": "opendesign-project",
  "version": 4,
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
    "sewing_lines": [
      { "id": 1, "a": { "pattern": 4, "start": 0, "end": 1 }, "b": { "pattern": 4, "start": 2, "end": 3 } }
    ],
    "tacks": [],
    "next_id": 5,
    "next_sewing_id": 2,
    "next_tack_id": 1
  }
}"#;

    #[test]
    fn version_4_files_load_each_side_as_a_one_edge_group() {
        use crate::document::SeamEdge;
        let doc = from_json(V4_FILE).unwrap();
        let id = doc.patterns[0].id;
        assert_eq!(doc.sewing_lines.len(), 1);
        assert_eq!(
            doc.sewing_lines[0].a,
            vec![SeamEdge {
                pattern: id,
                start: 0.0,
                end: 1.0,
                forward: true,
            }]
        );
        assert_eq!(
            doc.sewing_lines[0].b,
            vec![SeamEdge {
                pattern: id,
                start: 2.0,
                end: 3.0,
                forward: true,
            }]
        );

        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 11"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    /// A file exactly as the one-to-many-seams commit (format version 5)
    /// wrote it: no pattern has a fabric, topstitches or trims yet.
    const V5_FILE: &str = r#"{
  "format": "opendesign-project",
  "version": 5,
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
    "tacks": [],
    "next_id": 5,
    "next_sewing_id": 1,
    "next_tack_id": 1
  }
}"#;

    #[test]
    fn version_5_files_load_with_the_default_fabric() {
        let doc = from_json(V5_FILE).unwrap();
        assert_eq!(doc.patterns[0].fabric, crate::document::Fabric::cotton());

        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 11"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    #[test]
    fn version_5_files_load_with_no_topstitches_or_trims() {
        use crate::document::TrimKind;
        let mut doc = from_json(V5_FILE).unwrap();
        assert!(doc.patterns[0].topstitches.is_empty());
        assert!(doc.patterns[0].trims.is_empty());

        let id = doc.patterns[0].id;
        crate::command::AddTopstitch::new(id, vec![Vec2::new(10.0, 10.0), Vec2::new(90.0, 10.0)])
            .apply(&mut doc);
        crate::command::AddTrim::new(id, TrimKind::Button, Vec2::new(50.0, 50.0)).apply(&mut doc);
        assert_eq!(doc.patterns[0].topstitches.len(), 1);
        assert_eq!(doc.patterns[0].trims.len(), 1);

        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 11"));
        assert_eq!(from_json(&resaved).unwrap(), doc);
    }

    /// A file exactly as the topstitch/trims commit (format version 7)
    /// wrote it: no size chart, and no pattern has a grading rule yet.
    const V7_FILE: &str = r#"{
  "format": "opendesign-project",
  "version": 7,
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
        "internal_lines": [],
        "fabric": {
          "name": "Cotton",
          "mass_per_area": 0.15,
          "stretch_compliance": 1e-7,
          "bending_compliance": 1e-5,
          "friction": 0.3,
          "color": [235, 230, 210],
          "texture": "plain"
        },
        "topstitches": [],
        "trims": []
      }
    ],
    "sewing_lines": [],
    "tacks": [],
    "next_id": 5,
    "next_sewing_id": 1,
    "next_tack_id": 1
  }
}"#;

    #[test]
    fn version_7_files_load_with_the_standard_size_chart_and_no_grading() {
        let doc = from_json(V7_FILE).unwrap();
        assert_eq!(doc.size_chart, crate::document::SizeChart::standard());
        assert_eq!(doc.patterns[0].grading, crate::document::Grading::default());
        // Ungraded, so every size looks the same as the base.
        assert_eq!(doc.patterns[0].graded_outline(2), doc.patterns[0].outline);

        let id = doc.patterns[0].id;
        let mut doc = doc;
        crate::command::SetGrading::new(
            id,
            crate::document::Grading::uniform_scale(&doc.pattern(id).unwrap().outline, 5.0),
        )
        .apply(&mut doc);
        assert_ne!(doc.patterns[0].graded_outline(1), doc.patterns[0].outline);

        let resaved = to_json(&doc);
        assert!(resaved.contains("\"version\": 11"));
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

    #[test]
    fn version_8_files_get_forward_edges_and_positional_tacks() {
        use crate::command::AddSewingLine;
        use crate::document::{SeamEdge, TackPoint, TackTarget};
        let mut doc = sample();
        let (a, b) = (doc.patterns[0].id, doc.patterns[1].id);
        let edge = |pattern| SeamEdge {
            pattern,
            start: 1.0,
            end: 2.0,
            forward: true,
        };
        AddSewingLine::new(vec![edge(a)], vec![edge(b)]).apply(&mut doc);
        // As version 8 wrote it: no direction, tacks by outline index.
        let mut file: serde_json::Value = serde_json::from_str(&to_json(&doc)).unwrap();
        file["version"] = 8.into();
        let body = &mut file["document"];
        for side in ["a", "b"] {
            body["sewing_lines"][0][side][0]
                .as_object_mut()
                .unwrap()
                .remove("forward");
        }
        body["tacks"] = serde_json::json!([
            { "id": 1, "a": { "pattern": a.0, "index": 2 }, "b": { "pattern": b.0, "index": 0 } },
            { "id": 2, "a": { "pattern": a.0, "index": 9 }, "b": { "pattern": b.0, "index": 0 } }
        ]);
        body["next_tack_id"] = 3.into();

        let loaded = from_json(&file.to_string()).unwrap();
        assert_eq!(loaded.sewing_lines, doc.sewing_lines, "edges sewn forwards");
        // The tack holds the same outline points, now by position; the one
        // pointing past the outline is dropped.
        assert_eq!(loaded.tacks.len(), 1);
        assert_eq!(
            loaded.tacks[0].a,
            TackPoint {
                pattern: a,
                at: Vec2::new(500.0, 600.0)
            }
        );
        assert_eq!(
            loaded.tacks[0].b,
            TackTarget::Pattern(TackPoint {
                pattern: b,
                at: Vec2::ZERO
            })
        );
    }

    #[test]
    fn version_9_files_load_with_the_default_particle_distance() {
        let mut doc = sample();
        doc.patterns[0].particle_distance = Some(12.0);
        let mut file: serde_json::Value = serde_json::from_str(&to_json(&doc)).unwrap();
        file["version"] = 9.into();
        for pattern in file["document"]["patterns"].as_array_mut().unwrap() {
            pattern.as_object_mut().unwrap().remove("particle_distance");
        }
        let loaded = from_json(&file.to_string()).unwrap();
        assert!(loaded
            .patterns
            .iter()
            .all(|p| p.particle_distance.is_none()));

        // The current version keeps it.
        let kept = from_json(&to_json(&doc)).unwrap();
        assert_eq!(kept.patterns[0].particle_distance, Some(12.0));
    }

    #[test]
    fn version_10_files_load_shown_unlocked_and_placed_automatically() {
        use crate::document::Arrangement;
        let mut doc = sample();
        doc.patterns[0].hidden = true;
        doc.patterns[0].locked = true;
        doc.patterns[0].arrangement = Some(Arrangement {
            center: [0.0, 1.2, 0.3],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
        });
        let mut file: serde_json::Value = serde_json::from_str(&to_json(&doc)).unwrap();
        file["version"] = 10.into();
        for pattern in file["document"]["patterns"].as_array_mut().unwrap() {
            let pattern = pattern.as_object_mut().unwrap();
            for key in ["hidden", "locked", "arrangement"] {
                pattern.remove(key);
            }
        }
        let loaded = from_json(&file.to_string()).unwrap();
        assert!(loaded
            .patterns
            .iter()
            .all(|p| !p.hidden && !p.locked && p.arrangement.is_none()));

        // The current version keeps them.
        assert_eq!(from_json(&to_json(&doc)).unwrap(), doc);
    }

    #[test]
    fn version_11_files_load_with_no_darts() {
        use crate::document::Dart;
        let mut doc = sample();
        doc.patterns[0].darts.push(Dart {
            start: Vec2::new(10.0, 10.0),
            end: Vec2::new(10.0, 60.0),
            width: 20.0,
        });
        let mut file: serde_json::Value = serde_json::from_str(&to_json(&doc)).unwrap();
        file["version"] = 11.into();
        for pattern in file["document"]["patterns"].as_array_mut().unwrap() {
            pattern.as_object_mut().unwrap().remove("darts");
        }
        let loaded = from_json(&file.to_string()).unwrap();
        assert!(loaded.patterns.iter().all(|p| p.darts.is_empty()));

        // The current version keeps them.
        assert_eq!(from_json(&to_json(&doc)).unwrap(), doc);
    }

    #[test]
    fn seams_and_tacks_on_missing_patterns_are_rejected() {
        use crate::command::{AddSewingLine, AddTack};
        use crate::document::{SeamEdge, TackPoint, TackTarget};
        let mut doc = sample();
        let a = doc.patterns[0].id;
        let edge = SeamEdge {
            pattern: a,
            start: 0.0,
            end: 1.0,
            forward: true,
        };
        AddSewingLine::new(vec![edge], vec![edge]).apply(&mut doc);
        let mut bad: serde_json::Value = serde_json::from_str(&to_json(&doc)).unwrap();
        bad["document"]["sewing_lines"][0]["b"][0]["pattern"] = 99.into();
        assert!(matches!(
            from_json(&bad.to_string()),
            Err(ProjectError::BadSewingLine(_))
        ));

        let mut doc = sample();
        let point = TackPoint {
            pattern: doc.patterns[0].id,
            at: Vec2::ZERO,
        };
        AddTack::new(point, TackTarget::Avatar([0.0; 3])).apply(&mut doc);
        let mut bad: serde_json::Value = serde_json::from_str(&to_json(&doc)).unwrap();
        bad["document"]["tacks"][0]["a"]["pattern"] = 99.into();
        assert!(matches!(
            from_json(&bad.to_string()),
            Err(ProjectError::BadTack(_))
        ));
        assert_eq!(from_json(&to_json(&doc)).unwrap(), doc);
    }
}
