//! Procedural mannequins: smooth, closed meshes built from elliptical
//! cross-sections and sized from a few body measurements. Everything here
//! is generated, so there are no third-party avatar files to license.
//!
//! The avatar stands on the floor (y = 0) facing +Z, arms slightly out in
//! an A-pose. Each body part is its own closed surface; parts overlap where
//! they join, as on a shop mannequin.

use glam::Vec3;

use crate::mesh::Mesh;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvatarKind {
    Female,
    Male,
    Kid,
}

impl AvatarKind {
    pub const ALL: [AvatarKind; 3] = [AvatarKind::Female, AvatarKind::Male, AvatarKind::Kid];

    pub fn label(self) -> &'static str {
        match self {
            AvatarKind::Female => "Female",
            AvatarKind::Male => "Male",
            AvatarKind::Kid => "Kid",
        }
    }

    pub fn default_measurements(self) -> Measurements {
        let [height, chest, waist, hip] = match self {
            AvatarKind::Female => [168.0, 88.0, 70.0, 96.0],
            AvatarKind::Male => [178.0, 100.0, 86.0, 100.0],
            AvatarKind::Kid => [128.0, 64.0, 58.0, 66.0],
        };
        Measurements {
            height,
            chest,
            waist,
            hip,
        }
    }

    fn proportions(self) -> &'static Proportions {
        match self {
            AvatarKind::Female => &FEMALE,
            AvatarKind::Male => &MALE,
            AvatarKind::Kid => &KID,
        }
    }
}

/// Body measurements in centimetres; chest, waist and hip are girths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurements {
    pub height: f32,
    pub chest: f32,
    pub waist: f32,
    pub hip: f32,
}

impl Measurements {
    pub const HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 80.0..=220.0;
    pub const GIRTH_RANGE: std::ops::RangeInclusive<f32> = 40.0..=170.0;

    /// Keep every value in a range the generator handles well.
    pub fn clamped(self) -> Self {
        let girth = |v: f32| v.clamp(*Self::GIRTH_RANGE.start(), *Self::GIRTH_RANGE.end());
        Self {
            height: self
                .height
                .clamp(*Self::HEIGHT_RANGE.start(), *Self::HEIGHT_RANGE.end()),
            chest: girth(self.chest),
            waist: girth(self.waist),
            hip: girth(self.hip),
        }
    }
}

/// Where the measurements are taken, in metres above the floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landmarks {
    pub chest: f32,
    pub waist: f32,
    pub hip: f32,
}

/// Shape of one body type. Heights and lengths are fractions of the total
/// height; `*_depth` are depth/width ratios of a cross-section.
struct Proportions {
    crotch: f32,
    hip: f32,
    waist: f32,
    underbust: f32,
    chest: f32,
    armpit: f32,
    shoulder: f32,
    neck_base: f32,
    neck_top: f32,
    head_height: f32,
    head_half_width: f32,
    head_half_depth: f32,
    neck_radius: f32,
    shoulder_half_width: f32,
    chest_depth: f32,
    waist_depth: f32,
    hip_depth: f32,
    /// How far the chest section sits forward, as a fraction of its depth.
    bust: f32,
    /// Under-bust girth as a fraction of the chest girth.
    underbust_girth: f32,
    knee: f32,
    calf: f32,
    thigh: f32,
    ankle: f32,
    upper_arm: f32,
    forearm: f32,
    hand: f32,
}

const FEMALE: Proportions = Proportions {
    crotch: 0.47,
    hip: 0.515,
    waist: 0.625,
    underbust: 0.685,
    chest: 0.72,
    armpit: 0.745,
    shoulder: 0.815,
    neck_base: 0.835,
    neck_top: 0.86,
    head_height: 0.13,
    head_half_width: 0.044,
    head_half_depth: 0.054,
    neck_radius: 0.031,
    shoulder_half_width: 0.118,
    chest_depth: 0.80,
    waist_depth: 0.74,
    hip_depth: 0.80,
    bust: 0.10,
    underbust_girth: 0.89,
    knee: 0.285,
    calf: 0.20,
    thigh: 0.40,
    ankle: 0.045,
    upper_arm: 0.185,
    forearm: 0.15,
    hand: 0.105,
};

const MALE: Proportions = Proportions {
    crotch: 0.47,
    hip: 0.52,
    waist: 0.61,
    underbust: 0.68,
    chest: 0.72,
    armpit: 0.745,
    shoulder: 0.82,
    neck_base: 0.84,
    neck_top: 0.865,
    head_height: 0.13,
    head_half_width: 0.045,
    head_half_depth: 0.056,
    neck_radius: 0.034,
    shoulder_half_width: 0.128,
    chest_depth: 0.70,
    waist_depth: 0.74,
    hip_depth: 0.74,
    bust: 0.0,
    underbust_girth: 0.94,
    knee: 0.285,
    calf: 0.20,
    thigh: 0.40,
    ankle: 0.045,
    upper_arm: 0.19,
    forearm: 0.155,
    hand: 0.108,
};

const KID: Proportions = Proportions {
    crotch: 0.44,
    hip: 0.49,
    waist: 0.59,
    underbust: 0.655,
    chest: 0.69,
    armpit: 0.715,
    shoulder: 0.79,
    neck_base: 0.81,
    neck_top: 0.835,
    head_height: 0.16,
    head_half_width: 0.055,
    head_half_depth: 0.066,
    neck_radius: 0.034,
    shoulder_half_width: 0.115,
    chest_depth: 0.78,
    waist_depth: 0.80,
    hip_depth: 0.80,
    bust: 0.0,
    underbust_girth: 0.97,
    knee: 0.27,
    calf: 0.19,
    thigh: 0.37,
    ankle: 0.045,
    upper_arm: 0.18,
    forearm: 0.145,
    hand: 0.10,
};

/// Arms hang this far out from vertical.
const ARM_ANGLE: f32 = 16.0 * std::f32::consts::PI / 180.0;

/// Ellipse perimeter (Ramanujan's second approximation).
pub fn ellipse_perimeter(a: f32, b: f32) -> f32 {
    let h = ((a - b) / (a + b)).powi(2);
    std::f32::consts::PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()))
}

/// Semi-axes (half-width, half-depth) of an ellipse with this girth and
/// depth/width ratio.
fn section(girth: f32, depth: f32) -> (f32, f32) {
    let a = girth / ellipse_perimeter(1.0, depth);
    (a, a * depth)
}

pub fn landmarks(kind: AvatarKind, m: Measurements) -> Landmarks {
    let p = kind.proportions();
    let h = m.clamped().height / 100.0;
    Landmarks {
        chest: p.chest * h,
        waist: p.waist * h,
        hip: p.hip * h,
    }
}

/// The whole mannequin as one mesh.
pub fn avatar_mesh(kind: AvatarKind, m: Measurements) -> Mesh {
    Mesh::merged(&avatar_parts(kind, m))
}

/// Torso with head, then legs, feet and arms; each one closed.
pub fn avatar_parts(kind: AvatarKind, m: Measurements) -> Vec<Mesh> {
    let p = kind.proportions();
    let m = m.clamped();
    let h = m.height / 100.0;
    let (chest_a, chest_b) = section(m.chest / 100.0, p.chest_depth);
    let (waist_a, waist_b) = section(m.waist / 100.0, p.waist_depth);
    let (hip_a, hip_b) = section(m.hip / 100.0, p.hip_depth);
    let (under_a, under_b) = section(
        m.chest / 100.0 * p.underbust_girth,
        (p.chest_depth + p.waist_depth) * 0.5,
    );
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let z = |dz: f32| Vec3::new(0.0, 0.0, dz);

    // Torso, neck and head share one surface, swept up from the crotch.
    let neck = p.neck_radius * h;
    let head_hh = p.head_height * 0.5 * h;
    let head_y = h - head_hh;
    let (head_w, head_d) = (p.head_half_width * h, p.head_half_depth * h);
    let face = z(0.006 * h);
    let shoulder = p.shoulder_half_width * h;
    let head_ring = |theta: f32| {
        let (s, c) = theta.sin_cos();
        Key::new(head_y + head_hh * s, head_w * c, head_d * c, face)
    };
    let torso = Limb {
        origin: Vec3::ZERO,
        t: Vec3::Y,
        u: Vec3::X,
        segments: 48,
        subdivisions: 6,
        caps: [0.03 * h, head_hh],
        keys: vec![
            Key::new(p.crotch * h, hip_a * 0.8, hip_b * 0.68, z(-0.06 * hip_b)),
            Key::new(p.hip * h, hip_a, hip_b, z(-0.1 * hip_b)),
            Key::new(
                lerp(p.hip, p.waist, 0.5) * h,
                lerp(hip_a, waist_a, 0.55),
                lerp(hip_b, waist_b, 0.55),
                z(-0.06 * hip_b),
            ),
            Key::new(p.waist * h, waist_a, waist_b, z(0.0)),
            Key::new(p.underbust * h, under_a, under_b, z(p.bust * 0.4 * chest_b)),
            Key::new(p.chest * h, chest_a, chest_b, z(p.bust * chest_b)),
            Key::new(p.armpit * h, chest_a * 0.98, chest_b * 0.92, z(0.0)),
            Key::new(
                (p.shoulder - 0.02) * h,
                shoulder * 0.95,
                chest_b * 0.72,
                z(-0.05 * chest_b),
            ),
            Key::new(
                p.shoulder * h,
                shoulder * 0.82,
                chest_b * 0.58,
                z(-0.05 * chest_b),
            ),
            Key::new(p.neck_base * h, neck * 1.2, neck * 1.1, z(-0.2 * neck)),
            Key::new(p.neck_top * h, neck, neck * 0.95, z(0.0)),
            head_ring(-0.8),
            head_ring(-0.35),
            head_ring(0.0),
        ],
    };

    let mut parts = vec![torso.mesh()];

    for side in [1.0f32, -1.0] {
        // Legs: up from the ankle, the rounded top hidden in the hips.
        let leg_x = side * hip_a * 0.48;
        let leg = |y: f32, r: f32, depth: f32, dz: f32| {
            Key::new(y * h, r * hip_a, depth * hip_a, z(dz * h))
        };
        parts.push(
            Limb {
                origin: Vec3::new(leg_x, 0.0, 0.0),
                t: Vec3::Y,
                u: Vec3::X,
                segments: 28,
                subdivisions: 6,
                caps: [0.012 * h, 0.04 * h],
                keys: vec![
                    leg(p.ankle, 0.21, 0.22, 0.0),
                    leg(p.calf, 0.34, 0.34, -0.006),
                    leg(p.knee, 0.33, 0.33, 0.0),
                    leg(p.thigh, 0.50, 0.50, 0.0),
                    leg(p.crotch, 0.53, 0.6, -0.004),
                    leg(p.hip, 0.5, 0.62, -0.01),
                ],
            }
            .mesh(),
        );

        // Feet: an ellipsoid pointing forward, sole on the floor.
        let foot_h = 0.025 * h;
        parts.push(
            Limb {
                origin: Vec3::new(leg_x, foot_h, 0.0),
                t: Vec3::Z,
                u: Vec3::X,
                segments: 20,
                subdivisions: 1,
                caps: [0.045 * h, 0.105 * h],
                keys: vec![Key::new(0.0, 0.028 * h, foot_h, Vec3::ZERO)],
            }
            .mesh(),
        );

        // Arms: from the shoulder joint, tilted out, ending in a flat hand
        // with the palm towards the thigh.
        let r = 0.31 * chest_a;
        let (sin, cos) = ARM_ANGLE.sin_cos();
        let (upper, fore, hand) = (p.upper_arm * h, p.forearm * h, p.hand * h);
        let wrist = upper + fore;
        let arm = |s: f32, a: f32, b: f32| Key::new(s, a * chest_a, b * chest_a, Vec3::ZERO);
        parts.push(
            Limb {
                origin: Vec3::new(side * (shoulder - r), p.shoulder * h - 0.9 * r, 0.0),
                t: Vec3::new(side * sin, -cos, 0.0),
                u: Vec3::new(cos, side * sin, 0.0),
                segments: 28,
                subdivisions: 6,
                caps: [r, 0.15 * hand],
                keys: vec![
                    arm(0.0, 0.31, 0.31),
                    arm(upper * 0.5, 0.27, 0.28),
                    arm(upper, 0.21, 0.22),
                    arm(upper + fore * 0.4, 0.22, 0.21),
                    arm(wrist, 0.12, 0.16),
                    arm(wrist + hand * 0.2, 0.10, 0.25),
                    arm(wrist + hand * 0.55, 0.09, 0.25),
                    arm(wrist + hand * 0.85, 0.07, 0.18),
                ],
            }
            .mesh(),
        );
    }
    parts
}

/// One cross-section of a limb: position along the axis, semi-axes along
/// the limb's `u` and `v`, and a shift of the section's centre.
#[derive(Debug, Clone, Copy)]
struct Key {
    s: f32,
    a: f32,
    b: f32,
    offset: Vec3,
}

impl Key {
    fn new(s: f32, a: f32, b: f32, offset: Vec3) -> Self {
        Self { s, a, b, offset }
    }
}

/// A closed tube swept along a straight axis, with rounded ends.
struct Limb {
    origin: Vec3,
    /// Axis direction, from the first key to the last. Unit length.
    t: Vec3,
    /// First section axis, perpendicular to `t`. The second is `t × u`,
    /// which makes the triangles face outward.
    u: Vec3,
    segments: u32,
    /// Smooth sections per gap between keys.
    subdivisions: u32,
    /// Length of the rounded cap before the first and after the last key.
    caps: [f32; 2],
    keys: Vec<Key>,
}

impl Limb {
    /// The sections: keys smoothed with a Catmull-Rom spline (the axis
    /// position linearly, so sections never fold back), then
    /// quarter-ellipse caps.
    fn sections(&self) -> Vec<Key> {
        const CAP_RINGS: usize = 6;
        let keys = &self.keys;
        let n = keys.len();
        let at = |i: isize| keys[i.clamp(0, n as isize - 1) as usize];
        let catmull = |p0: f32, p1: f32, p2: f32, p3: f32, t: f32| {
            let (t2, t3) = (t * t, t * t * t);
            0.5 * (2.0 * p1
                + (p2 - p0) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
        };
        let mut body = Vec::new();
        for i in 0..n.saturating_sub(1) as isize {
            let [k0, k1, k2, k3] = [at(i - 1), at(i), at(i + 1), at(i + 2)];
            for step in 0..self.subdivisions {
                let t = step as f32 / self.subdivisions as f32;
                let c = |f: fn(&Key) -> f32| catmull(f(&k0), f(&k1), f(&k2), f(&k3), t);
                let offset = Vec3::new(c(|k| k.offset.x), c(|k| k.offset.y), c(|k| k.offset.z));
                body.push(Key {
                    s: k1.s + (k2.s - k1.s) * t,
                    a: c(|k| k.a).max(1e-3),
                    b: c(|k| k.b).max(1e-3),
                    offset,
                });
            }
        }
        body.push(keys[n - 1]);

        let cap = |key: Key, length: f32, dir: f32| {
            (1..CAP_RINGS).map(move |i| {
                let theta = i as f32 / CAP_RINGS as f32 * std::f32::consts::FRAC_PI_2;
                let (s, c) = theta.sin_cos();
                Key {
                    s: key.s + dir * length * s,
                    a: key.a * c,
                    b: key.b * c,
                    offset: key.offset,
                }
            })
        };
        let mut sections: Vec<Key> = cap(body[0], self.caps[0], -1.0).collect();
        sections.reverse();
        sections.extend_from_slice(&body);
        sections.extend(cap(body[body.len() - 1], self.caps[1], 1.0));
        sections
    }

    fn mesh(&self) -> Mesh {
        let sections = self.sections();
        let v = self.t.cross(self.u);
        let seg = self.segments;
        let centre = |k: &Key| self.origin + self.t * k.s + k.offset;

        let mut positions = Vec::with_capacity(sections.len() * seg as usize + 2);
        for k in &sections {
            let c = centre(k);
            for j in 0..seg {
                let phi = j as f32 / seg as f32 * std::f32::consts::TAU;
                positions.push(c + self.u * (k.a * phi.cos()) + v * (k.b * phi.sin()));
            }
        }
        let (first, last) = (&self.keys[0], &self.keys[self.keys.len() - 1]);
        let start = positions.len() as u32;
        positions.push(centre(first) - self.t * self.caps[0]);
        positions.push(centre(last) + self.t * self.caps[1]);
        let end = start + 1;

        let ring = |r: u32, j: u32| r * seg + j % seg;
        let rings = sections.len() as u32;
        let mut indices = Vec::new();
        for j in 0..seg {
            indices.extend([start, ring(0, j + 1), ring(0, j)]);
            for r in 0..rings - 1 {
                let (a, b) = (ring(r, j), ring(r, j + 1));
                let (c, d) = (ring(r + 1, j), ring(r + 1, j + 1));
                indices.extend([a, b, c, b, d, c]);
            }
            indices.extend([end, ring(rings - 1, j), ring(rings - 1, j + 1)]);
        }
        let mut mesh = Mesh {
            positions,
            normals: Vec::new(),
            indices,
        };
        mesh.compute_normals();
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::perimeter;
    use crate::pick::{ray_triangle, Ray};

    /// Inside test by counting crossings along a ray; needs a closed mesh.
    fn inside(mesh: &Mesh, p: Vec3) -> bool {
        let ray = Ray::new(p, Vec3::new(0.31, 0.83, 0.47));
        mesh.triangles()
            .filter(|&tri| ray_triangle(&ray, tri).is_some())
            .count()
            % 2
            == 1
    }

    /// Girth of the torso at height `y`, from the mesh's own vertices.
    fn girth(torso: &Mesh, y: f32) -> f32 {
        let ring: Vec<Vec3> = torso
            .positions
            .iter()
            .copied()
            .filter(|p| (p.y - y).abs() < 1e-4)
            .collect();
        assert_eq!(ring.len(), 48, "one section at {y}");
        perimeter(&ring)
    }

    #[test]
    fn every_part_is_closed_and_faces_outward() {
        for kind in AvatarKind::ALL {
            let parts = avatar_parts(kind, kind.default_measurements());
            assert_eq!(parts.len(), 7, "torso, 2 legs, 2 feet, 2 arms");
            for (i, part) in parts.iter().enumerate() {
                assert!(part.is_watertight(), "{kind:?} part {i} has holes");
                assert!(part.signed_volume() > 0.0, "{kind:?} part {i} inside out");
                assert_eq!(part.normals.len(), part.positions.len());
                for (k, (p, n)) in part.positions.iter().zip(&part.normals).enumerate() {
                    assert!((n.length() - 1.0).abs() < 1e-4);
                    // Sample the surface: a step along the normal leaves
                    // the part, a step against it goes in.
                    if k % 23 == 0 {
                        assert!(
                            !inside(part, *p + *n * 5e-4),
                            "{kind:?} part {i} vertex {k}"
                        );
                        assert!(inside(part, *p - *n * 5e-4), "{kind:?} part {i} vertex {k}");
                    }
                }
            }
        }
    }

    #[test]
    fn merged_avatar_is_watertight_and_reasonably_sized() {
        let mesh = avatar_mesh(
            AvatarKind::Female,
            AvatarKind::Female.default_measurements(),
        );
        assert!(mesh.is_watertight());
        assert!(
            mesh.positions.len() > 3000,
            "smooth: {}",
            mesh.positions.len()
        );
        assert!(mesh.positions.len() < 40_000, "not wasteful");
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.positions.len()));
    }

    #[test]
    fn stands_on_the_floor_at_its_height_and_faces_front() {
        for kind in AvatarKind::ALL {
            let m = kind.default_measurements();
            let b = avatar_mesh(kind, m).bounds().unwrap();
            assert!(b.min.y.abs() < 1e-3, "{kind:?} feet at {}", b.min.y);
            assert!((b.max.y - m.height / 100.0).abs() < 1e-3, "{kind:?}");
            // Symmetric left/right, feet pointing forward.
            assert!((b.min.x + b.max.x).abs() < 1e-3);
            assert!(b.max.z > -b.min.z);
            // Arms hang close enough to leave room for the pattern ring.
            assert!(b.max.x < 0.25 * m.height / 100.0, "{kind:?} {b:?}");
        }
    }

    #[test]
    fn girths_match_the_measurements() {
        for kind in AvatarKind::ALL {
            let m = kind.default_measurements();
            let torso = &avatar_parts(kind, m)[0];
            let l = landmarks(kind, m);
            for (name, y, want) in [
                ("chest", l.chest, m.chest),
                ("waist", l.waist, m.waist),
                ("hip", l.hip, m.hip),
            ] {
                let got = girth(torso, y) * 100.0;
                assert!(
                    (got - want).abs() / want < 0.01,
                    "{kind:?} {name}: {got:.1} cm, want {want}"
                );
            }
        }
    }

    #[test]
    fn sizing_changes_the_body() {
        let kind = AvatarKind::Male;
        let base = kind.default_measurements();
        let girths = |m: Measurements| {
            let torso = &avatar_parts(kind, m)[0];
            let l = landmarks(kind, m);
            [
                girth(torso, l.chest),
                girth(torso, l.waist),
                girth(torso, l.hip),
            ]
        };
        let before = girths(base);

        let wider = Measurements {
            chest: 112.0,
            ..base
        };
        let after = girths(wider);
        assert!(after[0] > before[0] + 0.1, "chest grows");
        assert!((after[1] - before[1]).abs() < 1e-4, "waist unchanged");
        assert!((after[2] - before[2]).abs() < 1e-4, "hip unchanged");

        let after = girths(Measurements {
            waist: 70.0,
            ..base
        });
        assert!(after[1] < before[1] - 0.1);
        let after = girths(Measurements { hip: 110.0, ..base });
        assert!(after[2] > before[2] + 0.05);

        let tall = Measurements {
            height: 195.0,
            ..base
        };
        let b = avatar_mesh(kind, tall).bounds().unwrap();
        assert!((b.max.y - 1.95).abs() < 1e-3);
    }

    #[test]
    fn measurements_are_clamped() {
        let silly = Measurements {
            height: 5.0,
            chest: 1000.0,
            waist: -3.0,
            hip: 90.0,
        }
        .clamped();
        assert_eq!(silly.height, 80.0);
        assert_eq!(silly.chest, 170.0);
        assert_eq!(silly.waist, 40.0);
        assert_eq!(silly.hip, 90.0);
        // And the generator copes with the extremes.
        let mesh = avatar_mesh(AvatarKind::Kid, silly);
        assert!(mesh.is_watertight());
        assert!(mesh.positions.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn ellipse_perimeter_is_accurate() {
        let circle = ellipse_perimeter(1.0, 1.0);
        assert!((circle - std::f32::consts::TAU).abs() < 1e-5);
        // A 2:1 ellipse is 9.6884 around (to 4 decimals).
        assert!((ellipse_perimeter(2.0, 1.0) - 9.6884).abs() < 1e-3);
        let (a, b) = section(0.9, 0.75);
        assert!((ellipse_perimeter(a, b) - 0.9).abs() < 1e-5);
        assert!((b / a - 0.75).abs() < 1e-6);
    }
}
