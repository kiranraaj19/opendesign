//! Proof-of-concept plugin hook: run a user-supplied Python script to
//! validate a pattern.
//!
//! This is deliberately one narrow extension point, not a general plugin
//! API: a validation script gets a pattern's name, point count and seam
//! allowance, and returns a message string (empty = no issues). That's
//! enough to prove a Python script can be invoked from `od_core` and
//! change what the app reports, without committing to a stable plugin
//! ABI, a sandboxing story, or a packaging format for third-party
//! plugins — those are real problems for a later phase, not this MVP.
//!
//! Gated behind the `python-plugins` feature (off by default) because it
//! pulls in `pyo3`, which embeds a Python interpreter: that's a new
//! runtime requirement (a `python3` dev install, found via `pyo3`'s
//! `auto-initialize`) that most contributors and CI jobs building
//! `od_core` don't need. With the feature off, nothing in this module is
//! compiled, so it doesn't affect `cargo test --workspace`,
//! `cargo clippy --workspace`, or the Windows cross-check
//! (`--target x86_64-pc-windows-gnu`) — pyo3's Windows linking needs
//! (a matching `python3.dll`/import lib) are exactly the kind of
//! cross-compile complication that check is meant to catch, so this
//! stays opt-in until there's a packaging story for it on Windows too.

use std::path::Path;

use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::document::Pattern;

/// Run `script_path`'s `validate(name, point_count, seam_allowance_mm)`
/// Python function against `pattern` and return what it returned.
///
/// The script is loaded fresh each call (no caching, no persistent
/// interpreter state between patterns) — simplicity over throughput,
/// fine for "run this on save" or "run this from a menu item" use.
pub fn run_validation_script(script_path: &Path, pattern: &Pattern) -> PyResult<String> {
    let code = std::fs::read_to_string(script_path).map_err(|err| {
        PyErr::new::<pyo3::exceptions::PyIOError, _>(format!(
            "reading {}: {err}",
            script_path.display()
        ))
    })?;
    let code = std::ffi::CString::new(code)
        .map_err(|err| PyErr::new::<pyo3::exceptions::PyValueError, _>(err.to_string()))?;
    let file_name = std::ffi::CString::new(script_path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| std::ffi::CString::new("plugin.py").unwrap());
    let module_name = std::ffi::CString::new("od_plugin").unwrap();

    Python::with_gil(|py| {
        let module = PyModule::from_code(py, &code, &file_name, &module_name)?;
        let validate = module.getattr("validate")?;
        let result = validate.call1((
            pattern.name.clone(),
            pattern.outline.len(),
            pattern.seam_allowance,
        ))?;
        result.extract::<String>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{OutlinePoint, PatternId};

    fn example_script() -> std::path::PathBuf {
        // Checked in at the workspace root so both the crate's own tests
        // and a user poking at the feature from the CLI can find it.
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/plugins/validate_pattern.py")
    }

    #[test]
    fn runs_the_example_script_and_gets_its_message() {
        let pattern = Pattern::new(
            PatternId(1),
            "Sleeve",
            vec![
                OutlinePoint::corner(crate::geometry::Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(crate::geometry::Vec2::new(100.0, 0.0)),
                OutlinePoint::corner(crate::geometry::Vec2::new(100.0, 100.0)),
            ],
        );
        let message = run_validation_script(&example_script(), &pattern)
            .expect("example script should run cleanly");
        // The example script flags patterns with fewer than 4 outline
        // points; this one has exactly 3, so it should say so and name
        // the pattern, proving the round trip (args in, string out).
        assert!(message.contains("Sleeve"), "message was: {message}");
        assert!(message.contains('3'), "message was: {message}");
    }

    #[test]
    fn a_well_formed_pattern_gets_a_clean_bill() {
        let pattern = Pattern::new(
            PatternId(2),
            "Bodice",
            vec![
                OutlinePoint::corner(crate::geometry::Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(crate::geometry::Vec2::new(100.0, 0.0)),
                OutlinePoint::corner(crate::geometry::Vec2::new(100.0, 100.0)),
                OutlinePoint::corner(crate::geometry::Vec2::new(0.0, 100.0)),
            ],
        );
        let message = run_validation_script(&example_script(), &pattern)
            .expect("example script should run cleanly");
        assert!(message.is_empty(), "message was: {message}");
    }
}
