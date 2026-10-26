//! Colour tokens for the canvases. Panels and widgets use egui's own theme.

use egui::Color32;

pub struct Palette {
    pub canvas_bg: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub axis: Color32,
    pub pattern_fill: Color32,
    pub pattern_stroke: Color32,
    pub selected: Color32,
    pub preview: Color32,
    pub text: Color32,
    pub hint: Color32,
    pub avatar: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            canvas_bg: Color32::from_rgb(0x1E, 0x1E, 0x21),
            grid_minor: Color32::from_rgb(0x2A, 0x2A, 0x2E),
            grid_major: Color32::from_rgb(0x38, 0x38, 0x3D),
            axis: Color32::from_rgb(0x55, 0x55, 0x5C),
            pattern_fill: Color32::from_rgba_unmultiplied(0x7A, 0x9C, 0xC6, 0x50),
            pattern_stroke: Color32::from_rgb(0xA9, 0xC4, 0xE6),
            selected: Color32::from_rgb(0xFF, 0xB0, 0x3B),
            preview: Color32::from_rgb(0x6E, 0xE7, 0xB7),
            text: Color32::from_rgb(0xE4, 0xE4, 0xE7),
            hint: Color32::from_rgb(0x8B, 0x8B, 0x93),
            avatar: Color32::from_rgb(0x9C, 0x8F, 0x84),
        }
    } else {
        Palette {
            canvas_bg: Color32::from_rgb(0xF7, 0xF7, 0xF8),
            grid_minor: Color32::from_rgb(0xE8, 0xE8, 0xEB),
            grid_major: Color32::from_rgb(0xD4, 0xD4, 0xD8),
            axis: Color32::from_rgb(0xA1, 0xA1, 0xAA),
            pattern_fill: Color32::from_rgba_unmultiplied(0x3B, 0x6E, 0xB0, 0x38),
            pattern_stroke: Color32::from_rgb(0x2C, 0x55, 0x8C),
            selected: Color32::from_rgb(0xD9, 0x77, 0x06),
            preview: Color32::from_rgb(0x05, 0x96, 0x69),
            text: Color32::from_rgb(0x27, 0x27, 0x2A),
            hint: Color32::from_rgb(0x71, 0x71, 0x7A),
            avatar: Color32::from_rgb(0x8A, 0x7A, 0x6C),
        }
    }
}
