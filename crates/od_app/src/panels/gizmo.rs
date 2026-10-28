//! Moving and turning patterns in 3D before draping: a handle drawn over
//! the main selected pattern with an arrow per world axis and a ring for
//! turning about the vertical, plus placing patterns on the avatar's
//! arrangement points.

use egui::{Color32, Pos2, Rect, Stroke};
use od_core::{Arrangement, PatternId, SetArrangement};
use od_render::arrange::ArrangementPoint;
use od_render::glam::{Quat, Vec3};
use od_render::pattern::Placement;

use crate::app::{Session, Tool};

/// Length of each arrow, in metres.
const ARROW: f32 = 0.15;
/// Radius of the turning ring, in metres.
const RING: f32 = 0.11;
/// How close the pointer must be to a handle, in pixels.
const REACH_PX: f32 = 8.0;
/// Turn per pixel of horizontal drag on the ring, in radians.
const TURN_PER_PX: f32 = 0.01;

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(220, 70, 60),
    Color32::from_rgb(80, 180, 80),
    Color32::from_rgb(70, 120, 230),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Handle {
    /// Move along world X, Y or Z.
    Axis(usize),
    /// Turn about the vertical through the main pattern's centre.
    Turn,
}

/// A move or turn in progress, previewed until the drag ends.
#[derive(Debug, Clone, PartialEq)]
pub struct GizmoDrag {
    pub ids: Vec<PatternId>,
    handle: Handle,
    /// The main pattern's centre when the drag started.
    pivot: Vec3,
    offset: Vec3,
    turn: f32,
}

impl GizmoDrag {
    /// `p` moved and turned by this drag so far.
    pub fn apply(&self, p: Placement) -> Placement {
        let rotation = Quat::from_rotation_y(self.turn);
        Placement {
            center: self.pivot + rotation * (p.center - self.pivot) + self.offset,
            right: rotation * p.right,
            up: rotation * p.up,
        }
    }
}

/// Where the handle goes: the main selected pattern's centre, if it can be
/// moved (Select tool, before draping, unlocked).
pub fn centre(s: &Session, placements: &[(PatternId, Placement)]) -> Option<Vec3> {
    let v = &s.viewport;
    if s.tool != Tool::Select || v.drape.is_some() || v.pinning || v.arranging {
        return None;
    }
    let id = s.selection.primary()?;
    if s.doc.pattern(id)?.locked {
        return None;
    }
    placements
        .iter()
        .find(|(p, _)| *p == id)
        .map(|(_, at)| at.center)
}

/// The ring's points, for hit-testing and drawing.
fn ring(centre: Vec3) -> impl Iterator<Item = Vec3> {
    (0..=48).map(move |i| {
        let t = i as f32 / 48.0 * std::f32::consts::TAU;
        centre + Vec3::new(t.cos(), 0.0, t.sin()) * RING
    })
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// The handle under `pos`, if any.
pub fn hit(s: &Session, rect: Rect, centre: Vec3, pos: Pos2) -> Option<Handle> {
    let v = &s.viewport;
    let c = v.screen_pos(rect, centre)?;
    for (i, axis) in AXES.iter().enumerate() {
        let tip = v.screen_pos(rect, centre + *axis * ARROW)?;
        if distance_to_segment(pos, c, tip) <= REACH_PX {
            return Some(Handle::Axis(i));
        }
    }
    let ring: Vec<Pos2> = ring(centre).filter_map(|p| v.screen_pos(rect, p)).collect();
    let near_ring = ring
        .windows(2)
        .any(|w| distance_to_segment(pos, w[0], w[1]) <= REACH_PX);
    near_ring.then_some(Handle::Turn)
}

/// Start dragging `handle`: every selected, unlocked pattern moves.
pub fn begin(s: &Session, handle: Handle, centre: Vec3) -> GizmoDrag {
    GizmoDrag {
        ids: s.unlocked_selection(),
        handle,
        pivot: centre,
        offset: Vec3::ZERO,
        turn: 0.0,
    }
}

/// Follow the pointer having moved `delta` pixels this frame. Shift turns
/// in 15° steps.
pub fn update(s: &mut Session, rect: Rect, delta: egui::Vec2, total: egui::Vec2, coarse: bool) {
    let Some(mut drag) = s.viewport.gizmo.take() else {
        return;
    };
    match drag.handle {
        Handle::Axis(i) => {
            let v = &s.viewport;
            let from = drag.pivot + drag.offset;
            if let (Some(a), Some(b)) = (
                v.screen_pos(rect, from),
                v.screen_pos(rect, from + AXES[i] * ARROW),
            ) {
                // Pixels the arrow spans on screen, so a drag along it
                // moves the piece under the pointer.
                let along = b - a;
                let px = along.length();
                if px > 1.0 {
                    let metres = delta.dot(along / px) / px * ARROW;
                    drag.offset += AXES[i] * metres;
                }
            }
        }
        Handle::Turn => {
            let turn = total.x * TURN_PER_PX;
            let step = if coarse {
                15f32.to_radians()
            } else {
                1f32.to_radians()
            };
            drag.turn = (turn / step).round() * step;
        }
    }
    s.viewport.gizmo = Some(drag);
}

/// Finish the drag: one undo step saving every moved pattern's place.
pub fn end(s: &mut Session, placements: &[(PatternId, Placement)]) {
    let Some(drag) = s.viewport.gizmo.take() else {
        return;
    };
    if drag.offset == Vec3::ZERO && drag.turn == 0.0 {
        return;
    }
    let moved: Vec<(PatternId, Placement)> = placements
        .iter()
        .filter(|(id, _)| drag.ids.contains(id))
        .copied()
        .collect();
    let verb = match drag.handle {
        Handle::Axis(_) => "Move",
        Handle::Turn => "Turn",
    };
    let at: std::collections::HashMap<_, _> = moved.into_iter().collect();
    s.for_each(drag.ids.clone(), verb, "patterns in 3D", |id| {
        Box::new(SetArrangement::new(
            id,
            at.get(&id).map(|p| arrangement(*p)),
        ))
    });
}

/// A placement as saved in the document.
pub fn arrangement(p: Placement) -> Arrangement {
    Arrangement {
        center: p.center.into(),
        right: p.right.into(),
        up: p.up.into(),
    }
}

/// Turn each selected, unlocked pattern `degrees` about the vertical
/// through its own centre, from wherever it is placed now.
pub fn turn_selected(s: &mut Session, degrees: f32) {
    let rotation = Quat::from_rotation_y(degrees.to_radians());
    let at: std::collections::HashMap<_, _> = crate::panels::viewport3d::placements(s)
        .into_iter()
        .collect();
    let ids = s.unlocked_selection();
    s.for_each(ids, "Turn", "patterns in 3D", |id| {
        let turned = at.get(&id).map(|p| Placement {
            center: p.center,
            right: rotation * p.right,
            up: rotation * p.up,
        });
        Box::new(SetArrangement::new(id, turned.map(arrangement)))
    });
}

/// Give the selected, unlocked patterns back to the automatic layout.
pub fn reset_selected(s: &mut Session) {
    let ids = s.unlocked_selection();
    s.for_each(ids, "Reset", "3D positions", |id| {
        Box::new(SetArrangement::new(id, None))
    });
}

/// Put the selected, unlocked patterns on `point`.
pub fn arrange_at(s: &mut Session, point: &ArrangementPoint) {
    let at = arrangement(point.placement());
    let ids = s.unlocked_selection();
    s.for_each(ids, "Arrange", "patterns", |id| {
        Box::new(SetArrangement::new(id, Some(at)))
    });
}

/// The arrangement point under `pos`, if any.
pub fn point_at(s: &Session, rect: Rect, pos: Pos2) -> Option<ArrangementPoint> {
    let v = &s.viewport;
    v.arrangement_points()
        .into_iter()
        .filter_map(|p| Some((v.screen_pos(rect, p.position)?.distance(pos), p)))
        .filter(|(d, _)| *d <= REACH_PX + 4.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
}

pub fn draw(
    painter: &egui::Painter,
    s: &Session,
    rect: Rect,
    centre: Vec3,
    pal: &crate::theme::Palette,
) {
    let v = &s.viewport;
    let Some(c) = v.screen_pos(rect, centre) else {
        return;
    };
    let active = v.gizmo.as_ref().map(|g| g.handle);
    let ring: Vec<Pos2> = ring(centre).filter_map(|p| v.screen_pos(rect, p)).collect();
    let width = |handle| if active == Some(handle) { 3.0 } else { 2.0 };
    painter.add(egui::Shape::line(
        ring,
        Stroke::new(width(Handle::Turn), pal.selected),
    ));
    for (i, axis) in AXES.iter().enumerate() {
        if let Some(tip) = v.screen_pos(rect, centre + *axis * ARROW) {
            let stroke = Stroke::new(width(Handle::Axis(i)), AXIS_COLORS[i]);
            painter.line_segment([c, tip], stroke);
            painter.circle_filled(tip, 4.0, AXIS_COLORS[i]);
        }
    }
    if let Some(drag) = &v.gizmo {
        let text = match drag.handle {
            Handle::Axis(_) => format!("{:.0} mm", drag.offset.length() * 1000.0),
            Handle::Turn => format!("{:+.0}°", drag.turn.to_degrees()),
        };
        painter.text(
            c + egui::vec2(14.0, -14.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::monospace(12.0),
            pal.preview,
        );
    }
}

/// Markers for the arrangement points, named.
pub fn draw_points(painter: &egui::Painter, s: &Session, rect: Rect, pal: &crate::theme::Palette) {
    let v = &s.viewport;
    for point in v.arrangement_points() {
        if let Some(at) = v.screen_pos(rect, point.position) {
            painter.circle_filled(at, 6.0, pal.accent);
            painter.circle_stroke(at, 6.0, Stroke::new(1.0, pal.canvas_bg));
            painter.text(
                at + egui::vec2(9.0, 0.0),
                egui::Align2::LEFT_CENTER,
                point.name,
                egui::FontId::proportional(12.0),
                pal.hint,
            );
        }
    }
}
