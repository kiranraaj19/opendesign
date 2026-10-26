//! Undo/redo stacks of executed commands.

use crate::command::Command;
use crate::document::Document;

#[derive(Default)]
pub struct History {
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply `command` and record it. Clears the redo stack.
    pub fn execute(&mut self, doc: &mut Document, mut command: Box<dyn Command>) {
        command.apply(doc);
        self.undo.push(command);
        self.redo.clear();
    }

    /// Returns `false` when there is nothing to undo.
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let Some(mut command) = self.undo.pop() else {
            return false;
        };
        command.undo(doc);
        self.redo.push(command);
        true
    }

    /// Returns `false` when there is nothing to redo.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let Some(mut command) = self.redo.pop() else {
            return false;
        };
        command.apply(doc);
        self.undo.push(command);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|c| c.label())
    }

    /// Labels of applied commands, oldest first.
    pub fn done_labels(&self) -> Vec<String> {
        self.undo.iter().map(|c| c.label()).collect()
    }

    /// Labels of undone commands, next-to-redo first.
    pub fn undone_labels(&self) -> Vec<String> {
        self.redo.iter().rev().map(|c| c.label()).collect()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, DeletePattern, MovePattern, RenamePattern};
    use crate::geometry::{rectangle, Vec2};

    fn add(doc: &mut Document, history: &mut History, name: &str) {
        let outline = rectangle(Vec2::ZERO, Vec2::new(10.0, 10.0));
        history.execute(doc, Box::new(AddPattern::new(name, outline)));
    }

    #[test]
    fn undo_and_redo_add() {
        let (mut doc, mut history) = (Document::default(), History::new());
        add(&mut doc, &mut history, "Front");
        let id = doc.patterns[0].id;

        assert!(history.undo(&mut doc));
        assert!(doc.patterns.is_empty());
        assert!(history.redo(&mut doc));
        assert_eq!(doc.patterns[0].id, id, "redo keeps the same id");
        assert!(!history.redo(&mut doc));
    }

    #[test]
    fn new_command_clears_redo() {
        let (mut doc, mut history) = (Document::default(), History::new());
        add(&mut doc, &mut history, "Front");
        history.undo(&mut doc);
        add(&mut doc, &mut history, "Back");
        assert!(!history.can_redo());
        assert_eq!(history.done_labels(), vec!["Add Back"]);
    }

    #[test]
    fn delete_undo_restores_order() {
        let (mut doc, mut history) = (Document::default(), History::new());
        for name in ["A", "B", "C"] {
            add(&mut doc, &mut history, name);
        }
        let b = doc.patterns[1].id;
        history.execute(&mut doc, Box::new(DeletePattern::new(b)));
        assert_eq!(history.undo_label().as_deref(), Some("Delete B"));
        history.undo(&mut doc);
        let names: Vec<_> = doc.patterns.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C"]);
    }

    #[test]
    fn move_and_rename_round_trip() {
        let (mut doc, mut history) = (Document::default(), History::new());
        add(&mut doc, &mut history, "Sleeve");
        let id = doc.patterns[0].id;
        let before = doc.clone();

        history.execute(
            &mut doc,
            Box::new(MovePattern::new(id, Vec2::new(5.0, -3.0))),
        );
        history.execute(&mut doc, Box::new(RenamePattern::new(id, "Left sleeve")));
        assert_eq!(doc.patterns[0].outline[0], Vec2::new(5.0, -3.0));
        assert_eq!(doc.patterns[0].name, "Left sleeve");

        history.undo(&mut doc);
        history.undo(&mut doc);
        assert_eq!(doc, before);
    }
}
