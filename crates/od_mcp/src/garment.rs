//! Ready-made garments: every piece drafted to the avatar's size, sewn and
//! arranged around the body, so a drape gives a garment straight away.
//! The pieces are ordinary patterns afterwards, to be edited like any
//! other.

use od_core::document::TOLERANCE;
use od_core::{
    AddPattern, AddSewingLine, Arrangement, Fabric, Joint, OutlinePoint, PatternId, PointKind,
    SeamEdge, SetArrangement, SetFabric, Vec2,
};
use od_render::avatar::{joint_positions, landmarks};
use od_render::glam::Vec3;
use od_render::mesh::Mesh;
use od_render::pick::{ray_mesh, Ray};

use crate::session::{Limb, Session};

/// What to make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    TShirt,
    Skirt,
    Dress,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "tshirt" | "t-shirt" | "top" => Kind::TShirt,
            "skirt" => Kind::Skirt,
            "dress" => Kind::Dress,
            _ => return None,
        })
    }
}

pub struct Options {
    pub kind: Kind,
    pub fabric: Fabric,
    /// Room around the body, in mm of girth.
    pub ease: f32,
    /// From the shoulder (tops, dresses) or waist (skirts) down, in mm.
    pub length: Option<f32>,
    /// From the top of the shoulder down the arm, in mm.
    pub sleeve_length: f32,
    /// How much wider the hem is than the hips on each side, in mm.
    pub flare: f32,
}

/// What was made: each piece's role and id, and how many seams.
pub struct Made {
    pub pieces: Vec<(&'static str, PatternId)>,
    pub seams: usize,
    pub notes: Vec<String>,
}

use PointKind::{Corner as C, Curve as Q};

pub fn create(s: &mut Session, o: &Options) -> Made {
    let spec = s.avatar;
    let marks = landmarks(spec.kind, spec.measurements);
    let joints = joint_positions(spec.kind, spec.measurements);
    let shoulder_y = joints[Joint::LeftShoulder.index()].y;
    let m = spec.measurements;
    // Lay new pieces out to the right of any already there.
    let mut cursor = s
        .doc
        .patterns
        .iter()
        .filter_map(|p| p.bounds())
        .map(|(_, max)| max.x + 100.0)
        .fold(0.0_f32, f32::max);
    let mut made = Made {
        pieces: Vec::new(),
        seams: 0,
        notes: Vec::new(),
    };
    let avatar = s.avatar_mesh();

    match o.kind {
        Kind::TShirt | Kind::Dress => {
            // Body: the starter block, widened to the chest plus ease and
            // lengthened below the underarm.
            let default_length = match o.kind {
                Kind::Dress => (shoulder_y - marks.hip) * 1000.0 + 450.0,
                _ => (shoulder_y - marks.hip) * 1000.0 + 120.0,
            };
            let length = o.length.unwrap_or(default_length).max(300.0);
            let girth = (m.chest.max(m.hip) * 10.0 + o.ease).max(400.0);
            // Front and back are each half the girth across, shoulder
            // tips just past the avatar's shoulder joints so the garment
            // hangs from them, and a neckline the head goes through.
            let width = girth / 2.0;
            let mid = width / 2.0;
            let shoulder =
                (joints[Joint::LeftShoulder.index()].x.abs() * 1000.0 + 25.0).min(mid - 20.0);
            // The solver has no friction, so a loose neckline lets the
            // garment slide off one shoulder: keep it close to the neck.
            let neck_y = joints[Joint::Neck.index()].y + 0.02;
            let neck_half = (clearance(&avatar, Vec3::X, (neck_y, neck_y), 0.0) - GAP) * 1000.0;
            let neck = (neck_half + 15.0).clamp(45.0, shoulder - 60.0);
            let hem_flare = if o.kind == Kind::Dress { o.flare } else { 0.0 };
            let body = |neck_depth: f32| -> Vec<OutlinePoint> {
                [
                    (mid - shoulder, 50.0, C),
                    (mid - neck, 0.0, C),
                    (mid - neck + 5.0, neck_depth, Q),
                    (mid + neck - 5.0, neck_depth, Q),
                    (mid + neck, 0.0, C),
                    (mid + shoulder, 50.0, C),
                    (mid + shoulder - 12.0, 215.0, Q),
                    (width, 230.0, C),
                    (width + hem_flare, length, C),
                    (-hem_flare, length, C),
                    (0.0, 230.0, C),
                    (mid - shoulder + 12.0, 215.0, Q),
                ]
                .into_iter()
                .map(|(x, y, kind)| OutlinePoint {
                    pos: Vec2::new(x, y),
                    kind,
                })
                .collect()
            };
            let sx = width / 520.0;
            let name = if o.kind == Kind::Dress {
                "Dress"
            } else {
                "Shirt"
            };
            let front = add(s, &format!("{name} front"), body(95.0), &mut cursor);
            let back = add(s, &format!("{name} back"), body(25.0), &mut cursor);

            // Sleeves: the cap's width scaled until it matches the armhole.
            let armhole = edge_length(s, front, 7, 5) + edge_length(s, back, 0, 10);
            let sleeve = |scale: f32| -> Vec<OutlinePoint> {
                let hem = o.sleeve_length.max(140.0);
                let taper = 30.0 * (hem - 130.0) / 200.0;
                [
                    (0.0, 130.0, C),
                    (80.0, 125.0, Q),
                    (120.0, 0.0, Q),
                    (320.0, 0.0, Q),
                    (360.0, 125.0, Q),
                    (440.0, 130.0, C),
                    (440.0 - taper, hem, C),
                    (taper, hem, C),
                ]
                .into_iter()
                .map(|(x, y, kind)| OutlinePoint {
                    pos: Vec2::new(x * scale, y),
                    kind,
                })
                .collect()
            };
            let mut scale = sx;
            for _ in 0..3 {
                let cap = od_core::Pattern::new(PatternId(0), "probe", sleeve(scale));
                let len = cap.segment_length(0.0, 5.0, TOLERANCE);
                if len > 1.0 {
                    scale *= armhole * 1.02 / len;
                }
            }
            let left = add(s, "Left sleeve", sleeve(scale), &mut cursor);
            let right = add(s, "Right sleeve", sleeve(scale), &mut cursor);

            // Front pattern x runs to the avatar's left (+X), back pattern x
            // to its right; see `od_render::arrange`.
            sew(s, &mut made, &[(front, 1, 0)], &[(back, 4, 5)]);
            sew(s, &mut made, &[(front, 4, 5)], &[(back, 1, 0)]);
            sew(s, &mut made, &[(front, 10, 9)], &[(back, 7, 8)]);
            sew(s, &mut made, &[(front, 7, 8)], &[(back, 10, 9)]);
            // The left sleeve's x runs towards the back, the right's to the
            // front; each cap goes round its armhole from underarm to
            // underarm, across the shoulder.
            sew(
                s,
                &mut made,
                &[(left, 0, 5)],
                &[(front, 7, 5), (back, 0, 10)],
            );
            sew(
                s,
                &mut made,
                &[(right, 0, 5)],
                &[(back, 7, 5), (front, 0, 10)],
            );
            sew(s, &mut made, &[(left, 0, 7)], &[(left, 5, 6)]);
            sew(s, &mut made, &[(right, 0, 7)], &[(right, 5, 6)]);

            let top = shoulder_y + 0.07;
            let mid_y = top - length / 2000.0;
            let span = (top - length / 1000.0, top - 0.05);
            let half_width = girth / 4000.0;
            let front_z = clearance(&avatar, Vec3::Z, span, half_width);
            let back_z = clearance(&avatar, Vec3::NEG_Z, span, half_width);
            arrange(s, front, Vec3::new(0.0, mid_y, front_z), Vec3::Z);
            arrange(s, back, Vec3::new(0.0, mid_y, -back_z), Vec3::NEG_Z);
            let sleeve_mid = top - o.sleeve_length.max(140.0) / 2000.0 - 0.02;
            for (id, side, limb) in [
                (left, Vec3::X, Limb::LeftArm),
                (right, Vec3::NEG_X, Limb::RightArm),
            ] {
                let reach = clearance(&avatar, side, (sleeve_mid - 0.1, sleeve_mid + 0.1), 0.08);
                arrange(s, id, Vec3::new(side.x * reach, sleeve_mid, 0.0), side);
                // Drape with the arm inside the sleeve, not beside it.
                s.wraps.insert(id, limb);
            }
            made.pieces = vec![
                ("front", front),
                ("back", back),
                ("left sleeve", left),
                ("right sleeve", right),
            ];
            made.notes.push(format!(
                "Body {:.0} mm round the chest ({:.0} mm ease), {:.0} mm long; sleeve cap {:.0} mm for a {:.0} mm armhole.",
                girth, o.ease, length, s.doc.pattern(left).map_or(0.0, |p| p.segment_length(0.0, 5.0, TOLERANCE)), armhole
            ));
        }
        Kind::Skirt => {
            let length = o.length.unwrap_or(600.0).max(150.0);
            let waist = (m.waist * 10.0 + o.ease.min(40.0)) / 2.0;
            let hip = (m.hip * 10.0 + o.ease) / 2.0;
            let hip_depth = ((marks.waist - marks.hip) * 1000.0).clamp(80.0, length - 20.0);
            let panel = || -> Vec<OutlinePoint> {
                [
                    ((hip - waist) / 2.0, 0.0),
                    ((hip + waist) / 2.0, 0.0),
                    (hip, hip_depth),
                    (hip + o.flare, length),
                    (-o.flare, length),
                    (0.0, hip_depth),
                ]
                .into_iter()
                .map(|(x, y)| OutlinePoint::corner(Vec2::new(x, y)))
                .collect()
            };
            let front = add(s, "Skirt front", panel(), &mut cursor);
            let back = add(s, "Skirt back", panel(), &mut cursor);
            sew(s, &mut made, &[(front, 1, 3)], &[(back, 0, 4)]);
            sew(s, &mut made, &[(front, 0, 4)], &[(back, 1, 3)]);
            let top = marks.waist + 0.02;
            let span = (top - length / 1000.0, top);
            let half_width = hip / 2000.0 + o.flare / 1000.0;
            let front_z = clearance(&avatar, Vec3::Z, span, half_width);
            let back_z = clearance(&avatar, Vec3::NEG_Z, span, half_width);
            let mid_y = top - length / 2000.0;
            arrange(s, front, Vec3::new(0.0, mid_y, front_z), Vec3::Z);
            arrange(s, back, Vec3::new(0.0, mid_y, -back_z), Vec3::NEG_Z);
            made.pieces = vec![("front", front), ("back", back)];
            made.notes.push(format!(
                "Waist {:.0} mm, hips {:.0} mm round, {:.0} mm long.",
                waist * 2.0,
                hip * 2.0,
                length
            ));
        }
    }

    for &(_, id) in &made.pieces {
        s.execute(SetFabric::new(id, o.fabric.clone()));
    }
    made
}

/// Add a piece left of `cursor`'s line, moving it on.
fn add(s: &mut Session, name: &str, outline: Vec<OutlinePoint>, cursor: &mut f32) -> PatternId {
    let min_x = outline.iter().map(|p| p.pos.x).fold(f32::MAX, f32::min);
    let max_x = outline.iter().map(|p| p.pos.x).fold(f32::MIN, f32::max);
    let shift = Vec2::new(*cursor - min_x, 0.0);
    let outline = outline
        .into_iter()
        .map(|p| OutlinePoint {
            pos: p.pos + shift,
            kind: p.kind,
        })
        .collect();
    s.execute(AddPattern::with_outline(name, outline));
    *cursor += max_x - min_x + 80.0;
    s.doc.patterns.last().expect("just added").id
}

/// Length of a pattern's outline from point `from` to point `to`, the
/// shorter way round.
fn edge_length(s: &Session, id: PatternId, from: usize, to: usize) -> f32 {
    s.doc.pattern(id).map_or(0.0, |p| {
        let e = p.shorter_edge(from as f32, to as f32);
        p.segment_length(e.start, e.end, TOLERANCE)
    })
}

/// A seam edge from point `from` to point `to`, the shorter way round.
pub fn edge(s: &Session, id: PatternId, from: usize, to: usize) -> Option<SeamEdge> {
    s.doc
        .pattern(id)
        .map(|p| p.shorter_edge(from as f32, to as f32))
}

fn sew(
    s: &mut Session,
    made: &mut Made,
    a: &[(PatternId, usize, usize)],
    b: &[(PatternId, usize, usize)],
) {
    let side = |s: &Session, edges: &[(PatternId, usize, usize)]| -> Vec<SeamEdge> {
        edges
            .iter()
            .filter_map(|&(id, from, to)| edge(s, id, from, to))
            .collect()
    };
    let (a, b) = (side(s, a), side(s, b));
    s.execute(AddSewingLine::new(a, b));
    made.seams += 1;
}

/// Stand piece `id` upright at `center`, its front facing `outward`.
fn arrange(s: &mut Session, id: PatternId, center: Vec3, outward: Vec3) {
    let right = Vec3::Y.cross(outward).normalize();
    s.execute(SetArrangement::new(
        id,
        Some(Arrangement {
            center: center.into(),
            right: right.into(),
            up: Vec3::Y.into(),
        }),
    ));
}

/// Room left between a piece and the body when it is placed, in metres.
const GAP: f32 = 0.04;

/// How far out along `outward` (a horizontal unit axis) the body reaches
/// between heights `ys` and within `half_width` of the middle, plus a gap,
/// in metres: where a piece facing that way can start without touching
/// the body.
fn clearance(body: &Mesh, outward: Vec3, ys: (f32, f32), half_width: f32) -> f32 {
    let across = Vec3::Y.cross(outward);
    let (low, high) = (ys.0.min(ys.1), ys.0.max(ys.1));
    let mut reach = 0.0_f32;
    let steps = 12;
    for i in 0..=steps {
        let y = low + (high - low) * i as f32 / steps as f32;
        for j in -steps..=steps {
            let side = across * (half_width * j as f32 / steps as f32);
            let origin = Vec3::new(0.0, y, 0.0) + side + outward * 3.0;
            let ray = Ray {
                origin,
                dir: -outward,
            };
            if let Some(t) = ray_mesh(&ray, body) {
                reach = reach.max(ray.at(t).dot(outward));
            }
        }
    }
    reach + GAP
}
