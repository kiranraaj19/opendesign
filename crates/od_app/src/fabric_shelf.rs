//! Fabric files the user has saved or loaded (`.odfab`, see
//! [`od_core::fabric_file`]), listed in the Library so they can be put on
//! any pattern. Remembered between launches by path.

use std::path::{Path, PathBuf};

use od_core::{fabric_file, Fabric};

/// Storage key for the paths of loaded fabric files.
pub const FABRICS_KEY: &str = "fabric_files_v1";

#[derive(Default)]
pub struct FabricShelf {
    /// Loaded fabrics and the files they came from, in load order.
    pub fabrics: Vec<(PathBuf, Fabric)>,
    /// The last file error, shown in the Library until dismissed.
    pub error: Option<String>,
}

impl FabricShelf {
    /// The fabrics in `paths` that still load; others are dropped quietly,
    /// since a file moved away since last time isn't worth a message.
    pub fn from_paths(paths: &[PathBuf]) -> Self {
        let mut shelf = Self::default();
        for path in paths {
            if let Ok(fabric) = fabric_file::load(path) {
                shelf.put(path, fabric);
            }
        }
        shelf
    }

    /// The fabrics listed last time the app was closed.
    pub fn restore(storage: Option<&dyn eframe::Storage>) -> Self {
        let paths: Vec<PathBuf> = storage
            .and_then(|s| eframe::get_value(s, FABRICS_KEY))
            .unwrap_or_default();
        Self::from_paths(&paths)
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.fabrics.iter().map(|(p, _)| p.clone()).collect()
    }

    /// Add or refresh the fabric from `path`.
    fn put(&mut self, path: &Path, fabric: Fabric) {
        match self.fabrics.iter_mut().find(|(p, _)| p == path) {
            Some(entry) => entry.1 = fabric,
            None => self.fabrics.push((path.to_owned(), fabric)),
        }
    }

    /// Read the fabric file at `path` onto the shelf.
    pub fn load(&mut self, path: &Path) -> Option<Fabric> {
        match fabric_file::load(path) {
            Ok(fabric) => {
                self.put(path, fabric.clone());
                Some(fabric)
            }
            Err(e) => {
                self.error = Some(format!("Couldn't open {}: {e}", path.display()));
                None
            }
        }
    }

    /// Write `fabric` to `path`, and keep it on the shelf.
    pub fn save(&mut self, fabric: &Fabric, path: &Path) -> bool {
        match fabric_file::save(fabric, path) {
            Ok(()) => {
                self.put(path, fabric.clone());
                true
            }
            Err(e) => {
                self.error = Some(format!("Couldn't save {}: {e}", path.display()));
                false
            }
        }
    }

    /// Ask for a fabric file and load it.
    pub fn load_dialog(&mut self) -> Option<Fabric> {
        let path = dialog().pick_file()?;
        self.load(&path)
    }

    /// Ask where to save `fabric`, and save it there.
    pub fn save_dialog(&mut self, fabric: &Fabric) {
        let Some(path) = dialog()
            .set_file_name(format!("{}.{}", fabric.name, fabric_file::EXTENSION))
            .save_file()
        else {
            return;
        };
        let has_extension = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(fabric_file::EXTENSION));
        let path = if has_extension {
            path
        } else {
            path.with_extension(fabric_file::EXTENSION)
        };
        self.save(fabric, &path);
    }

    /// Take a fabric off the shelf (the file stays).
    pub fn forget(&mut self, index: usize) {
        if index < self.fabrics.len() {
            self.fabrics.remove(index);
        }
    }
}

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("OpenDesign fabric", &[fabric_file::EXTENSION])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_fabrics_go_on_the_shelf_and_reload_by_path() {
        let dir = std::env::temp_dir().join(format!("od_app_shelf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wool.odfab");
        let wool = Fabric {
            name: "Wool".into(),
            ..Fabric::denim()
        };
        let mut shelf = FabricShelf::default();
        assert!(shelf.save(&wool, &path));
        // Saving again replaces rather than duplicates.
        assert!(shelf.save(&wool, &path));
        assert_eq!(shelf.fabrics.len(), 1);

        let again = FabricShelf::from_paths(&[path.clone(), dir.join("gone.odfab")]);
        assert_eq!(again.fabrics, vec![(path.clone(), wool)]);
        assert_eq!(again.paths(), vec![path]);

        let mut shelf = FabricShelf::default();
        assert!(shelf.load(&dir.join("gone.odfab")).is_none());
        assert!(shelf.error.as_deref().unwrap().contains("gone.odfab"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
