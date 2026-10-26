//! Commands are the only way to change a [`Document`]. Each one knows how to
//! apply itself and how to reverse that change.

use crate::document::{Document, Pattern, PatternId};
use crate::geometry::Vec2;

pub trait Command {
    /// Short label shown in the History panel and the Edit menu.
    fn label(&self) -> String;
    fn apply(&mut self, doc: &mut Document);
    fn undo(&mut self, doc: &mut Document);
}

pub struct AddPattern {
    name: String,
    outline: Vec<Vec2>,
    /// Assigned on first apply so redo recreates the same id.
    id: Option<PatternId>,
}

impl AddPattern {
    pub fn new(name: impl Into<String>, outline: Vec<Vec2>) -> Self {
        Self {
            name: name.into(),
            outline,
            id: None,
        }
    }

    /// Id of the created pattern, available after the command has run.
    pub fn id(&self) -> Option<PatternId> {
        self.id
    }
}

impl Command for AddPattern {
    fn label(&self) -> String {
        format!("Add {}", self.name)
    }

    fn apply(&mut self, doc: &mut Document) {
        let id = *self.id.get_or_insert_with(|| doc.allocate_id());
        doc.patterns.push(Pattern {
            id,
            name: self.name.clone(),
            outline: self.outline.clone(),
        });
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(id) = self.id {
            doc.patterns.retain(|p| p.id != id);
        }
    }
}

pub struct DeletePattern {
    id: PatternId,
    /// The removed pattern and its position, kept for undo.
    removed: Option<(usize, Pattern)>,
}

impl DeletePattern {
    pub fn new(id: PatternId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for DeletePattern {
    fn label(&self) -> String {
        match &self.removed {
            Some((_, p)) => format!("Delete {}", p.name),
            None => "Delete pattern".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(index) = doc.index_of(self.id) {
            self.removed = Some((index, doc.patterns.remove(index)));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((index, pattern)) = self.removed.take() {
            doc.patterns.insert(index.min(doc.patterns.len()), pattern);
        }
    }
}

pub struct MovePattern {
    id: PatternId,
    delta: Vec2,
}

impl MovePattern {
    pub fn new(id: PatternId, delta: Vec2) -> Self {
        Self { id, delta }
    }

    fn shift(doc: &mut Document, id: PatternId, delta: Vec2) {
        if let Some(p) = doc.pattern_mut(id) {
            p.outline.iter_mut().for_each(|pt| *pt += delta);
        }
    }
}

impl Command for MovePattern {
    fn label(&self) -> String {
        "Move pattern".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        Self::shift(doc, self.id, self.delta);
    }

    fn undo(&mut self, doc: &mut Document) {
        Self::shift(doc, self.id, -self.delta);
    }
}

pub struct RenamePattern {
    id: PatternId,
    name: String,
}

impl RenamePattern {
    pub fn new(id: PatternId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

impl Command for RenamePattern {
    fn label(&self) -> String {
        format!("Rename to {}", self.name)
    }

    // Swapping makes apply and undo the same operation.
    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.name, &mut self.name);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}
