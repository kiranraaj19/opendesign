//! OpenDesign cloth simulation (Phase 6): an XPBD solver, collision
//! shapes to drape against, and code that turns a [`od_core::Document`]'s
//! patterns and sewing data into a simulatable [`cloth::ClothScene`].
//!
//! Nothing here touches a GPU; it only needs positions in, positions out,
//! so it can be driven by `od_app` every frame or exercised directly in
//! tests.

pub mod cloth;
pub mod collision;
pub mod drape;
pub mod mesh;
pub mod xpbd;
