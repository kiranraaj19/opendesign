//! Colourways: named colour variants of a garment. Each one remembers a
//! fabric colour per pattern; switching to it paints those colours onto
//! the patterns' fabrics, so the views and exports need nothing new.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::document::{Document, PatternId};

/// One colour variant: the fabric colour of each pattern it knows. A
/// pattern added since it was saved keeps whatever colour it has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Colourway {
    pub name: String,
    pub colors: Vec<(PatternId, [u8; 3])>,
}

impl Colourway {
    /// The patterns' colours as they are now.
    pub fn capture(doc: &Document, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            colors: doc
                .patterns
                .iter()
                .map(|p| (p.id, p.fabric.color))
                .collect(),
        }
    }

    /// Does `doc` show exactly these colours (for patterns that still
    /// exist)?
    pub fn is_shown(&self, doc: &Document) -> bool {
        self.colors
            .iter()
            .all(|&(id, color)| doc.pattern(id).is_none_or(|p| p.fabric.color == color))
    }
}

/// Paint `colors` onto the patterns, returning what they were.
fn paint(doc: &mut Document, colors: &[(PatternId, [u8; 3])]) -> Vec<(PatternId, [u8; 3])> {
    let mut previous = Vec::new();
    for &(id, color) in colors {
        if let Some(p) = doc.pattern_mut(id) {
            previous.push((id, p.fabric.color));
            p.fabric.color = color;
        }
    }
    previous
}

/// A default name for the next colourway: "Colourway N".
pub fn next_name(doc: &Document) -> String {
    let mut n = doc.colourways.len() + 1;
    while doc
        .colourways
        .iter()
        .any(|c| c.name == format!("Colourway {n}"))
    {
        n += 1;
    }
    format!("Colourway {n}")
}

/// Save the current colours as a new colourway, which becomes the one
/// shown.
pub struct AddColourway {
    colourway: Colourway,
    previous_active: Option<usize>,
}

impl AddColourway {
    pub fn new(doc: &Document, name: impl Into<String>) -> Self {
        Self {
            colourway: Colourway::capture(doc, name),
            previous_active: None,
        }
    }
}

impl Command for AddColourway {
    fn label(&self) -> String {
        format!("Add colourway: {}", self.colourway.name)
    }

    fn apply(&mut self, doc: &mut Document) {
        self.previous_active = doc.active_colourway;
        doc.colourways.push(self.colourway.clone());
        doc.active_colourway = Some(doc.colourways.len() - 1);
    }

    fn undo(&mut self, doc: &mut Document) {
        doc.colourways.pop();
        doc.active_colourway = self.previous_active;
    }
}

/// Switch to colourway `index`: its colours onto the patterns.
pub struct ApplyColourway {
    index: usize,
    name: String,
    /// The colours it replaced, for undo.
    previous: Vec<(PatternId, [u8; 3])>,
    previous_active: Option<usize>,
}

impl ApplyColourway {
    pub fn new(doc: &Document, index: usize) -> Self {
        let name = doc
            .colourways
            .get(index)
            .map_or_else(String::new, |c| c.name.clone());
        Self {
            index,
            name,
            previous: Vec::new(),
            previous_active: None,
        }
    }
}

impl Command for ApplyColourway {
    fn label(&self) -> String {
        format!("Switch colourway: {}", self.name)
    }

    fn apply(&mut self, doc: &mut Document) {
        self.previous_active = doc.active_colourway;
        let Some(colors) = doc.colourways.get(self.index).map(|c| c.colors.clone()) else {
            return;
        };
        self.previous = paint(doc, &colors);
        doc.active_colourway = Some(self.index);
    }

    fn undo(&mut self, doc: &mut Document) {
        paint(doc, &std::mem::take(&mut self.previous));
        doc.active_colourway = self.previous_active;
    }
}

/// Store the patterns' current colours in colourway `index`.
pub struct UpdateColourway {
    index: usize,
    /// Swapped with the colourway's colours on apply and undo.
    colors: Option<Vec<(PatternId, [u8; 3])>>,
}

impl UpdateColourway {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            colors: None,
        }
    }
}

impl Command for UpdateColourway {
    fn label(&self) -> String {
        "Update colourway".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let colors = self
            .colors
            .take()
            .unwrap_or_else(|| Colourway::capture(doc, "").colors);
        if let Some(c) = doc.colourways.get_mut(self.index) {
            self.colors = Some(std::mem::replace(&mut c.colors, colors));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let (Some(c), Some(colors)) = (doc.colourways.get_mut(self.index), self.colors.take()) {
            self.colors = Some(std::mem::replace(&mut c.colors, colors));
        }
    }
}

/// Rename colourway `index`.
pub struct RenameColourway {
    index: usize,
    /// Swapped with the colourway's name on apply and undo.
    name: String,
}

impl RenameColourway {
    pub fn new(index: usize, name: impl Into<String>) -> Self {
        Self {
            index,
            name: name.into(),
        }
    }
}

impl Command for RenameColourway {
    fn label(&self) -> String {
        "Rename colourway".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(c) = doc.colourways.get_mut(self.index) {
            std::mem::swap(&mut c.name, &mut self.name);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

/// Forget colourway `index`; the patterns keep the colours they have.
pub struct RemoveColourway {
    index: usize,
    removed: Option<Colourway>,
    previous_active: Option<usize>,
}

impl RemoveColourway {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            removed: None,
            previous_active: None,
        }
    }
}

impl Command for RemoveColourway {
    fn label(&self) -> String {
        match &self.removed {
            Some(c) => format!("Delete colourway: {}", c.name),
            None => "Delete colourway".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if self.index >= doc.colourways.len() {
            return;
        }
        self.previous_active = doc.active_colourway;
        self.removed = Some(doc.colourways.remove(self.index));
        doc.active_colourway = match doc.active_colourway {
            Some(a) if a == self.index => None,
            Some(a) if a > self.index => Some(a - 1),
            other => other,
        };
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(c) = self.removed.take() {
            doc.colourways.insert(self.index, c);
            doc.active_colourway = self.previous_active;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Fabric, Pattern};
    use crate::history::History;

    fn doc() -> Document {
        let mut doc = Document::new("t");
        doc.patterns
            .push(Pattern::new(PatternId(1), "a", Vec::new()));
        doc.patterns
            .push(Pattern::new(PatternId(2), "b", Vec::new()));
        doc
    }

    fn colors(doc: &Document) -> Vec<[u8; 3]> {
        doc.patterns.iter().map(|p| p.fabric.color).collect()
    }

    #[test]
    fn colourways_switch_and_undo_exactly() {
        let mut doc = doc();
        let mut history = History::new();
        let original = colors(&doc);
        let add = AddColourway::new(&doc, "Natural");
        history.execute(&mut doc, Box::new(add));
        assert_eq!(doc.active_colourway, Some(0));

        // Recolour and save that as a second colourway.
        doc.patterns[0].fabric.color = [200, 0, 0];
        doc.patterns[1].fabric.color = [0, 0, 200];
        let navy = colors(&doc);
        let add = AddColourway::new(&doc, "Bold");
        history.execute(&mut doc, Box::new(add));
        assert_eq!(doc.colourways.len(), 2);
        assert!(doc.colourways[1].is_shown(&doc));
        assert!(!doc.colourways[0].is_shown(&doc));

        let switch = ApplyColourway::new(&doc, 0);
        history.execute(&mut doc, Box::new(switch));
        assert_eq!(colors(&doc), original);
        assert_eq!(doc.active_colourway, Some(0));
        assert_eq!(
            history.undo_label().as_deref(),
            Some("Switch colourway: Natural")
        );
        history.undo(&mut doc);
        assert_eq!(colors(&doc), navy);
        assert_eq!(doc.active_colourway, Some(1));
        history.redo(&mut doc);
        assert_eq!(colors(&doc), original);

        // Undo everything: no colourways, the bold colours (set directly,
        // not by a command) still on.
        while history.can_undo() {
            history.undo(&mut doc);
        }
        assert!(doc.colourways.is_empty());
        assert_eq!(doc.active_colourway, None);
    }

    #[test]
    fn a_pattern_added_later_keeps_its_colour() {
        let mut doc = doc();
        let mut add = AddColourway::new(&doc, "One");
        add.apply(&mut doc);
        let mut late = Pattern::new(PatternId(3), "c", Vec::new());
        late.fabric = Fabric::denim();
        doc.patterns.push(late);
        doc.patterns[0].fabric.color = [1, 2, 3];
        let mut apply = ApplyColourway::new(&doc, 0);
        apply.apply(&mut doc);
        assert_eq!(doc.patterns[0].fabric.color, Fabric::cotton().color);
        assert_eq!(doc.patterns[2].fabric.color, Fabric::denim().color);
        apply.undo(&mut doc);
        assert_eq!(doc.patterns[0].fabric.color, [1, 2, 3]);
    }

    #[test]
    fn update_rename_and_remove_undo_exactly() {
        let mut doc = doc();
        AddColourway::new(&doc, "A").apply(&mut doc);
        AddColourway::new(&doc, "B").apply(&mut doc);
        let before = doc.clone();

        doc.patterns[0].fabric.color = [9, 9, 9];
        let recoloured = doc.clone();
        let mut update = UpdateColourway::new(0);
        update.apply(&mut doc);
        assert_eq!(doc.colourways[0].colors[0].1, [9, 9, 9]);
        update.undo(&mut doc);
        assert_eq!(doc, recoloured);
        update.apply(&mut doc);
        assert_eq!(doc.colourways[0].colors[0].1, [9, 9, 9]);
        update.undo(&mut doc);

        let mut rename = RenameColourway::new(1, "Summer");
        rename.apply(&mut doc);
        assert_eq!(doc.colourways[1].name, "Summer");
        rename.undo(&mut doc);
        assert_eq!(doc, recoloured);

        // Removing the shown one shows none; removing an earlier one
        // shifts the shown index down.
        let mut remove = RemoveColourway::new(1);
        remove.apply(&mut doc);
        assert_eq!((doc.colourways.len(), doc.active_colourway), (1, None));
        assert_eq!(remove.label(), "Delete colourway: B");
        remove.undo(&mut doc);
        assert_eq!(doc, recoloured);
        let mut remove = RemoveColourway::new(0);
        remove.apply(&mut doc);
        assert_eq!(doc.active_colourway, Some(0));
        remove.undo(&mut doc);
        assert_eq!(doc, recoloured);
        assert_eq!(before.colourways, doc.colourways);
    }

    #[test]
    fn next_name_skips_taken_names() {
        let mut doc = doc();
        assert_eq!(next_name(&doc), "Colourway 1");
        AddColourway::new(&doc, "Colourway 2").apply(&mut doc);
        assert_eq!(next_name(&doc), "Colourway 3");
    }

    #[test]
    fn colourways_are_saved_and_old_files_have_none() {
        let mut doc = doc();
        AddColourway::new(&doc, "A").apply(&mut doc);
        let json = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc);
        // A document saved before colourways.
        let mut value = serde_json::to_value(&doc).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("colourways");
        object.remove("active_colourway");
        let old: Document = serde_json::from_value(value).unwrap();
        assert!(old.colourways.is_empty());
        assert_eq!(old.active_colourway, None);
    }
}
