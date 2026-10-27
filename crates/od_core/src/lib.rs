//! Core of OpenDesign: the document model and the command/undo system.
//!
//! Every change to a [`Document`] goes through a [`Command`] executed by a
//! [`History`], so every tool in the app gets undo/redo for free.

pub mod bom;
pub mod command;
pub mod document;
pub mod dxf;
pub mod geometry;
pub mod history;
#[cfg(feature = "python-plugins")]
pub mod plugin;
pub mod project;

pub use bom::{document_bom, piece_bom, total_area_m2, PieceBom};
pub use command::{
    AddInternalLine, AddNotch, AddPattern, AddSewingLine, AddTack, AddTopstitch, AddTrim, Command,
    DeletePattern, DeletePoint, DuplicatePattern, InsertPoint, MirrorPattern, MovePattern,
    MovePoint, PatternValue, RemoveInternalLine, RemoveNotch, RemoveSewingLine, RemoveTack,
    RemoveTopstitch, RemoveTrim, RenamePattern, SetFabric, SetGrading, SetPatternValue,
    SetPointKind,
};
pub use document::{
    Axis, Document, Fabric, FabricTexture, Grading, InternalLine, Notch, OutlinePoint, Pattern,
    PatternId, PointKind, SeamEdge, SeamPoint, SewingLine, SewingLineId, SizeChart, Tack, TackId,
    Topstitch, Trim, TrimKind,
};
pub use geometry::Vec2;
pub use history::History;
