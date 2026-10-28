//! Which patterns are selected. Several can be; the one picked last is the
//! main one, which Properties shows and single-pattern tools act on.

use od_core::{Document, PatternId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// In the order they were picked; the last is the main one.
    ids: Vec<PatternId>,
}

impl Selection {
    /// The main selected pattern: the one picked last.
    pub fn primary(&self) -> Option<PatternId> {
        self.ids.last().copied()
    }

    /// Every selected pattern, in the order they were picked.
    pub fn ids(&self) -> &[PatternId] {
        &self.ids
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn contains(&self, id: PatternId) -> bool {
        self.ids.contains(&id)
    }

    /// Select only `id`, or nothing.
    pub fn set(&mut self, id: Option<PatternId>) {
        self.ids.clear();
        self.ids.extend(id);
    }

    /// Select exactly `ids`; the last becomes the main one.
    pub fn set_all(&mut self, ids: impl IntoIterator<Item = PatternId>) {
        self.ids.clear();
        for id in ids {
            self.add(id);
        }
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Add `id`, making it the main one.
    pub fn add(&mut self, id: PatternId) {
        self.ids.retain(|&i| i != id);
        self.ids.push(id);
    }

    /// Add `id` if it isn't selected, else remove it.
    pub fn toggle(&mut self, id: PatternId) {
        if self.contains(id) {
            self.ids.retain(|&i| i != id);
        } else {
            self.ids.push(id);
        }
    }

    /// Drop patterns that no longer exist, e.g. after undo.
    pub fn retain_existing(&mut self, doc: &Document) {
        self.ids.retain(|&id| doc.pattern(id).is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PatternId = PatternId(1);
    const B: PatternId = PatternId(2);

    #[test]
    fn the_last_picked_is_the_main_one() {
        let mut s = Selection::default();
        assert_eq!(s.primary(), None);
        s.set(Some(A));
        s.add(B);
        assert_eq!(s.primary(), Some(B));
        assert_eq!(s.ids(), [A, B]);
        // Picking A again makes it the main one, without a duplicate.
        s.add(A);
        assert_eq!(s.ids(), [B, A]);
        s.toggle(A);
        assert_eq!(s.ids(), [B]);
        s.toggle(A);
        assert_eq!(s.primary(), Some(A));
        s.set(None);
        assert!(s.ids().is_empty());
    }
}
