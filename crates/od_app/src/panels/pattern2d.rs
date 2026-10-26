//! 2D pattern window: draw and arrange flat garment pieces.
//!
//! Document units are millimetres; `zoom` is screen pixels per millimetre.

use egui::{Align2, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2 as Px};
use od_core::{geometry, MovePattern, PatternId, Vec2};

use crate::app::{Session, Tool};
use crate::theme;

/// Rectangles smaller than this (in mm) are treated as accidental clicks.
const MIN_RECT_MM: f32 = 5.0;

pub struct Canvas2D {
    pan: Px,
    zoom: f32,
    drag: Option<Drag>,
    /// Where the canvas was drawn last frame.
    rect: Rect,
}

enum Drag {
    Move { id: PatternId, delta: Vec2 },
    Rect { start: Vec2, end: Vec2 },
}

impl Default for Canvas2D {
    fn default() -> Self {
        Self {
            pan: Px::ZERO,
            zoom: 0.6,
            drag: None,
            rect: Rect::NOTHING,
        }
    }
}

impl Canvas2D {
    #[cfg(test)]
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Document point at the centre of the visible canvas.
    pub fn view_center(&self) -> Vec2 {
        if self.rect.is_positive() {
            self.to_doc(self.rect, self.rect.center())
        } else {
            Vec2::ZERO
        }
    }

    /// Screen position of a document point, as of the last frame.
    #[cfg(test)]
    pub fn screen_pos(&self, p: Vec2) -> Pos2 {
        self.to_screen(self.rect, p)
    }

    fn to_screen(&self, rect: Rect, p: Vec2) -> Pos2 {
        rect.center() + self.pan + Px::new(p.x, p.y) * self.zoom
    }

    fn to_doc(&self, rect: Rect, p: Pos2) -> Vec2 {
        let v = (p - rect.center() - self.pan) / self.zoom;
        Vec2::new(v.x, v.y)
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "2D pattern canvas")
    });
    let rect = response.rect;
    s.canvas.rect = rect;
    let pal = theme::palette(ui.visuals().dark_mode);

    // Zoom around the cursor so the point under it stays put.
    if let Some(hover) = response.hover_pos() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let anchor = s.canvas.to_doc(rect, hover);
            s.canvas.zoom = (s.canvas.zoom * (scroll * 0.002).exp()).clamp(0.05, 20.0);
            s.canvas.pan += hover - s.canvas.to_screen(rect, anchor);
        }
    }
    if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
        s.canvas.pan += response.drag_delta();
    }

    handle_primary(&response, rect, s);

    // Background and grid.
    painter.rect_filled(rect, 0.0, pal.canvas_bg);
    draw_grid(&painter, rect, &s.canvas, &pal);

    // Patterns, with the in-progress move shown as an offset preview.
    for pattern in &s.doc.patterns {
        let offset = match &s.canvas.drag {
            Some(Drag::Move { id, delta }) if *id == pattern.id => *delta,
            _ => Vec2::ZERO,
        };
        let points: Vec<Pos2> = pattern
            .outline
            .iter()
            .map(|p| s.canvas.to_screen(rect, *p + offset))
            .collect();
        let selected = s.selection == Some(pattern.id);
        let stroke = if selected {
            Stroke::new(2.5, pal.selected)
        } else {
            Stroke::new(1.5, pal.pattern_stroke)
        };
        // Placeholder fill: convex only. A real tessellator arrives with the wgpu renderer.
        painter.add(Shape::convex_polygon(
            points.clone(),
            pal.pattern_fill,
            Stroke::NONE,
        ));
        painter.add(Shape::closed_line(points.clone(), stroke));
        if selected {
            for p in &points {
                painter.circle_filled(*p, 3.5, pal.selected);
            }
        }
        if let Some((min, max)) = pattern.bounds() {
            let center = s
                .canvas
                .to_screen(rect, (min + max + offset + offset) * 0.5);
            painter.text(
                center,
                Align2::CENTER_CENTER,
                &pattern.name,
                FontId::proportional(13.0),
                pal.text,
            );
        }
    }

    if let Some(Drag::Rect { start, end }) = &s.canvas.drag {
        let points = geometry::rectangle(*start, *end)
            .into_iter()
            .map(|p| s.canvas.to_screen(rect, p))
            .collect();
        painter.add(Shape::closed_line(points, Stroke::new(1.5, pal.preview)));
        let size = *end - *start;
        let label = format!("{:.0} × {:.0} mm", size.x.abs(), size.y.abs());
        let at = s.canvas.to_screen(rect, *end) + Px::new(10.0, 10.0);
        painter.text(
            at,
            Align2::LEFT_TOP,
            label,
            FontId::monospace(12.0),
            pal.preview,
        );
    }

    let hint = match s.tool {
        Tool::Select => "Click to select · drag to move",
        Tool::Rectangle => "Drag to draw a rectangle",
    };
    painter.text(
        rect.left_top() + Px::new(10.0, 8.0),
        Align2::LEFT_TOP,
        format!(
            "{hint} · scroll to zoom · right-drag to pan · {:.0}%",
            s.canvas.zoom * 100.0
        ),
        FontId::proportional(12.0),
        pal.hint,
    );
}

fn handle_primary(response: &egui::Response, rect: Rect, s: &mut Session) {
    let pointer = response
        .interact_pointer_pos()
        .map(|p| s.canvas.to_doc(rect, p));

    if response.clicked() && s.tool == Tool::Select {
        s.selection = pointer.and_then(|p| s.doc.pattern_at(p));
    }

    // egui reports a drag only after the pointer has moved a few pixels, so
    // anchor it at the press point and count the movement already made.
    let press = response.ctx.input(|i| i.pointer.press_origin());
    if response.drag_started_by(PointerButton::Primary) {
        if let (Some(origin), Some(p)) = (press.map(|o| s.canvas.to_doc(rect, o)), pointer) {
            s.canvas.drag = match s.tool {
                Tool::Select => {
                    s.selection = s.doc.pattern_at(origin);
                    s.selection.map(|id| Drag::Move {
                        id,
                        delta: p - origin,
                    })
                }
                Tool::Rectangle => Some(Drag::Rect {
                    start: origin,
                    end: p,
                }),
            };
        }
    } else if response.dragged_by(PointerButton::Primary) {
        let step = response.drag_delta() / s.canvas.zoom;
        match &mut s.canvas.drag {
            Some(Drag::Move { delta, .. }) => *delta += Vec2::new(step.x, step.y),
            Some(Drag::Rect { end, .. }) => {
                if let Some(p) = pointer {
                    *end = p;
                }
            }
            None => {}
        }
    }

    // Only a finished drag becomes a command, so one drag is one undo step.
    if response.drag_stopped() {
        match s.canvas.drag.take() {
            Some(Drag::Move { id, delta }) if delta != Vec2::ZERO => {
                s.execute(MovePattern::new(id, delta));
            }
            Some(Drag::Rect { start, end }) => {
                let size = end - start;
                if size.x.abs() >= MIN_RECT_MM && size.y.abs() >= MIN_RECT_MM {
                    let name = format!("Pattern {}", s.doc.patterns.len() + 1);
                    s.add_pattern(name, geometry::rectangle(start, end));
                }
            }
            _ => {}
        }
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect, canvas: &Canvas2D, pal: &theme::Palette) {
    // Pick a grid spacing that stays readable at any zoom.
    let mut step_mm = 10.0;
    while step_mm * canvas.zoom < 12.0 {
        step_mm *= 5.0;
    }
    let min = canvas.to_doc(rect, rect.left_top());
    let max = canvas.to_doc(rect, rect.right_bottom());

    let mut x = (min.x / step_mm).floor() * step_mm;
    while x <= max.x {
        let major = (x / step_mm).round() as i64 % 5 == 0;
        let color = if x == 0.0 {
            pal.axis
        } else if major {
            pal.grid_major
        } else {
            pal.grid_minor
        };
        let sx = canvas.to_screen(rect, Vec2::new(x, 0.0)).x;
        painter.line_segment(
            [Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())],
            Stroke::new(1.0, color),
        );
        x += step_mm;
    }
    let mut y = (min.y / step_mm).floor() * step_mm;
    while y <= max.y {
        let major = (y / step_mm).round() as i64 % 5 == 0;
        let color = if y == 0.0 {
            pal.axis
        } else if major {
            pal.grid_major
        } else {
            pal.grid_minor
        };
        let sy = canvas.to_screen(rect, Vec2::new(0.0, y)).y;
        painter.line_segment(
            [Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)],
            Stroke::new(1.0, color),
        );
        y += step_mm;
    }
}
