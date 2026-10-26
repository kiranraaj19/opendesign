//! The document: everything that is saved in a project file.

use crate::geometry::{self, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PatternId(pub u64);

/// A flat garment piece, drawn in the 2D window and draped in 3D.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub id: PatternId,
    pub name: String,
    /// Closed outline in millimetres.
    pub outline: Vec<Vec2>,
}

impl Pattern {
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        geometry::bounds(&self.outline)
    }

    pub fn area(&self) -> f32 {
        geometry::area(&self.outline)
    }

    pub fn perimeter(&self) -> f32 {
        geometry::perimeter(&self.outline)
    }

    pub fn contains(&self, p: Vec2) -> bool {
        geometry::contains(&self.outline, p)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub name: String,
    pub patterns: Vec<Pattern>,
    next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

impl Document {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            patterns: Vec::new(),
            next_id: 1,
        }
    }

    /// Reserve a fresh id. Ids are never reused, even after undo.
    pub fn allocate_id(&mut self) -> PatternId {
        let id = PatternId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut Pattern> {
        self.patterns.iter_mut().find(|p| p.id == id)
    }

    pub fn index_of(&self, id: PatternId) -> Option<usize> {
        self.patterns.iter().position(|p| p.id == id)
    }

    /// Topmost pattern under `p` (later patterns draw on top).
    pub fn pattern_at(&self, p: Vec2) -> Option<PatternId> {
        self.patterns
            .iter()
            .rev()
            .find(|pat| pat.contains(p))
            .map(|pat| pat.id)
    }
}
