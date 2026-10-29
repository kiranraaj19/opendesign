//! Named sets of pieces (groups), kept in the document so they are saved
//! with the project.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::document::{Document, PatternId};

/// A named set of pieces, picked together from the Objects tab.
///
/// Members are pattern ids, which are never reused, so a deleted member
/// simply stops counting (see [`Group::members_in`]) and comes back if the
/// delete is undone. Ids of patterns that are gone are dropped when a
/// file is loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub members: Vec<PatternId>,
}

impl Group {
    pub fn new(name: impl Into<String>, members: Vec<PatternId>) -> Self {
        Self {
            name: name.into(),
            members,
        }
    }

    /// The members that are in `doc`, in group order.
    pub fn members_in<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = PatternId> + 'a {
        self.members
            .iter()
            .copied()
            .filter(|&id| doc.pattern(id).is_some())
    }
}

/// The first of "`stem` 1", "`stem` 2", ... not in `taken`.
pub fn unused_name<'a>(stem: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    (1..)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !taken.clone().any(|t| t == name))
        .unwrap_or_else(|| stem.to_owned())
}

/// Replace the document's groups with `groups`: adding, removing,
/// renaming and refilling them are all this, so undo is a swap back.
pub struct SetGroups {
    label: String,
    /// The list to put in place; holds the old one after apply.
    groups: Vec<Group>,
}

impl SetGroups {
    pub fn new(label: impl Into<String>, groups: Vec<Group>) -> Self {
        Self {
            label: label.into(),
            groups,
        }
    }
}

impl Command for SetGroups {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        std::mem::swap(&mut doc.groups, &mut self.groups);
    }

    fn undo(&mut self, doc: &mut Document) {
        std::mem::swap(&mut doc.groups, &mut self.groups);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddPattern, DeletePattern};
    use crate::geometry::{rectangle, Vec2};
    use crate::{project, History};

    fn two_patterns() -> (Document, History, [PatternId; 2]) {
        let (mut doc, mut history) = (Document::default(), History::new());
        for x in [0.0, 200.0] {
            let outline = rectangle(Vec2::new(x, 0.0), Vec2::new(x + 100.0, 100.0));
            history.execute(&mut doc, Box::new(AddPattern::new("Piece", outline)));
        }
        let ids = [doc.patterns[0].id, doc.patterns[1].id];
        (doc, history, ids)
    }

    #[test]
    fn groups_are_added_and_removed_with_exact_undo() {
        let (mut doc, mut history, ids) = two_patterns();
        let group = Group::new("Bodice", ids.to_vec());
        history.execute(
            &mut doc,
            Box::new(SetGroups::new("Group 2 patterns", vec![group.clone()])),
        );
        assert_eq!(doc.groups.first(), Some(&group));
        assert_eq!(history.undo_label().as_deref(), Some("Group 2 patterns"));
        history.undo(&mut doc);
        assert!(doc.groups.is_empty());
        history.redo(&mut doc);
        assert_eq!(doc.groups, [group]);
    }

    #[test]
    fn a_deleted_member_drops_out_and_comes_back_on_undo() {
        let (mut doc, mut history, ids) = two_patterns();
        let group = Group::new("Bodice", ids.to_vec());
        history.execute(&mut doc, Box::new(SetGroups::new("Group", vec![group])));
        history.execute(&mut doc, Box::new(DeletePattern::new(ids[0])));
        let live: Vec<_> = doc.groups[0].members_in(&doc).collect();
        assert_eq!(live, [ids[1]]);
        history.undo(&mut doc);
        let live: Vec<_> = doc.groups[0].members_in(&doc).collect();
        assert_eq!(live, ids);
    }

    #[test]
    fn groups_are_saved_and_stale_members_dropped_on_load() {
        let (mut doc, mut history, ids) = two_patterns();
        let group = Group::new("Bodice", ids.to_vec());
        history.execute(&mut doc, Box::new(SetGroups::new("Group", vec![group])));
        let loaded = project::from_json(&project::to_json(&doc)).unwrap();
        assert_eq!(loaded, doc);

        history.execute(&mut doc, Box::new(DeletePattern::new(ids[1])));
        let loaded = project::from_json(&project::to_json(&doc)).unwrap();
        assert_eq!(loaded.groups[0].members, [ids[0]]);
    }

    #[test]
    fn files_without_groups_still_load() {
        let (doc, _, _) = two_patterns();
        let mut json: serde_json::Value = serde_json::from_str(&project::to_json(&doc)).unwrap();
        let document = json["document"].as_object_mut().unwrap();
        document.remove("groups");
        let loaded = project::from_json(&json.to_string()).unwrap();
        assert!(loaded.groups.is_empty());
        assert_eq!(loaded.patterns, doc.patterns);
    }

    #[test]
    fn unused_names_count_up() {
        let taken = ["Group 1", "Group 3"];
        assert_eq!(unused_name("Group", taken.iter().copied()), "Group 2");
        assert_eq!(unused_name("View", [].iter().copied()), "View 1");
    }
}
