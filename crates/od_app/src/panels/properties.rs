//! Properties of the current selection, plus document-wide sizing (size
//! chart preview) and the bill of materials.

use od_core::{
    document_bom, total_area_m2, Axis, Fabric, Grading, Pattern, PatternId, PatternValue,
    PointKind, RemoveInternalLine, RenamePattern, SetFabric, SetGrading, SetParticleDistance,
    SetPatternValue, PARTICLE_DISTANCE_RANGE,
};
use od_sim::cloth::DEFAULT_PARTICLE_DISTANCE;

use crate::app::Session;

/// Edits in progress, so typing or dragging a value makes one undo step
/// when it is let go rather than one per keystroke or pixel.
#[derive(Default)]
pub struct PropertiesState {
    editing: Option<(PatternId, String)>,
    pending: Option<(PatternId, PatternValue, f32)>,
    /// A grading percentage being dragged but not yet committed, mirroring
    /// `pending` above.
    grading_pending: Option<(PatternId, f32)>,
    /// A particle distance being dragged or typed but not yet committed.
    distance_pending: Option<(PatternId, f32)>,
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
    let Some(pattern) = s.selection.and_then(|id| s.doc.pattern(id)) else {
        ui.add_space(8.0);
        ui.weak("Nothing selected.");
        ui.add_space(8.0);
        ui.label(format!("Document: {}", s.doc.name));
        ui.label(format!("Patterns: {}", s.doc.patterns.len()));
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
        ui.text_edit_singleline(name)
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

    fabric_section(ui, s, id);
    simulation_section(ui, s, id);
    grading_section(ui, s, id);

    if let Some(pattern) = s.doc.pattern(id) {
        point_section(ui, s, pattern.clone());
    }
    lines_section(ui, s, id);

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
    });

    ui.add_space(8.0);
    if ui.button("Delete pattern").clicked() {
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
                s.execute(SetPatternValue::new(id, field, value));
            }
        }
    }
}

/// Pick a fabric for the pattern from the built-in presets. Custom fabrics
/// aren't editable here (out of scope for this pass); the picker just
/// shows the current one, matched by name, with "Custom" if it was loaded
/// from a project that set something else.
fn fabric_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(fabric) = s.doc.pattern(id).map(|p| p.fabric.clone()) else {
        return;
    };
    ui.add_space(8.0);
    ui.label("Fabric");
    ui.horizontal_wrapped(|ui| {
        for preset in Fabric::presets() {
            let selected = preset.name == fabric.name;
            if ui.selectable_label(selected, &preset.name).clicked() && !selected {
                s.execute(SetFabric::new(id, preset));
            }
        }
    });
    ui.label(format!(
        "{:.0} g/m², friction {:.1}",
        fabric.mass_per_area * 1000.0,
        fabric.friction
    ));
}

/// How finely the pattern is meshed for draping: its own particle
/// distance, or the default. Takes effect the next time draping starts.
fn simulation_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(own) = s.doc.pattern(id).map(|p| p.particle_distance) else {
        return;
    };
    let current = own.unwrap_or(DEFAULT_PARTICLE_DISTANCE);
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
}

fn lines_section(ui: &mut egui::Ui, s: &mut Session, id: PatternId) {
    let Some(pattern) = s.doc.pattern(id) else {
        return;
    };
    if pattern.internal_lines.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label("Internal lines");
    let mut remove = None;
    for (i, line) in pattern.internal_lines.iter().enumerate() {
        let length: f32 = line.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        ui.horizontal(|ui| {
            ui.label(format!("Line {}: {length:.0} mm", i + 1));
            if ui.button(format!("Remove line {}", i + 1)).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.execute(RemoveInternalLine::new(id, i));
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
                let outline = pattern.outline.clone();
                let grading = if value == 0.0 {
                    Grading::default()
                } else {
                    Grading::uniform_scale(&outline, value)
                };
                s.execute(SetGrading::new(id, grading));
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
    });
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
