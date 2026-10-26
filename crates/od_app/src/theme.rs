//! Design tokens: one set of named colours per mode, used for both the egui
//! widgets (via [`install`]) and the custom-painted canvases.

use egui::{Color32, Stroke, Theme, Visuals};

pub struct Palette {
    // Chrome
    pub app_bg: Color32,
    pub panel_bg: Color32,
    pub widget_bg: Color32,
    pub widget_hover: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub text: Color32,
    pub hint: Color32,
    // Canvases
    pub canvas_bg: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub axis: Color32,
    pub pattern_fill: Color32,
    pub pattern_stroke: Color32,
    pub selected: Color32,
    pub preview: Color32,
    pub avatar: Color32,
    pub warning: Color32,
}

pub const DARK: Palette = Palette {
    app_bg: Color32::from_rgb(0x16, 0x16, 0x19),
    panel_bg: Color32::from_rgb(0x23, 0x23, 0x27),
    widget_bg: Color32::from_rgb(0x2E, 0x2E, 0x33),
    widget_hover: Color32::from_rgb(0x3A, 0x3A, 0x40),
    border: Color32::from_rgb(0x38, 0x38, 0x3D),
    accent: Color32::from_rgb(0x3D, 0x8B, 0xFD),
    text: Color32::from_rgb(0xE4, 0xE4, 0xE7),
    hint: Color32::from_rgb(0x8B, 0x8B, 0x93),
    canvas_bg: Color32::from_rgb(0x1E, 0x1E, 0x21),
    grid_minor: Color32::from_rgb(0x2A, 0x2A, 0x2E),
    grid_major: Color32::from_rgb(0x38, 0x38, 0x3D),
    axis: Color32::from_rgb(0x55, 0x55, 0x5C),
    pattern_fill: Color32::from_rgba_unmultiplied_const(0x7A, 0x9C, 0xC6, 0x50),
    pattern_stroke: Color32::from_rgb(0xA9, 0xC4, 0xE6),
    selected: Color32::from_rgb(0xFF, 0xB0, 0x3B),
    preview: Color32::from_rgb(0x6E, 0xE7, 0xB7),
    avatar: Color32::from_rgb(0x9C, 0x8F, 0x84),
    warning: Color32::from_rgb(0xF4, 0x56, 0x56),
};

pub const LIGHT: Palette = Palette {
    app_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    panel_bg: Color32::from_rgb(0xF4, 0xF4, 0xF5),
    widget_bg: Color32::from_rgb(0xE4, 0xE4, 0xE7),
    widget_hover: Color32::from_rgb(0xD4, 0xD4, 0xD8),
    border: Color32::from_rgb(0xD4, 0xD4, 0xD8),
    accent: Color32::from_rgb(0x25, 0x63, 0xEB),
    text: Color32::from_rgb(0x27, 0x27, 0x2A),
    hint: Color32::from_rgb(0x71, 0x71, 0x7A),
    canvas_bg: Color32::from_rgb(0xF7, 0xF7, 0xF8),
    grid_minor: Color32::from_rgb(0xE8, 0xE8, 0xEB),
    grid_major: Color32::from_rgb(0xD4, 0xD4, 0xD8),
    axis: Color32::from_rgb(0xA1, 0xA1, 0xAA),
    pattern_fill: Color32::from_rgba_unmultiplied_const(0x3B, 0x6E, 0xB0, 0x38),
    pattern_stroke: Color32::from_rgb(0x2C, 0x55, 0x8C),
    selected: Color32::from_rgb(0xD9, 0x77, 0x06),
    preview: Color32::from_rgb(0x05, 0x96, 0x69),
    avatar: Color32::from_rgb(0x8A, 0x7A, 0x6C),
    warning: Color32::from_rgb(0xDC, 0x26, 0x26),
};

pub fn palette(dark: bool) -> &'static Palette {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Map the tokens onto egui's widget styling.
pub fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.panel_bg;
    v.window_fill = p.panel_bg;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = p.app_bg;
    v.faint_bg_color = p.widget_bg;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel_bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    for state in [&mut w.inactive, &mut w.open] {
        state.bg_fill = p.widget_bg;
        state.weak_bg_fill = p.widget_bg;
        state.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.hovered.bg_fill = p.widget_hover;
    w.hovered.weak_bg_fill = p.widget_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.accent;
    v
}

/// Register both modes; egui switches between them with the theme preference.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, true));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_applies_tokens_to_both_modes() {
        let ctx = egui::Context::default();
        install(&ctx);
        for (theme, p) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
            let v = &ctx.style_of(theme).visuals;
            assert_eq!(v.panel_fill, p.panel_bg, "{theme:?} panel");
            assert_eq!(v.selection.bg_fill, p.accent, "{theme:?} accent");
            assert_eq!(v.widgets.inactive.bg_fill, p.widget_bg, "{theme:?} widget");
        }
    }

    #[test]
    fn modes_are_distinct() {
        assert_ne!(DARK.panel_bg, LIGHT.panel_bg);
        assert_ne!(DARK.text, LIGHT.text);
        assert_eq!(palette(true).panel_bg, DARK.panel_bg);
        assert_eq!(palette(false).panel_bg, LIGHT.panel_bg);
    }
}
