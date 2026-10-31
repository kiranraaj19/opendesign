//! Offscreen renders of the garment for the assistant to look at: the 3D
//! scene from a chosen side (flat pieces, or the drape when there is one),
//! optionally as a strain heatmap, or a flat sheet of every pattern piece.

use std::sync::Arc;

use od_core::document::TOLERANCE;
use od_core::{Fabric, FabricTexture, PatternId};
use od_render::camera::{OrbitCamera, ViewPreset};
use od_render::glam::{Vec2, Vec3};
use od_render::material::{plain_weave, twill_weave, Material};
use od_render::mesh::Aabb;
use od_render::pattern::Placement;
use od_render::render::{render_still, StillSettings};
use od_render::scene::{linear, AvatarDraw, Color, GridColors};
use od_render::view::{place_pattern, scene_bounds, FrameInput, PlacedPattern, Theme};
use od_render::wgpu;

use crate::session::Session;

/// Which way the camera looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    Preset(ViewPreset),
    /// From the front, a little to the avatar's left and above.
    ThreeQuarter,
    /// Every pattern piece laid flat, as drafted.
    Sheet,
}

impl View {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "front" => View::Preset(ViewPreset::Front),
            "back" => View::Preset(ViewPreset::Back),
            "left" => View::Preset(ViewPreset::Left),
            "right" => View::Preset(ViewPreset::Right),
            "top" => View::Preset(ViewPreset::Top),
            "three_quarter" => View::ThreeQuarter,
            "pattern" | "sheet" => View::Sheet,
            _ => return None,
        })
    }
}

pub struct Request {
    pub view: View,
    pub size: [u32; 2],
    pub avatar: bool,
    pub dark: bool,
    /// Colour the cloth by how stretched it is instead of by fabric.
    pub strain: bool,
    /// Turn the camera this many degrees around the scene after aiming.
    pub turn: f32,
}

/// A render as PNG bytes, and a line saying what it shows.
pub fn render(s: &mut Session, req: &Request) -> Result<(Vec<u8>, String), String> {
    let gpu = s.gpu()?;
    let theme = theme(req.dark);
    let aspect = req.size[0] as f32 / req.size[1].max(1) as f32;
    let avatar = s.avatar_mesh();
    let avatar_bounds = avatar.bounds().unwrap_or(Aabb {
        min: Vec3::ZERO,
        max: Vec3::ONE,
    });

    let (placed, draped, strain_colors) = if req.view == View::Sheet {
        (sheet(s), false, None)
    } else if let Some(state) = s.current_drape() {
        let placed = state.drape.placed_patterns();
        let colors = req.strain.then(|| {
            let by_id = state.drape.strain_colors(strain_color);
            placed
                .iter()
                .map(|p| {
                    by_id
                        .iter()
                        .find(|(id, _)| *id == p.key)
                        .map(|(_, c)| c.clone())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        });
        (placed, true, colors)
    } else {
        let placements = s.placements();
        let placed = placements
            .iter()
            .filter_map(|(id, placement)| {
                let outline = flat_outline(s, *id)?;
                place_pattern(*id, &outline, placement)
            })
            .collect();
        (placed, false, None)
    };
    if req.view == View::Sheet && placed.is_empty() {
        return Err("there are no pattern pieces to show".into());
    }
    let materials: Vec<Option<Material>> = placed
        .iter()
        .map(|p| s.doc.pattern(p.key).map(|pat| material(&pat.fabric)))
        .collect();

    let show_avatar = req.avatar && req.view != View::Sheet;
    let bounds = if req.view == View::Sheet {
        placed
            .iter()
            .filter_map(|p| p.mesh.bounds())
            .reduce(Aabb::union)
            .unwrap_or(avatar_bounds)
    } else if show_avatar {
        scene_bounds(avatar_bounds, &placed)
    } else {
        placed
            .iter()
            .filter_map(|p| p.mesh.bounds())
            .reduce(Aabb::union)
            .unwrap_or(avatar_bounds)
    };
    let mut camera = OrbitCamera::default();
    match req.view {
        View::Preset(preset) => camera.set_view(preset),
        View::ThreeQuarter => {}
        View::Sheet => camera.set_view(ViewPreset::Front),
    }
    camera.frame(bounds, aspect);
    if req.turn != 0.0 {
        camera.orbit(req.turn.to_radians(), 0.0);
    }

    let pixels = render_still(
        &gpu.device,
        &gpu.queue,
        FrameInput {
            camera: &camera,
            aspect,
            encode_srgb: true,
            avatar: show_avatar.then(|| AvatarDraw {
                key: 1,
                mesh: avatar.clone(),
                color: theme.avatar,
            }),
            avatar_bounds: if req.view == View::Sheet {
                bounds
            } else {
                avatar_bounds
            },
            patterns: &placed,
            selected: &[],
            theme: &theme,
            pattern_colors: strain_colors.as_deref(),
            pattern_materials: Some(&materials),
            display: Default::default(),
        },
        StillSettings {
            size: req.size,
            ..Default::default()
        },
        clear(req.dark),
    );
    let png = encode_png(&pixels, req.size)?;
    let what = match (req.view, draped) {
        (View::Sheet, _) => "the flat pattern pieces",
        (_, true) => "the draped garment",
        (_, false) => "the pieces flat around the avatar, not draped yet",
    };
    Ok((
        png,
        format!(
            "Rendered {what} at {}×{} on {}.",
            req.size[0], req.size[1], gpu.adapter
        ),
    ))
}

/// A pattern's outline as the 3D code wants it.
fn flat_outline(s: &Session, id: PatternId) -> Option<Vec<Vec2>> {
    let pattern = s.doc.pattern(id)?;
    Some(
        pattern
            .polyline(TOLERANCE)
            .into_iter()
            .map(|p| Vec2::new(p.x, p.y))
            .collect(),
    )
}

/// Every shown pattern laid flat where it is drafted, facing the camera.
fn sheet(s: &Session) -> Vec<PlacedPattern<PatternId>> {
    s.doc
        .patterns
        .iter()
        .filter(|p| !p.hidden)
        .filter_map(|p| {
            let outline = flat_outline(s, p.id)?;
            let (min, max) = outline
                .iter()
                .fold((outline[0], outline[0]), |(lo, hi), q| {
                    (lo.min(*q), hi.max(*q))
                });
            let mid = (min + max) * 0.5 / 1000.0;
            let placement = Placement {
                // 2D y points down; the sheet stands upright.
                center: Vec3::new(mid.x, 2.0 - mid.y, 0.0),
                right: Vec3::X,
                up: Vec3::Y,
            };
            place_pattern(p.id, &outline, &placement)
        })
        .collect()
}

/// Blue where the cloth is relaxed, through green, to red at 10% stretch
/// and more.
fn strain_color(strain: f32) -> Color {
    let t = (strain / 0.10).clamp(0.0, 1.0);
    let (a, b, u) = if t < 0.5 {
        ([0.15, 0.35, 0.85], [0.2, 0.75, 0.3], t * 2.0)
    } else {
        ([0.2, 0.75, 0.3], [0.9, 0.15, 0.1], (t - 0.5) * 2.0)
    };
    let c = |i: usize| a[i] + (b[i] - a[i]) * u;
    [c(0), c(1), c(2), 1.0]
}

/// A fabric's look, as the app draws it.
fn material(fabric: &Fabric) -> Material {
    let [r, g, b] = fabric.color;
    let base = linear([r, g, b, 255]);
    // FNV-1a of the look, so the same fabric reuses one texture.
    let key = format!("{}/{:?}/{:?}", fabric.name, fabric.color, fabric.texture);
    let id = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    match fabric.texture {
        FabricTexture::Flat => Material::flat(base),
        FabricTexture::Plain => {
            Material::textured(base, Arc::new(plain_weave(id, fabric.color, 4)))
        }
        FabricTexture::Twill => {
            Material::textured(base, Arc::new(twill_weave(id, fabric.color, 4)))
        }
    }
}

/// The app's light and dark theme colours.
fn theme(dark: bool) -> Theme {
    let rgb = |c: u32| linear([(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    if dark {
        Theme {
            avatar: rgb(0x9C8F84),
            pattern: rgb(0x7A9CC6),
            selected: rgb(0xFFB03B),
            outline: rgb(0xA9C4E6),
            grid: GridColors {
                minor: rgb(0x2A2A2E),
                major: rgb(0x38383D),
                axis: rgb(0x55555C),
            },
            shadow: [0.0, 0.0, 0.0, 0.35],
        }
    } else {
        Theme {
            avatar: rgb(0x8A7A6C),
            pattern: rgb(0x3B6EB0),
            selected: rgb(0xD97706),
            outline: rgb(0x2C558C),
            grid: GridColors {
                minor: rgb(0xE8E8EB),
                major: rgb(0xD4D4D8),
                axis: rgb(0xA1A1AA),
            },
            shadow: [0.0, 0.0, 0.0, 0.18],
        }
    }
}

/// The background, as sRGB bytes (the target isn't an sRGB format).
fn clear(dark: bool) -> wgpu::Color {
    let bg: u32 = if dark { 0x1E1E21 } else { 0xF7F7F8 };
    let channel = |shift: u32| f64::from((bg >> shift) as u8) / 255.0;
    wgpu::Color {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

fn encode_png(pixels: &[u8], size: [u32; 2]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(pixels, size[0], size[1], image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("couldn't encode the picture: {e}"))?;
    Ok(out)
}

/// Standard base64, for sending a picture inside JSON.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn strain_runs_from_blue_to_red() {
        let relaxed = strain_color(0.0);
        let torn = strain_color(0.5);
        assert!(relaxed[2] > relaxed[0] && torn[0] > torn[2]);
    }
}
