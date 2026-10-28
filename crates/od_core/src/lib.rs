//! Core of OpenDesign: the document model and the command/undo system.
//!
//! Every change to a [`Document`] goes through a [`Command`] executed by a
//! [`History`], so every tool in the app gets undo/redo for free.

pub mod bom;
pub mod clipboard;
pub mod command;
pub mod document;
pub mod dxf;
pub mod edit;
pub mod geometry;
pub mod history;
#[cfg(feature = "python-plugins")]
pub mod plugin;
pub mod project;
pub mod seam;

pub use bom::{document_bom, piece_bom, total_area_m2, PieceBom};
pub use clipboard::{Clip, Paste};
pub use command::{
    AddDart, AddInternalLine, AddNotch, AddPattern, AddSewingLine, AddTack, AddTopstitch, AddTrim,
    Batch, Command, DeletePattern, DeletePoint, DuplicatePattern, FlipSeamSide, InsertPoint,
    MirrorPattern, MovePattern, MovePoint, PatternFlag, PatternValue, RemoveDart,
    RemoveInternalLine, RemoveNotch, RemoveSewingLine, RemoveTack, RemoveTopstitch, RemoveTrim,
    RenamePattern, RotatePattern, SetArrangement, SetDart, SetFabric, SetGrading, SetOutline,
    SetParticleDistance, SetPatternFlag, SetPatternValue, SetPointKind, SetSeamEdge,
    PARTICLE_DISTANCE_RANGE,
};
pub use document::{
    Arrangement, Axis, Dart, Document, Fabric, FabricTexture, Grading, InternalLine, Notch,
    OutlinePoint, Pattern, PatternId, PointKind, SeamEdge, SewingLine, SewingLineId, SizeChart,
    Tack, TackId, TackPoint, TackTarget, Topstitch, Trim, TrimKind,
};
pub use geometry::Vec2;
pub use history::History;
pub use seam::Side;
