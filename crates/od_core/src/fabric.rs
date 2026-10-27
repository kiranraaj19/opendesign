//! A minimal fabric catalog (Phase 8 placeholder for the full Phase 7
//! fabric/material model: physical properties, weave textures, PBR).
//!
//! Each entry is just a name and a swatch colour, enough for the asset
//! library to list fabrics and let a pattern be tagged with one. A
//! [`Pattern`](crate::document::Pattern) stores the chosen fabric by name
//! (`Pattern::fabric`), so adding fabrics here later doesn't need a project
//! migration.

/// One built-in fabric: a name and an approximate swatch colour (RGB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fabric {
    pub name: &'static str,
    pub color: [u8; 3],
}

/// Built-in fabrics shown in the Library panel, swatch colour only (no
/// weight, stretch or weave yet — that's Phase 7).
pub const BUILTIN_FABRICS: &[Fabric] = &[
    Fabric {
        name: "Cotton poplin",
        color: [0xE8, 0xE2, 0xD0],
    },
    Fabric {
        name: "Denim",
        color: [0x3B, 0x5B, 0x7D],
    },
    Fabric {
        name: "Silk charmeuse",
        color: [0xC9, 0x9B, 0xC4],
    },
    Fabric {
        name: "Wool felt",
        color: [0x6E, 0x5A, 0x4E],
    },
    Fabric {
        name: "Jersey knit",
        color: [0x8A, 0x8F, 0x98],
    },
];

/// Look up a built-in fabric by name, e.g. a pattern's [`Pattern::fabric`].
pub fn find(name: &str) -> Option<Fabric> {
    BUILTIN_FABRICS.iter().copied().find(|f| f.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_names_are_unique_and_findable() {
        let mut names: Vec<_> = BUILTIN_FABRICS.iter().map(|f| f.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BUILTIN_FABRICS.len());

        for fabric in BUILTIN_FABRICS {
            assert_eq!(find(fabric.name), Some(*fabric));
        }
        assert_eq!(find("Not a fabric"), None);
    }
}
