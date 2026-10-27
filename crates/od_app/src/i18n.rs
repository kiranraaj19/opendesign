//! Minimal i18n layer for `od_app`'s UI strings.
//!
//! This is a hand-rolled key→string lookup rather than a crate like
//! `fluent`: the app currently has a handful of translated strings, and a
//! `match` over an enum is enough to prove the plumbing (a `Locale` the
//! user can switch, a lookup function every call site routes through, a
//! round trip through saved preferences) without pulling in a message
//! format / plural rules / `.ftl` resource loader sized for hundreds of
//! strings. If translation coverage grows well past the menu bar, revisit
//! `fluent` then — the call sites below (`t(locale, Key::...)`) are shaped
//! so swapping the lookup for a resource-backed one later only touches
//! this file.

/// Supported UI locales. English is the only fully covered locale; French
/// covers the strings below to prove the mechanism end-to-end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Locale {
    #[default]
    En,
    Fr,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::En, Locale::Fr];

    /// The locale's own name, in that locale (for the picker itself).
    pub fn display_name(self) -> &'static str {
        match self {
            Locale::En => "English",
            Locale::Fr => "Français",
        }
    }
}

/// Keys for every string translated so far. Deliberately small: top-level
/// menu labels, the File menu's items and Undo/Redo, enough to prove a
/// string can be swapped end-to-end without translating the whole app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    MenuFile,
    MenuEdit,
    MenuView,
    MenuHelp,
    New,
    Open,
    OpenRecent,
    ClearRecent,
    Save,
    SaveAs,
    Quit,
    Undo,
    Redo,
    Delete,
    Language,
}

/// Look up `key`'s string in `locale`. Falls back to English text inline
/// (there's no separate "missing key" case: every key is matched for
/// every locale below, so a translator adding a locale gets a compile
/// error, not a silent fallback, if they miss one).
pub fn t(locale: Locale, key: Key) -> &'static str {
    use Key::*;
    use Locale::*;
    match (locale, key) {
        (En, MenuFile) => "File",
        (Fr, MenuFile) => "Fichier",

        (En, MenuEdit) => "Edit",
        (Fr, MenuEdit) => "Édition",

        (En, MenuView) => "View",
        (Fr, MenuView) => "Affichage",

        (En, MenuHelp) => "Help",
        (Fr, MenuHelp) => "Aide",

        (En, New) => "New",
        (Fr, New) => "Nouveau",

        (En, Open) => "Open…",
        (Fr, Open) => "Ouvrir…",

        (En, OpenRecent) => "Open Recent",
        (Fr, OpenRecent) => "Récemment ouverts",

        (En, ClearRecent) => "Clear recent",
        (Fr, ClearRecent) => "Effacer la liste",

        (En, Save) => "Save",
        (Fr, Save) => "Enregistrer",

        (En, SaveAs) => "Save As…",
        (Fr, SaveAs) => "Enregistrer sous…",

        (En, Quit) => "Quit",
        (Fr, Quit) => "Quitter",

        (En, Undo) => "Undo",
        (Fr, Undo) => "Annuler",

        (En, Redo) => "Redo",
        (Fr, Redo) => "Rétablir",

        (En, Delete) => "Delete",
        (Fr, Delete) => "Supprimer",

        (En, Language) => "Language",
        (Fr, Language) => "Langue",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_matches_existing_ui_labels() {
        // These exact strings are relied on by app.rs's UI tests
        // (get_by_label("File") etc.) — English must stay the literal
        // default so switching in a locale layer doesn't change behaviour
        // for users (and tests) who never touch the language picker.
        assert_eq!(t(Locale::En, Key::MenuFile), "File");
        assert_eq!(t(Locale::En, Key::MenuEdit), "Edit");
        assert_eq!(t(Locale::En, Key::MenuView), "View");
        assert_eq!(t(Locale::En, Key::MenuHelp), "Help");
    }

    #[test]
    fn french_translations_are_distinct() {
        for key in [Key::MenuFile, Key::MenuEdit, Key::MenuView, Key::MenuHelp] {
            assert_ne!(t(Locale::En, key), t(Locale::Fr, key));
        }
    }

    #[test]
    fn default_locale_is_english() {
        assert_eq!(Locale::default(), Locale::En);
    }
}
