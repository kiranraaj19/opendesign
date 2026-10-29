//! Preferences (Edit → Preferences): the grid, snapping, the defaults new
//! pieces get and the sewing-length check, remembered between sessions
//! the way the dock layout is. The theme is remembered by egui itself.

use crate::app::{SeamCheck, Session};
use crate::panels::pattern2d::Snap;

/// Storage key for [`Preferences`]. Fields missing from an older save
/// take their defaults, so the suffix only changes if one changes meaning.
pub const PREFS_KEY: &str = "preferences_v1";

/// The preferences saved last session, or the defaults.
pub fn restore(storage: Option<&dyn eframe::Storage>) -> Preferences {
    storage
        .and_then(|s| eframe::get_value(s, PREFS_KEY))
        .unwrap_or_default()
}

/// Defaults for new work that live nowhere else in the session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Defaults {
    /// Seam allowance of a new piece, in mm.
    pub seam_allowance: f32,
    /// Particle distance of pieces without their own, in mm.
    pub particle_distance: f32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            seam_allowance: od_core::document::DEFAULT_SEAM_ALLOWANCE,
            particle_distance: od_sim::cloth::DEFAULT_PARTICLE_DISTANCE,
        }
    }
}

/// Everything the Preferences window sets, as saved. While the app runs
/// the values live where they are used (the 2D canvas, the seam check,
/// [`Session::defaults`]); [`Preferences::capture`] gathers them to save
/// and [`Preferences::apply`] puts them back at start-up.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Finest 2D grid spacing, in mm, and the grid snap step.
    pub grid_mm: f32,
    pub snap_grid: bool,
    pub snap_points: bool,
    pub seam_allowance: f32,
    pub particle_distance: f32,
    pub check_lengths: bool,
    /// Sewing-length tolerance, in percent of the shorter side.
    pub length_tolerance: f32,
}

/// Bounds the window allows, also applied to values read back.
pub const GRID_RANGE: std::ops::RangeInclusive<f32> = 1.0..=100.0;
pub const SEAM_ALLOWANCE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=100.0;
pub const TOLERANCE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=50.0;

impl Default for Preferences {
    fn default() -> Self {
        let (snap, check, defaults) = (Snap::default(), SeamCheck::default(), Defaults::default());
        Self {
            grid_mm: crate::panels::pattern2d::DEFAULT_GRID_MM,
            snap_grid: snap.grid,
            snap_points: snap.points,
            seam_allowance: defaults.seam_allowance,
            particle_distance: defaults.particle_distance,
            check_lengths: check.enabled,
            length_tolerance: check.percent,
        }
    }
}

impl Preferences {
    pub fn capture(s: &Session) -> Self {
        Self {
            grid_mm: s.canvas.grid_mm,
            snap_grid: s.canvas.snap.grid,
            snap_points: s.canvas.snap.points,
            seam_allowance: s.defaults.seam_allowance,
            particle_distance: s.defaults.particle_distance,
            check_lengths: s.seam_check.enabled,
            length_tolerance: s.seam_check.percent,
        }
    }

    /// Put these preferences in place, clamped to what the window allows
    /// (a hand-edited or damaged save can hold anything).
    pub fn apply(&self, s: &mut Session) {
        let clamp = |v: f32, range: &std::ops::RangeInclusive<f32>, default: f32| {
            if v.is_finite() {
                v.clamp(*range.start(), *range.end())
            } else {
                default
            }
        };
        let d = Self::default();
        s.canvas.grid_mm = clamp(self.grid_mm, &GRID_RANGE, d.grid_mm);
        s.canvas.snap = Snap {
            grid: self.snap_grid,
            points: self.snap_points,
        };
        s.defaults = Defaults {
            seam_allowance: clamp(self.seam_allowance, &SEAM_ALLOWANCE_RANGE, d.seam_allowance),
            particle_distance: clamp(
                self.particle_distance,
                &od_core::PARTICLE_DISTANCE_RANGE,
                d.particle_distance,
            ),
        };
        s.seam_check = SeamCheck {
            enabled: self.check_lengths,
            percent: clamp(self.length_tolerance, &TOLERANCE_RANGE, d.length_tolerance),
        };
    }
}

/// The Preferences window's contents. Changes take effect at once.
pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    egui::Grid::new("preferences")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.strong("2D window");
            ui.end_row();
            ui.label("Grid size");
            ui.add(
                egui::DragValue::new(&mut s.canvas.grid_mm)
                    .range(GRID_RANGE)
                    .speed(0.5)
                    .suffix(" mm"),
            )
            .on_hover_text("Finest grid spacing, and the grid snap step; coarser when zoomed out");
            ui.end_row();
            ui.label("Snapping");
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.canvas.snap.grid, "Snap to grid");
                ui.checkbox(&mut s.canvas.snap.points, "Snap to points");
            });
            ui.end_row();

            ui.strong("New pieces");
            ui.end_row();
            ui.label("Seam allowance");
            ui.add(
                egui::DragValue::new(&mut s.defaults.seam_allowance)
                    .range(SEAM_ALLOWANCE_RANGE)
                    .speed(0.5)
                    .suffix(" mm"),
            )
            .on_hover_text("Given to pieces drawn or added from now on");
            ui.end_row();
            ui.label("Particle distance");
            ui.add(
                egui::DragValue::new(&mut s.defaults.particle_distance)
                    .range(od_core::PARTICLE_DISTANCE_RANGE)
                    .speed(0.5)
                    .suffix(" mm"),
            )
            .on_hover_text(
                "Simulation spacing for pieces without their own; used the next time draping starts",
            );
            ui.end_row();

            ui.strong("Sewing");
            ui.end_row();
            ui.label("Length check");
            ui.checkbox(&mut s.seam_check.enabled, "Check sewing length");
            ui.end_row();
            ui.label("Tolerance");
            ui.add_enabled(
                s.seam_check.enabled,
                egui::DragValue::new(&mut s.seam_check.percent)
                    .range(TOLERANCE_RANGE)
                    .speed(0.1)
                    .suffix(" %"),
            );
            ui.end_row();

            ui.strong("Appearance");
            ui.end_row();
            ui.label("Theme");
            egui::widgets::global_theme_preference_buttons(ui);
            ui.end_row();
        });
    ui.add_space(8.0);
    if ui
        .button("Restore defaults")
        .on_hover_text("Put every setting here back as it was when first installed")
        .clicked()
    {
        Preferences::default().apply(s);
    }
}
