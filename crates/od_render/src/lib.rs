//! OpenDesign's 3D view, apart from the app: an orbit camera, procedural
//! avatars, avatar import, pattern meshes, ray picking, and the wgpu
//! renderer that draws them. World units are metres, Y up, avatar facing +Z.
//!
//! Everything except [`gpu`] is plain CPU code and is tested without a GPU.

pub mod arrange;
pub mod avatar;
pub mod camera;
pub mod export;
pub mod gpu;
pub mod import;
pub mod material;
pub mod measure;
pub mod mesh;
pub mod pattern;
pub mod pick;
pub mod render;
pub mod scene;
pub mod view;

pub use glam;
pub use wgpu;

/// Depth buffer the app must ask eframe for (`NativeOptions::depth_buffer`
/// is the bit count, [`DEPTH_BITS`]).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const DEPTH_BITS: u8 = 32;
/// Multisampling for the whole window. WebGPU guarantees 4x for the formats
/// used here.
pub const MSAA_SAMPLES: u16 = 4;
