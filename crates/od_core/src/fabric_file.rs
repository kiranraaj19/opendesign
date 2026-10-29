//! Fabric files (`.odfab`): one [`Fabric`] in a versioned JSON wrapper,
//! so a fabric can be kept and reused across projects.
//!
//! ```json
//! { "format": "opendesign-fabric", "version": 1, "fabric": { ... } }
//! ```

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::Fabric;

pub const EXTENSION: &str = "odfab";
pub const FORMAT: &str = "opendesign-fabric";
pub const VERSION: u32 = 1;

#[derive(Debug)]
pub enum FabricFileError {
    Io(io::Error),
    /// Not valid JSON, or not a fabric.
    Malformed(serde_json::Error),
    /// Valid JSON, but not an OpenDesign fabric file.
    NotAFabric,
    /// Written by a newer OpenDesign.
    UnsupportedVersion(u32),
}

impl fmt::Display for FabricFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FabricFileError::Io(e) => write!(f, "{e}"),
            FabricFileError::Malformed(e) => write!(f, "the file is damaged ({e})"),
            FabricFileError::NotAFabric => write!(f, "this is not an OpenDesign fabric"),
            FabricFileError::UnsupportedVersion(v) => write!(
                f,
                "it was saved by a newer OpenDesign (fabric version {v}, this version reads {VERSION})"
            ),
        }
    }
}

impl std::error::Error for FabricFileError {}

impl From<io::Error> for FabricFileError {
    fn from(e: io::Error) -> Self {
        FabricFileError::Io(e)
    }
}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    version: u32,
    fabric: &'a Fabric,
}

#[derive(Deserialize)]
struct FileIn {
    format: String,
    version: u32,
    fabric: serde_json::Value,
}

/// `fabric` as the text of a fabric file.
pub fn to_string(fabric: &Fabric) -> String {
    serde_json::to_string_pretty(&FileOut {
        format: FORMAT,
        version: VERSION,
        fabric,
    })
    .expect("fabric serializes")
}

/// The fabric in the text of a fabric file.
pub fn from_str(text: &str) -> Result<Fabric, FabricFileError> {
    let file: FileIn = serde_json::from_str(text).map_err(|e| {
        if e.is_data() {
            FabricFileError::NotAFabric
        } else {
            FabricFileError::Malformed(e)
        }
    })?;
    if file.format != FORMAT {
        return Err(FabricFileError::NotAFabric);
    }
    if file.version > VERSION {
        return Err(FabricFileError::UnsupportedVersion(file.version));
    }
    serde_json::from_value(file.fabric).map_err(FabricFileError::Malformed)
}

pub fn save(fabric: &Fabric, path: &Path) -> Result<(), FabricFileError> {
    fs::write(path, to_string(fabric))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Fabric, FabricFileError> {
    from_str(&fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::FabricValue;

    #[test]
    fn a_fabric_file_round_trips() {
        let fabric = Fabric {
            name: "Stiff linen".into(),
            ..FabricValue::StretchWeft.set(&Fabric::denim(), 30.0)
        };
        let text = to_string(&fabric);
        assert!(text.contains("\"format\": \"opendesign-fabric\""));
        assert_eq!(from_str(&text).unwrap(), fabric);

        let dir = std::env::temp_dir().join(format!("od_fabric_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("linen.odfab");
        save(&fabric, &path).unwrap();
        assert_eq!(load(&path).unwrap(), fabric);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(matches!(
            from_str("not json"),
            Err(FabricFileError::Malformed(_))
        ));
        assert!(matches!(
            from_str(r#"{"format":"opendesign-project","version":1,"fabric":{}}"#),
            Err(FabricFileError::NotAFabric)
        ));
        assert!(matches!(
            from_str("[1, 2]"),
            Err(FabricFileError::NotAFabric)
        ));
        let newer = to_string(&Fabric::silk()).replace("\"version\": 1", "\"version\": 99");
        assert!(matches!(
            from_str(&newer),
            Err(FabricFileError::UnsupportedVersion(99))
        ));
        assert!(load(Path::new("/no/such/file.odfab")).is_err());
    }

    #[test]
    fn a_fabric_written_without_directions_loads() {
        let text = r#"{"format":"opendesign-fabric","version":1,"fabric":{"name":"Old",
            "mass_per_area":0.2,"stretch_compliance":3e-7,"bending_compliance":2e-5,
            "friction":0.3,"color":[1,2,3],"texture":"flat"}}"#;
        let f = from_str(text).unwrap();
        assert!(f.is_isotropic());
    }
}
