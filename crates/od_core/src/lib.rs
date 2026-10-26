//! Core of OpenDesign: the document model and the command/undo system.
//!
//! Every change to a [`Document`] goes through a [`Command`] executed by a
//! [`History`], so every tool in the app gets undo/redo for free.

pub mod command;
pub mod document;
pub mod geometry;
pub mod history;

pub use command::{AddPattern, Command, DeletePattern, MovePattern, RenamePattern};
pub use document::{Document, Pattern, PatternId};
pub use geometry::Vec2;
pub use history::History;
