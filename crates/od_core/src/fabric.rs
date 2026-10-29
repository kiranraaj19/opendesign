//! How a [`Fabric`] resists being pulled or bent in a given direction
//! relative to its grain, and how older files without per-direction
//! values are read; and the fabric editor's view of those values.

use serde::Deserialize;

use crate::command::Command;
use crate::document::{Document, Fabric, FabricTexture, Pattern, PatternId};
use crate::geometry::Vec2;

/// What a saved [`Fabric`] may contain. Fabrics saved before stretch and
/// bending were set per direction have one value for each; those load
/// with the same value in every direction, so they drape as they did.
#[derive(Deserialize)]
pub(crate) struct FabricFields {
    name: String,
    mass_per_area: f32,
    stretch_compliance: f32,
    #[serde(default)]
    weft_stretch_compliance: Option<f32>,
    #[serde(default)]
    shear_compliance: Option<f32>,
    bending_compliance: f32,
    #[serde(default)]
    weft_bending_compliance: Option<f32>,
    friction: f32,
    color: [u8; 3],
    texture: FabricTexture,
}

impl From<FabricFields> for Fabric {
    fn from(f: FabricFields) -> Self {
        Self {
            name: f.name,
            mass_per_area: f.mass_per_area,
            stretch_compliance: f.stretch_compliance,
            weft_stretch_compliance: f.weft_stretch_compliance.unwrap_or(f.stretch_compliance),
            shear_compliance: f.shear_compliance.unwrap_or(f.stretch_compliance),
            bending_compliance: f.bending_compliance,
            weft_bending_compliance: f.weft_bending_compliance.unwrap_or(f.bending_compliance),
            friction: f.friction,
            color: f.color,
            texture: f.texture,
        }
    }
}

/// Squared cosine and sine of the angle between `direction` and `grain`,
/// or `None` if either has no length.
fn cos_sin_squared(direction: Vec2, grain: Vec2) -> Option<(f32, f32)> {
    let lengths = direction.dot(direction) * grain.dot(grain);
    if lengths <= 1e-24 {
        return None;
    }
    let dot = direction.dot(grain);
    let cos2 = (dot * dot / lengths).clamp(0.0, 1.0);
    Some((cos2, 1.0 - cos2))
}

/// `parts` (weight, compliance) mixed by weight; the weights sum to one.
fn mix(parts: &[(f32, f32)]) -> f32 {
    parts
        .iter()
        .map(|&(weight, compliance)| weight * compliance)
        .sum()
}

impl Fabric {
    /// A fabric that stretches and bends the same in every direction, as
    /// every fabric did before they were set per direction.
    pub fn is_isotropic(&self) -> bool {
        self.weft_stretch_compliance == self.stretch_compliance
            && self.shear_compliance == self.stretch_compliance
            && self.weft_bending_compliance == self.bending_compliance
    }

    /// Compliance against stretching for a constraint lying along
    /// `direction`, on a piece whose warp runs along `grain` (both in the
    /// pattern's flat space). As for any orthotropic sheet pulled at angle
    /// θ to its main axis, it mixes the warp, weft and bias (shear) values
    /// by cos⁴θ, sin⁴θ and 2·sin²θ·cos²θ, which sum to one: along the
    /// grain it is the warp value, across it the weft value, and on the
    /// bias half shear.
    pub fn stretch_compliance_at(&self, direction: Vec2, grain: Vec2) -> f32 {
        let Some((c2, s2)) = cos_sin_squared(direction, grain) else {
            return self.stretch_compliance;
        };
        mix(&[
            (c2 * c2, self.stretch_compliance),
            (s2 * s2, self.weft_stretch_compliance),
            (2.0 * c2 * s2, self.shear_compliance),
        ])
    }

    /// Compliance against bending for a bending constraint spanning
    /// `direction` (across the fold it resists): along the grain it bends
    /// the warp threads, across it the weft, in between a mix of both by
    /// cos²θ and sin²θ.
    pub fn bending_compliance_at(&self, direction: Vec2, grain: Vec2) -> f32 {
        let Some((c2, s2)) = cos_sin_squared(direction, grain) else {
            return self.bending_compliance;
        };
        mix(&[
            (c2, self.bending_compliance),
            (s2, self.weft_bending_compliance),
        ])
    }
}

impl Pattern {
    /// Which way the grainline (the warp) runs, as a unit vector in the
    /// pattern's flat space: straight down at 0°, turning clockwise on
    /// screen as [`Pattern::grain_angle`] grows.
    pub fn grain_direction(&self) -> Vec2 {
        let (sin, cos) = self.grain_angle.to_radians().sin_cos();
        Vec2::new(-sin, cos)
    }
}

/// One of a fabric's physical values, as the fabric editor shows it: in
/// plain units (g/m², a friction coefficient), or for the compliances a
/// firmness from 0 (as soft as the editor goes) to 100 (as firm), on a
/// log scale since useful values span several orders of magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FabricValue {
    /// Weight, g/m².
    Weight,
    /// Firmness against stretching along the grain (warp).
    StretchWarp,
    /// Firmness against stretching across the grain (weft).
    StretchWeft,
    /// Firmness against shearing, for pulls on the bias.
    Shear,
    /// Firmness against folding the warp threads.
    BendWarp,
    /// Firmness against folding the weft threads.
    BendWeft,
    /// Friction coefficient, 0..1.
    Friction,
}

/// Softest and firmest stretch and shear compliances the editor offers.
const STRETCH_RANGE: (f32, f32) = (1e-3, 1e-9);
/// Softest and firmest bending compliances the editor offers.
const BEND_RANGE: (f32, f32) = (1e-2, 1e-7);

/// A compliance as a firmness from 0 (`soft`) to 100 (`firm`).
fn firmness(compliance: f32, (soft, firm): (f32, f32)) -> f32 {
    let t = (soft.ln() - compliance.max(1e-30).ln()) / (soft.ln() - firm.ln());
    (t * 100.0).clamp(0.0, 100.0)
}

/// The compliance at `firmness` 0..100 between `soft` and `firm`.
fn compliance(firmness: f32, (soft, firm): (f32, f32)) -> f32 {
    let t = firmness.clamp(0.0, 100.0) / 100.0;
    (soft.ln() + (firm.ln() - soft.ln()) * t).exp()
}

impl FabricValue {
    pub const ALL: [FabricValue; 7] = [
        FabricValue::Weight,
        FabricValue::StretchWarp,
        FabricValue::StretchWeft,
        FabricValue::Shear,
        FabricValue::BendWarp,
        FabricValue::BendWeft,
        FabricValue::Friction,
    ];

    /// What the editor calls it.
    pub fn label(self) -> &'static str {
        match self {
            FabricValue::Weight => "Weight",
            FabricValue::StretchWarp => "Stretch firmness along grain",
            FabricValue::StretchWeft => "Stretch firmness across grain",
            FabricValue::Shear => "Stretch firmness on the bias",
            FabricValue::BendWarp => "Bending stiffness along grain",
            FabricValue::BendWeft => "Bending stiffness across grain",
            FabricValue::Friction => "Friction",
        }
    }

    /// A sentence explaining it, for a tooltip.
    pub fn help(self) -> &'static str {
        match self {
            FabricValue::Weight => "How heavy a square metre of the fabric is",
            FabricValue::StretchWarp => {
                "How hard it is to stretch along the grainline (the warp threads): \
                 0 stretches easily, 100 hardly at all"
            }
            FabricValue::StretchWeft => {
                "How hard it is to stretch across the grainline (the weft threads)"
            }
            FabricValue::Shear => {
                "How hard it is to stretch diagonally to the grainline; \
                 woven fabrics give most on the bias"
            }
            FabricValue::BendWarp => {
                "How much it resists folding along the grainline: \
                 0 hangs limp, 100 holds its shape like canvas"
            }
            FabricValue::BendWeft => "How much it resists folding across the grainline",
            FabricValue::Friction => "How much it grips other surfaces, 0 to 1",
        }
    }

    /// The unit shown after the number.
    pub fn suffix(self) -> &'static str {
        match self {
            FabricValue::Weight => " g/m²",
            FabricValue::Friction => "",
            _ => " / 100",
        }
    }

    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            FabricValue::Weight => 10.0..=1500.0,
            FabricValue::Friction => 0.0..=1.0,
            _ => 0.0..=100.0,
        }
    }

    /// `fabric`'s value, in the editor's units.
    pub fn get(self, fabric: &Fabric) -> f32 {
        match self {
            FabricValue::Weight => fabric.mass_per_area * 1000.0,
            FabricValue::StretchWarp => firmness(fabric.stretch_compliance, STRETCH_RANGE),
            FabricValue::StretchWeft => firmness(fabric.weft_stretch_compliance, STRETCH_RANGE),
            FabricValue::Shear => firmness(fabric.shear_compliance, STRETCH_RANGE),
            FabricValue::BendWarp => firmness(fabric.bending_compliance, BEND_RANGE),
            FabricValue::BendWeft => firmness(fabric.weft_bending_compliance, BEND_RANGE),
            FabricValue::Friction => fabric.friction,
        }
    }

    /// `fabric` with this value set to `value`, in the editor's units.
    pub fn set(self, fabric: &Fabric, value: f32) -> Fabric {
        let mut f = fabric.clone();
        let value = value.clamp(*self.range().start(), *self.range().end());
        match self {
            FabricValue::Weight => f.mass_per_area = value / 1000.0,
            FabricValue::StretchWarp => f.stretch_compliance = compliance(value, STRETCH_RANGE),
            FabricValue::StretchWeft => {
                f.weft_stretch_compliance = compliance(value, STRETCH_RANGE)
            }
            FabricValue::Shear => f.shear_compliance = compliance(value, STRETCH_RANGE),
            FabricValue::BendWarp => f.bending_compliance = compliance(value, BEND_RANGE),
            FabricValue::BendWeft => f.weft_bending_compliance = compliance(value, BEND_RANGE),
            FabricValue::Friction => f.friction = value,
        }
        f
    }
}

/// Change a pattern's fabric in place, e.g. one value from the fabric
/// editor; like [`crate::SetFabric`], with a label saying what changed.
pub struct EditFabric {
    id: PatternId,
    /// Swapped with the pattern's fabric on `apply`/`undo`.
    fabric: Fabric,
    what: String,
}

impl EditFabric {
    pub fn new(id: PatternId, fabric: Fabric, what: impl Into<String>) -> Self {
        Self {
            id,
            fabric,
            what: what.into(),
        }
    }
}

impl Command for EditFabric {
    fn label(&self) -> String {
        format!("Edit fabric: {}", self.what)
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(p) = doc.pattern_mut(self.id) {
            std::mem::swap(&mut p.fabric, &mut self.fabric);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_values_round_trip_through_the_fabric() {
        for preset in Fabric::presets() {
            for value in FabricValue::ALL {
                let shown = value.get(&preset);
                assert!(value.range().contains(&shown), "{value:?} {shown}");
                let back = value.get(&value.set(&preset, shown));
                assert!((back - shown).abs() < 1e-3, "{value:?}: {shown} -> {back}");
            }
        }
        // Firmer is stiffer (less compliant), and the ends are the range's.
        let f = Fabric::cotton();
        let firm = FabricValue::StretchWeft.set(&f, 100.0);
        let soft = FabricValue::StretchWeft.set(&f, 0.0);
        assert!((firm.weft_stretch_compliance / 1e-9 - 1.0).abs() < 1e-3);
        assert!((soft.weft_stretch_compliance / 1e-3 - 1.0).abs() < 1e-3);
        assert_eq!(firm.stretch_compliance, f.stretch_compliance, "only weft");
        // Out of range is clamped.
        assert_eq!(FabricValue::Friction.set(&f, 3.0).friction, 1.0);
        assert_eq!(FabricValue::Weight.set(&f, 200.0).mass_per_area, 0.2);
    }

    #[test]
    fn edit_fabric_undoes_exactly() {
        let mut doc = Document::new("t");
        doc.patterns
            .push(Pattern::new(PatternId(1), "p", Vec::new()));
        let before = doc.patterns[0].fabric.clone();
        let edited = FabricValue::Shear.set(&before, 10.0);
        let mut cmd = EditFabric::new(PatternId(1), edited.clone(), "bias");
        cmd.apply(&mut doc);
        assert_eq!(doc.patterns[0].fabric, edited);
        assert_eq!(cmd.label(), "Edit fabric: bias");
        cmd.undo(&mut doc);
        assert_eq!(doc.patterns[0].fabric, before);
        cmd.apply(&mut doc);
        assert_eq!(doc.patterns[0].fabric, edited);
    }

    fn directional() -> Fabric {
        Fabric {
            stretch_compliance: 1e-8,
            weft_stretch_compliance: 1e-5,
            shear_compliance: 1e-4,
            bending_compliance: 1e-6,
            weft_bending_compliance: 1e-3,
            ..Fabric::cotton()
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= b.abs() * 1e-4
    }

    #[test]
    fn along_and_across_the_grain_read_the_warp_and_weft_values() {
        let f = directional();
        let grain = Vec2::new(0.0, 1.0);
        assert!(close(
            f.stretch_compliance_at(Vec2::new(0.0, 5.0), grain),
            1e-8
        ));
        assert!(close(
            f.stretch_compliance_at(Vec2::new(0.0, -5.0), grain),
            1e-8
        ));
        assert!(close(
            f.stretch_compliance_at(Vec2::new(3.0, 0.0), grain),
            1e-5
        ));
        assert!(close(
            f.bending_compliance_at(Vec2::new(0.0, 1.0), grain),
            1e-6
        ));
        assert!(close(
            f.bending_compliance_at(Vec2::new(1.0, 0.0), grain),
            1e-3
        ));
    }

    #[test]
    fn the_bias_mixes_in_shear() {
        let f = directional();
        let grain = Vec2::new(0.0, 1.0);
        let bias = f.stretch_compliance_at(Vec2::new(1.0, 1.0), grain);
        // Half of it is shear, so it's far softer than the warp.
        assert!(close(bias, 0.25e-8 + 0.25e-5 + 0.5e-4), "{bias}");
        let softer_shear = Fabric {
            shear_compliance: 1.0,
            ..f.clone()
        };
        assert!(softer_shear.stretch_compliance_at(Vec2::new(1.0, 1.0), grain) > bias);
        // Along the grain shear plays no part.
        assert!(close(
            softer_shear.stretch_compliance_at(Vec2::new(0.0, 1.0), grain),
            1e-8
        ));
    }

    #[test]
    fn an_isotropic_fabric_is_the_same_every_way() {
        let f = Fabric {
            weft_stretch_compliance: 1e-7,
            shear_compliance: 1e-7,
            weft_bending_compliance: 1e-5,
            ..Fabric::cotton()
        };
        assert!(f.is_isotropic());
        let grain = Vec2::new(0.3, 0.8);
        for d in [
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(-4.0, 1.0),
        ] {
            assert!(close(f.stretch_compliance_at(d, grain), 1e-7));
            assert!(close(f.bending_compliance_at(d, grain), 1e-5));
        }
        // No direction to go on: the warp value.
        assert_eq!(f.stretch_compliance_at(Vec2::ZERO, grain), 1e-7);
    }

    #[test]
    fn the_grain_direction_turns_clockwise_from_straight_down() {
        let mut p = Pattern::new(crate::document::PatternId(1), "p", Vec::new());
        let d = p.grain_direction();
        assert!(d.x.abs() < 1e-6 && (d.y - 1.0).abs() < 1e-6);
        // Clockwise on screen (y down) from straight down is towards -x.
        p.grain_angle = 90.0;
        let d = p.grain_direction();
        assert!((d.x + 1.0).abs() < 1e-6 && d.y.abs() < 1e-6, "{d:?}");
    }

    #[test]
    fn a_fabric_saved_without_directions_loads_the_same_every_way() {
        let json = r#"{"name":"Old","mass_per_area":0.2,"stretch_compliance":3e-7,
            "bending_compliance":2e-5,"friction":0.3,"color":[1,2,3],"texture":"plain"}"#;
        let f: Fabric = serde_json::from_str(json).unwrap();
        assert_eq!(f.weft_stretch_compliance, 3e-7);
        assert_eq!(f.shear_compliance, 3e-7);
        assert_eq!(f.weft_bending_compliance, 2e-5);
        assert!(f.is_isotropic());
        // And a full one round-trips exactly.
        let d = directional();
        let back: Fabric = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }
}
