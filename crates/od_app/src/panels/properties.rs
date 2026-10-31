//! Properties of the current selection, plus document-wide sizing (size
//! chart preview) and the bill of materials.

use od_core::document::TOLERANCE;
use od_core::edit::PleatKind;
use od_core::{
    document_bom, edit, total_area_m2, Axis, Dart, Fabric, Fold, Grading, Pattern, PatternId,
    PatternValue, PointKind, RemoveDart, RemoveInternalLine, RenamePattern, SetDart, SetFabric,
    SetFold, SetGrading, SetOutline, SetParticleDistance, SetPatternValue, PARTICLE_DISTANCE_RANGE,
};

use crate::app::{Session, Tool};

/// Edits in progress, so typing or dragging a value makes one undo step
/// when it is let go rather than one per keystroke or pixel.
pub struct PropertiesState {
    editing: Option<(PatternId, String)>,
    pending: Option<(PatternId, PatternValue, f32)>,
    /// A grading percentage being dragged but not yet committed, mirroring
    /// `pending` above.
    grading_pending: Option<(PatternId, f32)>,
    /// A particle distance being dragged or typed but not yet committed.
    distance_pending: Option<(PatternId, f32)>,
    /// How far from a corner "Round corner" starts the curve, in mm.
    round_distance: f32,
    /// An edge length being dragged or typed: pattern, the corner the edge
    /// starts at, and the length.
    length_pending: Option<(PatternId, usize, f32)>,
    /// A fold angle being dragged or typed: pattern, line and angle.
    fold_pending: Option<(PatternId, usize, f32)>,
    /// A fold strength being dragged or typed, as a percentage.
    strength_pending: Option<(PatternId, usize, f32)>,
    /// A diamond dart's length or width being dragged or typed.
    dart_pending: Option<((PatternId, usize, DartValue), f32)>,
}

impl Default for PropertiesState {
    fn default() -> Self {
        Self {
            editing: None,
            pending: None,
            grading_pending: None,
            distance_pending: None,
            round_distance: 20.0,
            length_pending: None,
            fold_pending: None,
            strength_pending: None,
            dart_pending: None,
        }
    }
}

impl PropertiesState {
    /// A value being dragged or typed but not yet committed, so the 2D
    /// window can show it live.
    pub fn preview(&self, id: PatternId, field: PatternValue) -> Option<f32> {
        self.pending
            .filter(|(pid, f, _)| *pid == id && *f == field)
            .map(|(_, _, v)| v)
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    size_section(ui, s);
    ui.add_space(8.0);
    bom_section(ui, s);
    tool_section(ui, s);
    crate::panels::measure2d::section(ui, s);

    if let Some(id) = s.selected_tack.filter(|&id| s.doc.tack(id).is_some()) {
        ui.add_space(8.0);
        tack_section(ui, s, id);
        return;
    }
    if let Some(id) = s
        .selected_seam
        .filter(|&id| s.doc.sewing_line(id).is_some())
    {
        ui.add_space(8.0);
        seam_section(ui, s, id);
        return;
    }
    if s.selection.len() > 1 {
        ui.add_space(8.0);
        several_section(ui, s);
        return;
    }
    let Some(pattern) = s.selection.primary().and_then(|id| s.doc.pattern(id)) else {
        ui.add_space(8.0);
        ui.weak("Nothing selected.");
        ui.add_space(8.0);
        ui.label(format!("Document: {}", s.doc.name));
        ui.label(format!("Patterns: {}", s.doc.patterns.len()));
        // Garment-wide, so shown with the document rather than a piece.
        crate::panels::colourways::ui(ui, s);
        return;
    };
    let id = pattern.id;

    ui.heading("Pattern");
    let name = match &mut s.props.editing {
        Some((editing_id, name)) if *editing_id == id => name,
        editing => &mut editing.insert((id, pattern.name.clone())).1,
    };
    let response = ui.horizontal(|ui| {
        ui.label("Name");
        // As wide as the panel, not egui's fixed default, which is wider
        // than the panel starts out and would widen every row below it.
        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
    });
    let committed = response.inner.lost_focus();
    let new_name = name.trim().to_owned();

    let curves = pattern
        .outline
        .iter()
        .filter(|p| p.kind == PointKind::Curve)
        .count();
    egui::Grid::new("pattern_props")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Points");
            ui.label(format!("{} ({curves} curve)", pattern.outline.len()));
            ui.end_row();
            if let Some((min, max)) = pattern.bounds() {
                ui.label("Size");
                ui.label(format!("{:.0} × {:.0} mm", max.x - min.x, max.y - min.y));
                ui.end_row();
            }
            ui.label("Area");
            ui.label(format!("{:.1} cm²", pattern.area() / 100.0));
            ui.end_row();
            ui.label("Perimeter");
            ui.label(format!("{:.0} mm", pattern.perimeter()));
            ui.end_row();
            ui.label("Notches");
            ui.label(pattern.notches.len().to_string());
            ui.end_row();
        });

    let unchanged = new_name.is_empty() || new_name == pattern.name;
    if committed {
        s.props.editing = None;
        if !unchanged {
            s.execute(RenamePattern::new(id, new_name));
        }
    }

    ui.add_space(8.0);
    egui::Grid::new("pattern_values")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Seam allowance");
            value_edit(ui, s, id, PatternValue::SeamAllowance, " mm", 0.0..=100.0);
            ui.end_row();
            ui.label("Grainline");
            value_edit(ui, s, id, PatternValue::GrainAngle, "°", -180.0..=180.0);
            ui.end_row();
        });

    link_section(ui, s, id);
    fabric_section(ui, s, id);
    simulation_section(ui, s, id);
    grading_section(ui, s, id);

    if let Some(pattern) = s.doc.pattern(id) {
        point_section(ui, s, pattern.clone());
    }
    lines_section(ui, s, id);
    darts_section(ui, s, id);
    crate::panels::details::pattern_section(ui, s, id);

    selection_actions(ui, s, "Delete pattern");
}

/// For a linked copy: what it follows, and a way to stop.
fn link_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(link) = s.doc.pattern(id).and_then(|p| p.link) else {
        return;
    };
    let source = s
        .doc
        .pattern(link.source)
        .map_or_else(|| "?".to_owned(), |p| p.name.clone());
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "Linked copy of {source}: editing either changes both"
        ));
        if ui
            .button("Unlink")
            .on_hover_text("Keep this shape, and stop following the other")
            .clicked()
        {
            s.execute(od_core::Unlink::new(id));
        }
    });
}

/// Settings for the tool in use, for tools that have any.
fn tool_section(ui: &mut egui::Ui, s: &mut Session) {
    fn size(value: &mut f32) -> egui::DragValue<'_> {
        egui::DragValue::new(value)
            .range(1.0..=1000.0)
            .suffix(" mm")
            .speed(0.5)
            .max_decimals(1)
    }
    match s.tool {
        Tool::Dart => {
            ui.add_space(8.0);
            ui.heading("Dart tool");
            egui::Grid::new("dart_tool").num_columns(2).show(ui, |ui| {
                ui.label("Width");
                ui.add(size(&mut s.canvas.dart_width));
                ui.end_row();
                ui.label("Depth (outline darts)");
                ui.add(size(&mut s.canvas.dart_depth));
                ui.end_row();
            });
        }
        Tool::Pleat => {
            ui.add_space(8.0);
            ui.heading("Pleat tool");
            let p = &mut s.canvas.pleat;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut p.kind, PleatKind::Knife, "Knife");
                ui.selectable_value(&mut p.kind, PleatKind::Box, "Box");
            });
            egui::Grid::new("pleat_tool").num_columns(2).show(ui, |ui| {
                ui.label("Pleats");
                ui.add(egui::DragValue::new(&mut p.count).range(1..=20));
                ui.end_row();
                ui.label("Depth");
                ui.add(size(&mut p.depth));
                ui.end_row();
                ui.label("Fold angle");
                ui.add(
                    egui::DragValue::new(&mut p.angle)
                        .range(0.0..=180.0)
                        .suffix("°"),
                );
                ui.end_row();
            });
            ui.checkbox(&mut p.tack, "Tack the top")
                .on_hover_text("Tack each pleat's folds together where the drag starts");
        }
        _ => crate::panels::details::tool_section(ui, s),
    }
}

/// Plain line, valley fold or mountain fold, and the fold's angle.
fn fold_edit(ui: &mut egui::Ui, s: &mut Session, id: PatternId, index: usize, fold: Option<Fold>) {
    let valley = fold.is_some_and(|f| f.angle >= 0.0);
    let mountain = fold.is_some_and(|f| f.angle < 0.0);
    let angle = fold.map_or(Fold::DEFAULT_ANGLE, |f| f.angle.abs());
    let strength = fold.map_or(Fold::DEFAULT_STRENGTH, |f| f.strength);
    let mut set = None;
    if ui.selectable_label(fold.is_none(), "Plain").clicked() && fold.is_some() {
        set = Some(None);
    }
    if ui.selectable_label(valley, "Valley").clicked() && !valley {
        set = Some(Some(Fold { angle, strength }));
    }
    if ui.selectable_label(mountain, "Mountain").clicked() && !mountain {
        set = Some(Some(Fold {
            angle: -angle,
            strength,
        }));
    }
    if let Some(fold) = fold {
        let mut value = match s.props.fold_pending {
            Some((pid, i, v)) if pid == id && i == index => v,
            _ => angle,
        };
        let response = ui.add(
            egui::DragValue::new(&mut value)
                .range(0.0..=180.0)
                .suffix("°"),
        );
        if response.changed() {
            s.props.fold_pending = Some((id, index, value));
        }
        let busy = response.dragged() || response.has_focus();
        if !busy {
            if let Some((pid, i, value)) = s.props.fold_pending.take() {
                if pid == id && i == index && value != angle {
                    set = Some(Some(Fold {
                        angle: value * fold.angle.signum(),
                        ..fold
                    }));
                }
            }
        }

        // How firmly the fold holds its angle, as a percentage.
        let mut value = match s.props.strength_pending {
            Some((pid, i, v)) if pid == id && i == index => v,
            _ => fold.strength * 100.0,
        };
        let response = ui
            .add(
                egui::DragValue::new(&mut value)
                    .range(0.0..=100.0)
                    .suffix("% firm")
                    .max_decimals(0),
            )
            .on_hover_text("How firmly the fold holds its angle while draping");
        if response.changed() {
            s.props.strength_pending = Some((id, index, value));
        }
        let busy = response.dragged() || response.has_focus();
        if !busy {
            if let Some((pid, i, value)) = s.props.strength_pending.take() {
                let strength = value / 100.0;
                if pid == id && i == index && strength != fold.strength {
                    set = Some(Some(Fold { strength, ..fold }));
                }
            }
        }
    }
    if let Some(fold) = set {
        s.execute(SetFold::new(id, index, fold));
    }
}

/// The diamond darts inside the pattern.
fn darts_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(pattern) = s.doc.pattern(id) else {
        return;
    };
    if pattern.darts.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label("Darts");
    let darts = pattern.darts.clone();
    let mut remove = None;
    let mut set = None;
    for (i, dart) in darts.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Dart {}", i + 1));
            let length = dart.start.distance(dart.end);
            for (field, current, suffix) in [
                (DartValue::Length, length, " mm long"),
                (DartValue::Width, dart.width, " mm wide"),
            ] {
                let key = (id, i, field);
                let mut value = match s.props.dart_pending {
                    Some((k, v)) if k == key => v,
                    _ => current,
                };
                let response = ui.add(
                    egui::DragValue::new(&mut value)
                        .range(1.0..=2000.0)
                        .suffix(suffix)
                        .speed(0.5)
                        .max_decimals(1),
                );
                if response.changed() {
                    s.props.dart_pending = Some((key, value));
                }
                let busy = response.dragged() || response.has_focus();
                if !busy {
                    if let Some((_, value)) = s.props.dart_pending.take_if(|(k, _)| *k == key) {
                        if value != current {
                            set = Some((i, field.set(*dart, value)));
                        }
                    }
                }
            }
            if ui.button(format!("Remove dart {}", i + 1)).clicked() {
                remove = Some(i);
            }
        });
    }
    let (target, _) = s.edit_target(id);
    if let Some((i, dart)) = set {
        s.execute(SetDart::new(target, i, dart));
    }
    if let Some(i) = remove {
        s.execute(RemoveDart::new(target, i));
    }
}

/// Which of a diamond dart's sizes is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DartValue {
    /// Tip to tip; the dart stays centred where it was.
    Length,
    /// Across the middle.
    Width,
}

impl DartValue {
    fn set(self, dart: Dart, value: f32) -> Dart {
        match self {
            DartValue::Width => Dart {
                width: value,
                ..dart
            },
            DartValue::Length => {
                let mid = dart.start.lerp(dart.end, 0.5);
                let d = dart.end - dart.start;
                let len = d.length();
                if len < 1e-6 {
                    return dart;
                }
                let half = d * (value * 0.5 / len);
                Dart {
                    start: mid - half,
                    end: mid + half,
                    ..dart
                }
            }
        }
    }
}

/// Several patterns selected: their names, and what can be done to all of
/// them at once.
fn several_section(ui: &mut egui::Ui, s: &mut Session) {
    ui.heading(format!("{} patterns selected", s.selection.len()));
    for &id in s.selection.ids() {
        if let Some(pattern) = s.doc.pattern(id) {
            ui.label(&pattern.name);
        }
    }
    align_section(ui, s);
    selection_actions(ui, s, "Delete patterns");
}

/// Align edges or centres of the selected patterns, or spread them out.
fn align_section(ui: &mut egui::Ui, s: &mut Session) {
    ui.add_space(8.0);
    ui.label("Align");
    ui.horizontal_wrapped(|ui| crate::panels::context::align_buttons(ui, s));
}

/// Flip, copy and delete, acting on every selected pattern.
fn selection_actions(ui: &mut egui::Ui, s: &mut Session, delete: &str) {
    ui.add_space(8.0);
    ui.label("Rotate");
    ui.horizontal_wrapped(|ui| {
        for (label, degrees, tip) in [
            ("-90°", -90.0, "Turn a quarter turn anticlockwise"),
            ("-45°", -45.0, "Turn an eighth of a turn anticlockwise"),
            ("+45°", 45.0, "Turn an eighth of a turn clockwise"),
            ("+90°", 90.0, "Turn a quarter turn clockwise"),
        ] {
            if ui.button(label).on_hover_text(tip).clicked() {
                s.rotate_selected(degrees);
            }
        }
    });
    ui.add_space(8.0);
    ui.label("3D position");
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Turn 3D -45°")
            .on_hover_text("Turn about the vertical, in the 3D view")
            .clicked()
        {
            crate::panels::gizmo::turn_selected(s, -45.0);
        }
        if ui
            .button("Turn 3D +45°")
            .on_hover_text("Turn about the vertical, in the 3D view")
            .clicked()
        {
            crate::panels::gizmo::turn_selected(s, 45.0);
        }
        if ui
            .button("Reset 3D position")
            .on_hover_text("Go back to the automatic layout around the avatar")
            .clicked()
        {
            crate::panels::gizmo::reset_selected(s);
        }
    });
    ui.add_space(8.0);
    ui.label("Symmetry");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Flip left-right").clicked() {
            s.mirror_selected(Axis::Vertical);
        }
        if ui.button("Flip up-down").clicked() {
            s.mirror_selected(Axis::Horizontal);
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Mirrored copy")
            .on_hover_text("Add a mirror image beside this pattern, like the other half of a fold")
            .clicked()
        {
            s.duplicate_selected(Some(Axis::Vertical));
        }
        if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
            s.duplicate_selected(None);
        }
        if ui
            .button("Linked mirrored copy")
            .on_hover_text(
                "A mirror image beside this pattern that changes whenever either one is edited",
            )
            .clicked()
        {
            s.linked_copy_selected();
        }
    });

    ui.add_space(8.0);
    if ui.button(delete).clicked() {
        s.canvas.selected_point = None;
        s.delete_selected();
    }
}

/// A numeric property that commits one command when dragging or typing
/// ends.
fn value_edit(
    ui: &mut egui::Ui,
    s: &mut Session,
    id: PatternId,
    field: PatternValue,
    suffix: &str,
    range: std::ops::RangeInclusive<f32>,
) {
    let Some(current) = s.doc.pattern(id).map(|p| field.get(p)) else {
        return;
    };
    let mut value = s.props.preview(id, field).unwrap_or(current);
    let response = ui.add(
        egui::DragValue::new(&mut value)
            .range(range)
            .suffix(suffix)
            .speed(0.5)
            .max_decimals(1),
    );
    if response.changed() {
        s.props.pending = Some((id, field, value));
    }
    let busy = response.dragged() || response.has_focus();
    if !busy {
        if let Some(value) = s.props.preview(id, field) {
            s.props.pending = None;
            if value != current {
                // A linked copy's are its source's, the grainline mapped
                // back through the link.
                let (target, link) = s.edit_target(id);
                let value = match (link, field) {
                    (Some(link), PatternValue::GrainAngle) => link.grain_to_source(value),
                    _ => value,
                };
                s.execute(SetPatternValue::new(target, field, value));
            }
        }
    }
}

/// Pick a fabric for the pattern from the built-in presets, or edit its
/// values (see [`crate::panels::fabric_editor`]). A preset shows as
/// picked only while the fabric is exactly it, so picking it again after
/// an edit goes back to the preset's values.
fn fabric_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(fabric) = s.doc.pattern(id).map(|p| p.fabric.clone()) else {
        return;
    };
    ui.add_space(8.0);
    ui.label("Fabric");
    ui.horizontal_wrapped(|ui| {
        for preset in Fabric::presets() {
            let selected = preset == fabric;
            if ui.selectable_label(selected, &preset.name).clicked() && !selected {
                let (target, _) = s.edit_target(id);
                s.execute(SetFabric::new(target, preset));
            }
        }
    });
    ui.label(format!(
        "{:.0} g/m², friction {:.1}",
        fabric.mass_per_area * 1000.0,
        fabric.friction
    ));
    crate::panels::fabric_editor::ui(ui, s, id);
}

/// How finely the pattern is meshed for draping: its own particle
/// distance, or the default. Takes effect the next time draping starts.
fn simulation_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(own) = s.doc.pattern(id).map(|p| p.particle_distance) else {
        return;
    };
    let current = own.unwrap_or(s.defaults.particle_distance);
    ui.add_space(8.0);
    ui.label("Simulation");
    ui.horizontal(|ui| {
        ui.label("Particle distance");
        let mut value = match s.props.distance_pending {
            Some((pid, v)) if pid == id => v,
            _ => current,
        };
        let response = ui.add(
            egui::DragValue::new(&mut value)
                .range(PARTICLE_DISTANCE_RANGE)
                .suffix(" mm")
                .speed(0.5)
                .max_decimals(1),
        );
        if response.changed() {
            s.props.distance_pending = Some((id, value));
        }
        let busy = response.dragged() || response.has_focus();
        if !busy {
            if let Some((pid, value)) = s.props.distance_pending.take() {
                if pid == id && value != current {
                    s.execute(SetParticleDistance::new(id, Some(value)));
                }
            }
        }
        if own.is_some() && ui.button("Default").clicked() {
            s.execute(SetParticleDistance::new(id, None));
        }
    });
}

/// The point picked with the Edit points tool.
fn point_section(ui: &mut egui::Ui, s: &mut Session, pattern: Pattern) {
    let Some((pid, index)) = s.canvas.selected_point else {
        return;
    };
    let Some(point) = pattern.outline.get(index).filter(|_| pid == pattern.id) else {
        return;
    };
    ui.add_space(8.0);
    ui.label(format!(
        "Point {} at {:.1}, {:.1} mm",
        index + 1,
        point.pos.x,
        point.pos.y
    ));
    ui.horizontal(|ui| {
        let toggle = match point.kind {
            PointKind::Corner => "Make curve point",
            PointKind::Curve => "Make corner point",
        };
        if ui.button(toggle).on_hover_text("C").clicked() {
            s.toggle_selected_point();
        }
        if ui
            .add_enabled(pattern.outline.len() > 3, egui::Button::new("Delete point"))
            .clicked()
        {
            s.delete_selected();
        }
    });
    if point.kind != PointKind::Corner || pattern.locked {
        return;
    }
    // A linked copy's outline is its source's: edit that.
    let (target, _) = s.edit_target(pattern.id);
    let Some(pattern) = s.doc.pattern(target).cloned() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut s.props.round_distance)
                .range(1.0..=500.0)
                .suffix(" mm")
                .speed(0.5)
                .max_decimals(1),
        );
        if ui
            .button("Round corner")
            .on_hover_text("Replace the corner with a curve starting this far along each edge")
            .clicked()
        {
            if let Some(outline) =
                edit::round_corner(&pattern.outline, index, s.props.round_distance)
            {
                s.execute(SetOutline::new(pattern.id, "Round corner", outline));
                s.canvas.selected_point = None;
            }
        }
    });
    edge_length(ui, s, &pattern, index);
}

/// The length of the edge from the selected corner to the next one, which
/// can be typed in: the far end moves along the edge to match.
fn edge_length(ui: &mut egui::Ui, s: &mut Session, pattern: &Pattern, index: usize) {
    let (start, end) = pattern.segment_at(index as f32);
    if start as usize != index {
        return;
    }
    let current = pattern.segment_length(start, end, TOLERANCE);
    ui.horizontal(|ui| {
        ui.label("Edge to next corner");
        let mut value = match s.props.length_pending {
            Some((pid, i, v)) if pid == pattern.id && i == index => v,
            _ => current,
        };
        let response = ui.add(
            egui::DragValue::new(&mut value)
                .range(1.0..=10_000.0)
                .suffix(" mm")
                .speed(0.5)
                .max_decimals(1),
        );
        if response.changed() {
            s.props.length_pending = Some((pattern.id, index, value));
        }
        let busy = response.dragged() || response.has_focus();
        if !busy {
            if let Some((pid, i, value)) = s.props.length_pending.take() {
                let changed = (value - current).abs() > 1e-3;
                if pid == pattern.id && i == index && changed {
                    if let Some(outline) = edit::with_edge_length(pattern, index, value) {
                        s.execute(SetOutline::new(pattern.id, "Set edge length", outline));
                    }
                }
            }
        }
    });
}

fn lines_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(pattern) = s.doc.pattern(id).cloned() else {
        return;
    };
    if pattern.internal_lines.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label("Internal lines");
    // A linked copy's lines are its source's, in the same order: edits go
    // there (cutting cuts the copy itself, which unlinks it).
    let (target, _) = s.edit_target(id);
    let source = s.doc.pattern(target).cloned().unwrap_or(pattern.clone());
    let mut remove = None;
    let mut split = None;
    let mut cut = None;
    for (i, line) in pattern.internal_lines.iter().enumerate() {
        let source_line = source.internal_lines.get(i).unwrap_or(line);
        let length: f32 = line.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Line {}: {length:.0} mm", i + 1));
            if ui.button(format!("Remove line {}", i + 1)).clicked() {
                remove = Some(i);
            }
            if !pattern.locked {
                fold_edit(ui, s, target, i, line.fold);
            }
            let crossings = (!pattern.locked)
                .then(|| edit::points_at_crossings(&source, &source_line.points))
                .flatten();
            if let Some(outline) = crossings {
                if ui
                    .button("Add outline points")
                    .on_hover_text("Add a corner point wherever this line crosses the outline")
                    .clicked()
                {
                    split = Some(outline);
                }
            }
            if !pattern.locked && od_core::piece::can_cut(&s.doc, id, i) {
                if ui
                    .button("Cut")
                    .on_hover_text("Cut the pattern in two along this line")
                    .clicked()
                {
                    cut = Some((i, false));
                }
                if ui
                    .button("Cut and sew")
                    .on_hover_text(
                        "Cut the pattern in two along this line, and sew the halves back together",
                    )
                    .clicked()
                {
                    cut = Some((i, true));
                }
            }
        });
    }
    if let Some(i) = remove {
        s.execute(RemoveInternalLine::new(target, i));
    }
    if let Some(outline) = split {
        s.execute(SetOutline::new(
            target,
            "Add points at intersection",
            outline,
        ));
        s.canvas.selected_point = None;
    }
    if let Some((line, sew)) = cut {
        s.execute(od_core::CutPattern::new(id, line, sew));
        s.canvas.selected_point = None;
        // The new half goes straight after the one that keeps the id.
        let other = s
            .doc
            .index_of(id)
            .and_then(|i| s.doc.patterns.get(i + 1))
            .map(|p| p.id);
        s.selection.set_all([id].into_iter().chain(other));
    }
}

/// Pick a size to preview in the 2D canvas and the bill of materials.
/// Grading itself (per pattern) is set below, in [`grading_section`].
fn size_section(ui: &mut egui::Ui, s: &mut Session) {
    ui.heading("Size");
    ui.horizontal(|ui| {
        for (i, name) in s.doc.size_chart.names.clone().into_iter().enumerate() {
            let label = if i == s.doc.size_chart.base {
                format!("{name} (base)")
            } else {
                name
            };
            if ui.selectable_label(i == s.preview_size, label).clicked() {
                s.preview_size = i;
            }
        }
    });
}

/// A pattern's grading rule, as a single "% per size" slider driving
/// [`Grading::uniform_scale`] — real hand-graded rules can vary point to
/// point, but this is enough to demonstrate the grading pipeline end to
/// end (see `od_core::document::Grading`).
fn grading_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(pattern) = s.doc.pattern(id) else {
        return;
    };
    // The rule is stored as point deltas, not a percentage, so recover the
    // percentage that would reproduce it (deltas scale linearly with it).
    let current_percent = pattern
        .grading
        .point_deltas
        .iter()
        .zip(&pattern.outline)
        .map(|(d, p)| (*d, p.pos))
        .find(|(d, _)| d.length() > 1e-6)
        .map(|(d, pos)| {
            let center = pattern.center();
            let radius = pos - center;
            let denom = radius.dot(radius);
            if denom > 1e-6 {
                d.dot(radius) / denom * 100.0
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);

    ui.add_space(8.0);
    ui.label("Grading")
        .on_hover_text("How much this piece grows per size step, as a percentage move outward from its centre for every outline point.");
    let mut percent = s
        .props
        .grading_pending
        .filter(|(pid, _)| *pid == id)
        .map(|(_, v)| v)
        .unwrap_or(current_percent);
    let response = ui.add(
        egui::DragValue::new(&mut percent)
            .range(-50.0..=50.0)
            .suffix("% per size")
            .speed(0.1)
            .max_decimals(1),
    );
    if response.changed() {
        s.props.grading_pending = Some((id, percent));
    }
    let busy = response.dragged() || response.has_focus();
    if !busy {
        if let Some((_, value)) = s.props.grading_pending.take_if(|(pid, _)| *pid == id) {
            if (value - current_percent).abs() > 1e-4 {
                let (target, _) = s.edit_target(id);
                let outline = s
                    .doc
                    .pattern(target)
                    .map_or_else(|| pattern.outline.clone(), |p| p.outline.clone());
                let grading = if value == 0.0 {
                    Grading::default()
                } else {
                    Grading::uniform_scale(&outline, value)
                };
                s.execute(SetGrading::new(target, grading));
            }
        }
    }
}

/// The bill of materials for every pattern, at the size being previewed.
fn bom_section(ui: &mut egui::Ui, s: &mut Session) {
    if s.doc.patterns.is_empty() {
        return;
    }
    let steps = s.preview_steps();
    let boms = document_bom(&s.doc, steps);
    ui.heading("Bill of materials");
    egui::Grid::new("bom_table")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Piece");
            ui.label("Area");
            ui.label("Cut line");
            ui.end_row();
            for piece in &boms {
                ui.label(&piece.name);
                ui.label(format!("{:.2} m²", piece.area_m2));
                ui.label(format!("{:.0} mm", piece.perimeter_mm));
                ui.end_row();
            }
        });
    ui.label(format!("Total fabric: {:.2} m²", total_area_m2(&boms)));
}

/// A selected seam: its side lengths and what can be done with it.
fn seam_section(ui: &mut egui::Ui, s: &mut Session, id: od_core::SewingLineId) {
    use od_core::Side;
    let Some(line) = s.doc.sewing_line(id) else {
        return;
    };
    let (a, b) = line.side_lengths(&s.doc).unwrap_or_default();
    let flagged = s.seam_check.flags(&s.doc, line);
    let keep = line.a.first().map(|e| e.pattern);
    let names = |side| {
        let mut names: Vec<&str> = line
            .side(side)
            .iter()
            .filter_map(|e| s.doc.pattern(e.pattern).map(|p| p.name.as_str()))
            .collect();
        names.dedup();
        names.join(", ")
    };
    let (names_a, names_b) = (names(Side::A), names(Side::B));
    let kind = format!("{}:{}", line.a.len(), line.b.len());

    ui.heading("Seam");
    egui::Grid::new("seam_props")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Sewing");
            ui.label(kind);
            ui.end_row();
            ui.label("Side A");
            ui.label(format!("{a:.1} mm ({names_a})"));
            ui.end_row();
            ui.label("Side B");
            ui.label(format!("{b:.1} mm ({names_b})"));
            ui.end_row();
            ui.label("Difference");
            let diff = format!("{:.1} mm", (a - b).abs());
            if flagged {
                let pal = crate::theme::palette(ui.visuals().dark_mode);
                ui.label(egui::RichText::new(diff).color(pal.warning))
                    .on_hover_text(format!(
                        "More than {:.1}% of the shorter side (View menu)",
                        s.seam_check.percent
                    ));
            } else {
                ui.label(diff);
            }
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Flip side B")
            .on_hover_text("Sew side B the other way round, to untwist the seam")
            .clicked()
        {
            s.execute(od_core::FlipSeamSide::new(id, Side::B));
        }
        if ui.button("Delete seam").clicked() {
            s.execute(od_core::RemoveSewingLine::new(id));
            s.selected_seam = None;
        }
        if od_core::piece::can_merge(&s.doc, id)
            && ui
                .button("Merge pieces")
                .on_hover_text("Join the two sewn pieces into one along this seam")
                .clicked()
        {
            s.execute(od_core::MergePatterns::new(id));
            s.selected_seam = None;
            s.selection.set(keep);
        }
    });
    crate::panels::details::seam_section(ui, s, id);
}

/// A selected tack: what it holds to what.
fn tack_section(ui: &mut egui::Ui, s: &mut Session, id: od_core::TackId) {
    use od_core::TackTarget;
    let Some(tack) = s.doc.tack(id) else {
        return;
    };
    let name = |pattern| {
        s.doc
            .pattern(pattern)
            .map_or_else(|| "?".to_owned(), |p| p.name.clone())
    };
    let from = name(tack.a.pattern);
    let (to, distance) = match tack.b {
        TackTarget::Pattern(b) => (name(b.pattern), Some(tack.a.at.distance(b.at))),
        TackTarget::Avatar(_) | TackTarget::AvatarSurface { .. } => ("Avatar".to_owned(), None),
    };

    ui.heading("Tack");
    egui::Grid::new("tack_props")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("From");
            ui.label(from);
            ui.end_row();
            ui.label("To");
            ui.label(to);
            ui.end_row();
            if let Some(d) = distance {
                ui.label("Apart in 2D");
                ui.label(format!("{d:.0} mm"));
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    if ui.button("Delete tack").clicked() {
        s.execute(od_core::RemoveTack::new(id));
        s.selected_tack = None;
    }
}
