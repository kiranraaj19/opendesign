//! DXF-AAMA import: pattern pieces from an ASCII DXF file, the reverse of
//! [`crate::dxf`].
//!
//! Two layer conventions are understood, and can be mixed:
//! - the one [`crate::dxf::write`] uses, a family of layers per piece
//!   (`<PIECE>` boundary, `<PIECE>_SEAM`, `_GRAIN`, `_NOTCH`, `_INTERNAL`,
//!   `_FOLD`, `_DART`);
//! - AAMA layer numbers, as most pattern CAD tools write them: `1`
//!   boundary, `4` notches, `7` grainline, `8` and `11` internal lines,
//!   `14` sew line (the seam allowance's inner edge). Pieces are usually
//!   `BLOCK`s placed with `INSERT`, one block per piece, with a
//!   `Piece Name:` text.
//!
//! Each closed boundary becomes a piece; the other entities in its group
//! (block or layer family) are given to the boundary they lie in, or the
//! nearest one. Entities are `POLYLINE`/`VERTEX` (with arc bulges),
//! `LWPOLYLINE`, `LINE` and `TEXT`; anything else is skipped. Curves come
//! in as runs of corner points. Units follow `$INSUNITS` (or an AAMA
//! `UNITS:` text) and are converted to mm. DXF's y axis points up and the
//! pattern window's down, so y is negated, as [`crate::dxf`] does on the
//! way out.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::command::{AddPattern, Batch, Command};
use crate::document::{Dart, Fold, InternalLine, Notch, OutlinePoint, Pattern, PatternId};
use crate::geometry::{self, Vec2};

#[derive(Debug)]
pub enum ImportError {
    Io(io::Error),
    /// Not an ASCII DXF file (e.g. binary DXF, or something else entirely).
    NotDxf(String),
    /// A DXF file, but without a closed boundary to make a piece from.
    NoPieces,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::NotDxf(why) => write!(f, "Not an ASCII DXF file: {why}"),
            Self::NoPieces => write!(f, "No closed pattern boundaries found"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<io::Error> for ImportError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// The pieces in the DXF file at `path`.
pub fn import(path: &Path) -> Result<Vec<Pattern>, ImportError> {
    let bytes = fs::read(path)?;
    read(&String::from_utf8_lossy(&bytes))
}

/// The pieces in DXF `text`, in file order. Their ids are placeholders:
/// [`AddPattern`] gives each a fresh one.
pub fn read(text: &str) -> Result<Vec<Pattern>, ImportError> {
    let pairs = pairs(text)?;
    let file = parse(&pairs);
    let scale = file.scale();
    let mut patterns = Vec::new();
    for group in file.groups() {
        patterns.extend(group.pieces(scale));
    }
    if patterns.is_empty() {
        return Err(ImportError::NoPieces);
    }
    Ok(patterns)
}

/// Adding `patterns` as one undo step, moved by `offset` mm.
pub fn import_command(label: impl Into<String>, patterns: Vec<Pattern>, offset: Vec2) -> Batch {
    let commands = patterns
        .into_iter()
        .map(|mut p| {
            if offset != Vec2::ZERO {
                p.translate(offset);
            }
            Box::new(AddPattern::from_pattern(p)) as Box<dyn Command>
        })
        .collect();
    Batch::new(label, commands)
}

/// The `(group code, value)` pairs of an ASCII DXF file.
fn pairs(text: &str) -> Result<Vec<(i32, &str)>, ImportError> {
    if text.starts_with("AutoCAD Binary DXF") {
        return Err(ImportError::NotDxf("binary DXF is not supported".into()));
    }
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out = Vec::with_capacity(lines.len() / 2);
    for (i, pair) in lines.chunks(2).enumerate() {
        let [code, value] = pair else {
            // A dangling last line (e.g. no newline after EOF) is harmless.
            break;
        };
        if code.is_empty() && value.is_empty() {
            continue;
        }
        let code = code
            .parse()
            .map_err(|_| ImportError::NotDxf(format!("line {} is not a group code", i * 2 + 1)))?;
        out.push((code, *value));
    }
    if !out.iter().any(|&(c, v)| c == 0 && v == "SECTION") {
        return Err(ImportError::NotDxf("no SECTION".into()));
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
enum Shape {
    Poly { points: Vec<Vec2>, closed: bool },
    Line(Vec2, Vec2),
    Text { at: Vec2, text: String },
}

impl Shape {
    fn map(&mut self, f: impl Fn(Vec2) -> Vec2) {
        match self {
            Shape::Poly { points, .. } => points.iter_mut().for_each(|p| *p = f(*p)),
            Shape::Line(a, b) => {
                *a = f(*a);
                *b = f(*b);
            }
            Shape::Text { at, .. } => *at = f(*at),
        }
    }

    /// A point that lies on the shape, for finding its piece.
    fn anchor(&self) -> Vec2 {
        match self {
            Shape::Poly { points, .. } => match points.as_slice() {
                [a, b, ..] => a.lerp(*b, 0.5),
                [a] => *a,
                [] => Vec2::ZERO,
            },
            Shape::Line(a, b) => a.lerp(*b, 0.5),
            Shape::Text { at, .. } => *at,
        }
    }

    fn points(&self) -> Vec<Vec2> {
        match self {
            Shape::Poly { points, .. } => points.clone(),
            Shape::Line(a, b) => vec![*a, *b],
            Shape::Text { at, .. } => vec![*at],
        }
    }

    /// A closed loop of at least three points, without a repeated last
    /// point, if this is one.
    fn ring(&self) -> Option<Vec<Vec2>> {
        let Shape::Poly { points, closed } = self else {
            return None;
        };
        let mut points = points.clone();
        let joined = points.len() > 2 && points[0].distance(points[points.len() - 1]) < 1e-3;
        if joined {
            points.pop();
        }
        points.dedup_by(|a, b| a.distance(*b) < 1e-4);
        (points.len() >= 3 && (*closed || joined)).then_some(points)
    }
}

#[derive(Debug, Clone)]
struct Entity {
    layer: String,
    shape: Shape,
}

struct Insert {
    block: String,
    at: Vec2,
    scale: Vec2,
    rotation: f32,
}

#[derive(Default)]
struct DxfFile {
    insunits: Option<i32>,
    blocks: Vec<(String, Vec<Entity>)>,
    entities: Vec<Entity>,
    inserts: Vec<Insert>,
}

/// The value of the first `code` in an entity's pairs.
fn value<'a>(pairs: &[(i32, &'a str)], code: i32) -> Option<&'a str> {
    pairs.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
}

fn number(pairs: &[(i32, &str)], code: i32) -> Option<f32> {
    value(pairs, code).and_then(|v| v.parse().ok())
}

fn point(pairs: &[(i32, &str)], x: i32) -> Vec2 {
    Vec2::new(
        number(pairs, x).unwrap_or(0.0),
        number(pairs, x + 10).unwrap_or(0.0),
    )
}

/// Points along a `bulge` arc from `a` to `b` (DXF's tangent of a quarter
/// of the included angle, positive anticlockwise), `a` left out, `b` in.
fn arc(a: Vec2, b: Vec2, bulge: f32) -> Vec<Vec2> {
    let chord = b - a;
    let d = chord.length();
    if bulge.abs() < 1e-6 || d < 1e-9 {
        return vec![b];
    }
    let sweep = 4.0 * bulge.atan();
    let left = Vec2::new(-chord.y, chord.x) * (1.0 / d);
    let center = a.lerp(b, 0.5) + left * (d * 0.5 * (1.0 - bulge * bulge) / (2.0 * bulge));
    let r = center.distance(a);
    let start = (a.y - center.y).atan2(a.x - center.x);
    let steps = (sweep.abs() / 10f32.to_radians()).ceil().max(1.0) as usize;
    let mut out: Vec<Vec2> = (1..steps)
        .map(|i| {
            let t = start + sweep * i as f32 / steps as f32;
            center + Vec2::new(t.cos(), t.sin()) * r
        })
        .collect();
    out.push(b);
    out
}

/// Polyline vertices with their bulges, as points along straight and arc
/// segments.
fn with_bulges(vertices: &[(Vec2, f32)], closed: bool) -> Vec<Vec2> {
    let mut out = Vec::new();
    for (i, &(p, bulge)) in vertices.iter().enumerate() {
        if i == 0 {
            out.push(p);
        }
        let next = if i + 1 < vertices.len() {
            Some(vertices[i + 1].0)
        } else if closed && vertices.len() > 1 {
            Some(vertices[0].0)
        } else {
            None
        };
        if let Some(q) = next {
            out.extend(arc(p, q, bulge));
        }
    }
    if closed && out.len() > 2 && out[0].distance(out[out.len() - 1]) < 1e-4 {
        out.pop();
    }
    out
}

/// A polyline vertex and the bulge of the segment after it.
type Vertex = (Vec2, f32);

/// Split the pairs at each code 0 into `(entity type, its pairs)`.
fn records<'a>(pairs: &[(i32, &'a str)]) -> Vec<(&'a str, Vec<(i32, &'a str)>)> {
    let mut out: Vec<(&str, Vec<(i32, &str)>)> = Vec::new();
    for &(code, v) in pairs {
        if code == 0 {
            out.push((v, Vec::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push((code, v));
        }
    }
    out
}

fn parse(pairs: &[(i32, &str)]) -> DxfFile {
    let mut file = DxfFile::default();
    let records = records(pairs);
    let mut section = "";
    let mut block: Option<(String, Vec2, Vec<Entity>)> = None;
    // An open POLYLINE: layer, closed flag, vertices with bulges.
    let mut poly: Option<(String, bool, Vec<Vertex>)> = None;
    let mut i = 0;
    while i < records.len() {
        let (kind, body) = &records[i];
        i += 1;
        let layer = || value(body, 8).unwrap_or("0").to_string();
        let mut entity = None;
        match *kind {
            "SECTION" => {
                section = value(body, 2).unwrap_or("");
                // Header variables are a 9 pair, then their value.
                if let Some(at) = body.iter().position(|&(c, v)| c == 9 && v == "$INSUNITS") {
                    file.insunits = body.get(at + 1).and_then(|(_, v)| v.parse().ok());
                }
            }
            "ENDSEC" => section = "",
            "VERTEX" if poly.is_some() => {
                if let Some((_, _, vertices)) = &mut poly {
                    vertices.push((point(body, 10), number(body, 42).unwrap_or(0.0)));
                }
            }
            "SEQEND" => {
                if let Some((layer, closed, vertices)) = poly.take() {
                    let points = with_bulges(&vertices, closed);
                    entity = Some(Entity {
                        layer,
                        shape: Shape::Poly { points, closed },
                    });
                }
            }
            "BLOCK" => {
                let name = value(body, 2).unwrap_or("").to_string();
                block = Some((name, point(body, 10), Vec::new()));
            }
            "ENDBLK" => {
                if let Some((name, base, mut entities)) = block.take() {
                    for e in &mut entities {
                        e.shape.map(|p| p - base);
                    }
                    file.blocks.push((name, entities));
                }
            }
            "POLYLINE" => {
                let flags: i32 = value(body, 70).and_then(|v| v.parse().ok()).unwrap_or(0);
                poly = Some((layer(), flags & 1 != 0, Vec::new()));
            }
            "LWPOLYLINE" => {
                let flags: i32 = value(body, 70).and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut vertices: Vec<(Vec2, f32)> = Vec::new();
                for &(code, v) in body {
                    let v: f32 = v.parse().unwrap_or(0.0);
                    match code {
                        10 => vertices.push((Vec2::new(v, 0.0), 0.0)),
                        20 => {
                            if let Some(last) = vertices.last_mut() {
                                last.0.y = v;
                            }
                        }
                        42 => {
                            if let Some(last) = vertices.last_mut() {
                                last.1 = v;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = flags & 1 != 0;
                entity = Some(Entity {
                    layer: layer(),
                    shape: Shape::Poly {
                        points: with_bulges(&vertices, closed),
                        closed,
                    },
                });
            }
            "LINE" => {
                entity = Some(Entity {
                    layer: layer(),
                    shape: Shape::Line(point(body, 10), point(body, 11)),
                });
            }
            "TEXT" | "MTEXT" => {
                entity = Some(Entity {
                    layer: layer(),
                    shape: Shape::Text {
                        at: point(body, 10),
                        text: value(body, 1).unwrap_or("").to_string(),
                    },
                });
            }
            "INSERT" if section == "ENTITIES" => file.inserts.push(Insert {
                block: value(body, 2).unwrap_or("").to_string(),
                at: point(body, 10),
                scale: Vec2::new(
                    number(body, 41).unwrap_or(1.0),
                    number(body, 42).unwrap_or(1.0),
                ),
                rotation: number(body, 50).unwrap_or(0.0),
            }),
            _ => {}
        }
        if let Some(e) = entity {
            match &mut block {
                Some((_, _, entities)) => entities.push(e),
                None if section == "ENTITIES" => file.entities.push(e),
                None => {}
            }
        }
    }
    file
}

/// What an entity is, from its layer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    Boundary,
    Seam,
    Grain,
    Notch,
    Internal,
    Fold,
    Dart,
    /// A layer whose contents don't map to anything here (AAMA turn
    /// points, drill holes, ...), apart from texts.
    Other,
}

/// The role of `layer` and the group (piece family) it belongs to.
fn role(layer: &str) -> (Role, &str) {
    if let Ok(n) = layer.parse::<u32>() {
        let role = match n {
            1 => Role::Boundary,
            4 => Role::Notch,
            7 => Role::Grain,
            8 | 11 => Role::Internal,
            14 => Role::Seam,
            _ => Role::Other,
        };
        return (role, "");
    }
    for (suffix, role) in [
        ("_SEAM", Role::Seam),
        ("_GRAIN", Role::Grain),
        ("_NOTCH", Role::Notch),
        ("_INTERNAL", Role::Internal),
        ("_FOLD", Role::Fold),
        ("_DART", Role::Dart),
    ] {
        if let Some(base) = layer.strip_suffix(suffix) {
            return (role, base);
        }
    }
    (Role::Boundary, layer)
}

/// Entities that make up one or more pieces: a block, or a family of
/// layers.
struct Group {
    name: String,
    entities: Vec<Entity>,
}

impl DxfFile {
    /// Millimetres per drawing unit.
    fn scale(&self) -> f32 {
        match self.insunits {
            Some(1) => 25.4,
            Some(2) => 304.8,
            Some(5) => 10.0,
            Some(6) => 1000.0,
            Some(4) => 1.0,
            _ => {
                // AAMA files say so in a text instead.
                let english = self
                    .blocks
                    .iter()
                    .flat_map(|(_, e)| e)
                    .chain(&self.entities)
                    .any(|e| match &e.shape {
                        Shape::Text { text, .. } => {
                            let t = text.to_ascii_uppercase();
                            t.starts_with("UNITS") && t.contains("ENGLISH")
                        }
                        _ => false,
                    });
                if english {
                    25.4
                } else {
                    1.0
                }
            }
        }
    }

    fn groups(&self) -> Vec<Group> {
        let mut groups = Vec::new();
        let blocks: HashMap<&str, &Vec<Entity>> =
            self.blocks.iter().map(|(n, e)| (n.as_str(), e)).collect();
        for insert in &self.inserts {
            let Some(entities) = blocks.get(insert.block.as_str()) else {
                continue;
            };
            let (sin, cos) = insert.rotation.to_radians().sin_cos();
            let place = |p: Vec2| {
                let p = Vec2::new(p.x * insert.scale.x, p.y * insert.scale.y);
                insert.at + Vec2::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos)
            };
            let mut entities = (*entities).clone();
            for e in &mut entities {
                e.shape.map(place);
            }
            groups.push(Group {
                name: insert.block.clone(),
                entities,
            });
        }
        // Loose entities, grouped by layer family in file order.
        let mut loose: Vec<Group> = Vec::new();
        for e in &self.entities {
            let (_, base) = role(&e.layer);
            match loose.iter_mut().find(|g| g.name == base) {
                Some(g) => g.entities.push(e.clone()),
                None => loose.push(Group {
                    name: base.to_string(),
                    entities: vec![e.clone()],
                }),
            }
        }
        groups.extend(loose);
        groups
    }
}

/// A layer or block name as a piece name: `FRONT_PANEL_3` → `Front Panel`.
fn piece_name(group: &str, index: usize) -> String {
    let words: Vec<&str> = group.split('_').filter(|w| !w.is_empty()).collect();
    // The exporter appends the piece id.
    let words = match words.split_last() {
        Some((last, rest)) if !rest.is_empty() && last.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => &words[..],
    };
    if words.is_empty() {
        return format!("Piece {}", index + 1);
    }
    words
        .iter()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The entities that belong to one piece, with what each one is.
type Marks = Vec<(Role, Shape)>;

impl Group {
    fn pieces(&self, scale: f32) -> Vec<Pattern> {
        // File units to mm, y down.
        let to_doc = |p: Vec2| Vec2::new(p.x * scale, -p.y * scale);
        let entities: Vec<(Role, Shape)> = self
            .entities
            .iter()
            .map(|e| {
                let mut shape = e.shape.clone();
                shape.map(to_doc);
                (role(&e.layer).0, shape)
            })
            .collect();
        let mut pieces: Vec<(Vec<Vec2>, Marks)> = entities
            .iter()
            .filter(|(r, _)| *r == Role::Boundary)
            .filter_map(|(_, s)| s.ring())
            .map(|ring| (ring, Vec::new()))
            .collect();
        if pieces.is_empty() {
            return Vec::new();
        }
        for (role, shape) in entities {
            let is_ring = shape.ring().is_some();
            if role == Role::Boundary && is_ring {
                continue;
            }
            // Open shapes on a boundary layer are marks on the piece.
            let role = match role {
                Role::Boundary if !matches!(shape, Shape::Text { .. }) => Role::Internal,
                r => r,
            };
            let at = shape.anchor();
            let owner = pieces
                .iter()
                .position(|(ring, _)| geometry::contains(ring, at))
                .unwrap_or_else(|| {
                    let gap = |ring: &[Vec2]| {
                        geometry::distance_along(ring, at).map_or(f32::MAX, |(_, q)| q.distance(at))
                    };
                    (0..pieces.len())
                        .min_by(|&a, &b| gap(&pieces[a].0).total_cmp(&gap(&pieces[b].0)))
                        .unwrap_or(0)
                });
            pieces[owner].1.push((role, shape));
        }
        let several = pieces.len() > 1;
        pieces
            .into_iter()
            .enumerate()
            .map(|(i, (ring, marks))| {
                let mut name = piece_name(&self.name, i);
                if several && !self.name.is_empty() {
                    name = format!("{name} {}", i + 1);
                }
                build(name, ring, &marks)
            })
            .collect()
    }
}

/// The text of an AAMA `Piece Name: ...` label.
fn label_name(text: &str) -> Option<String> {
    let (key, value) = text.split_once(':')?;
    let key = key.trim().to_ascii_uppercase();
    (key == "PIECE NAME" || key == "PIECE").then(|| value.trim().to_string())
}

fn build(name: String, ring: Vec<Vec2>, marks: &[(Role, Shape)]) -> Pattern {
    let outline: Vec<OutlinePoint> = ring.iter().copied().map(OutlinePoint::corner).collect();
    let mut p = Pattern::new(PatternId(0), name, outline);
    p.seam_allowance = 0.0;
    let texts: Vec<(Vec2, &str)> = marks
        .iter()
        .filter_map(|(_, s)| match s {
            Shape::Text { at, text } => Some((*at, text.as_str())),
            _ => None,
        })
        .collect();
    if let Some(name) = texts.iter().find_map(|(_, t)| label_name(t)) {
        if !name.is_empty() {
            p.name = name;
        }
    }
    let mut grain = None;
    for (role, shape) in marks {
        if matches!(shape, Shape::Text { .. }) {
            continue;
        }
        let points = shape.points();
        match role {
            Role::Seam => {
                // The allowance is the median distance of the seam line's
                // edge midpoints from the boundary (corners stick out
                // further), to 0.1 mm.
                let mut gaps: Vec<f32> = (0..points.len())
                    .map(|i| points[i].lerp(points[(i + 1) % points.len()], 0.5))
                    .filter_map(|q| {
                        geometry::distance_along(&ring, q).map(|(_, on)| on.distance(q))
                    })
                    .collect();
                gaps.sort_by(f32::total_cmp);
                if let Some(&median) = gaps.get(gaps.len() / 2) {
                    p.seam_allowance = (median * 10.0).round() / 10.0;
                }
            }
            Role::Grain if grain.is_none() && points.len() >= 2 => {
                grain = Some((points[0], points[points.len() - 1]));
            }
            Role::Notch => {
                // A tick across the outline, or one running in from it:
                // whichever of its ends or middle is on the outline.
                let candidates = [points[0], points[points.len() - 1], shape.anchor()];
                let best = candidates
                    .iter()
                    .filter_map(|&q| {
                        geometry::distance_along(&ring, q).map(|(d, on)| (d, on.distance(q)))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((at, _)) = best {
                    p.notches.push(Notch { at });
                }
            }
            Role::Internal | Role::Fold if points.len() >= 2 => {
                let mut points = points;
                if shape.ring().is_some() && points.first() != points.last() {
                    points.push(points[0]);
                }
                let mut line = InternalLine::new(points);
                if *role == Role::Fold {
                    line.fold = Some(fold_label(&texts, line.points[0]));
                }
                p.internal_lines.push(line);
            }
            Role::Dart => {
                if let Some(&[start, a, end, b]) = shape.ring().as_deref() {
                    p.darts.push(Dart {
                        start,
                        end,
                        width: a.distance(b),
                    });
                }
            }
            _ => {}
        }
    }
    if let Some((a, b)) = grain {
        // The inverse of the exporter's direction (-sin, cos), folded into
        // -90..=90 as a grainline has no head.
        let d = b - a;
        let mut angle = (-d.x).atan2(d.y).to_degrees();
        if angle > 90.0 + 1e-3 {
            angle -= 180.0;
        } else if angle <= -90.0 - 1e-3 {
            angle += 180.0;
        }
        p.grain_angle = crate::document::normalize_degrees(angle);
    }
    p.notches.sort_by(|a, b| a.at.total_cmp(&b.at));
    p
}

/// The fold labelled by the `MOUNTAIN 170` / `VALLEY 90` text nearest
/// `at`, or a default valley fold.
fn fold_label(texts: &[(Vec2, &str)], at: Vec2) -> Fold {
    let nearest = texts
        .iter()
        .min_by(|a, b| a.0.distance(at).total_cmp(&b.0.distance(at)));
    let Some((_, text)) = nearest else {
        return Fold::valley();
    };
    let mut words = text.split_whitespace();
    let kind = words.next().unwrap_or("").to_ascii_uppercase();
    let angle = words
        .next()
        .and_then(|a| a.parse::<f32>().ok())
        .unwrap_or(Fold::DEFAULT_ANGLE)
        .abs();
    let sign = if kind == "MOUNTAIN" { -1.0 } else { 1.0 };
    Fold {
        angle: angle * sign,
        ..Fold::valley()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, TOLERANCE};
    use crate::dxf;
    use crate::history::History;

    fn assert_near(a: Vec2, b: Vec2) {
        assert!(a.distance(b) < 0.01, "{a:?} != {b:?}");
    }

    fn panel(id: u64, name: &str, at: Vec2) -> Pattern {
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 200.0), (0.0, 200.0)];
        let mut p = Pattern::new(
            PatternId(id),
            name,
            corners
                .iter()
                .map(|&(x, y)| OutlinePoint::corner(at + Vec2::new(x, y)))
                .collect(),
        );
        p.seam_allowance = 12.0;
        p.grain_angle = 30.0;
        p.notches.push(Notch { at: 50.0 });
        p.notches.push(Notch { at: 250.0 });
        p.internal_lines.push(InternalLine::new(vec![
            at + Vec2::new(10.0, 10.0),
            at + Vec2::new(20.0, 30.0),
            at + Vec2::new(40.0, 30.0),
        ]));
        p.internal_lines.push(InternalLine {
            points: vec![at + Vec2::new(0.0, 150.0), at + Vec2::new(100.0, 150.0)],
            fold: Some(Fold {
                angle: -90.0,
                ..Fold::valley()
            }),
        });
        p.darts.push(Dart {
            start: at + Vec2::new(50.0, 40.0),
            end: at + Vec2::new(50.0, 120.0),
            width: 16.0,
        });
        p
    }

    fn sample() -> Document {
        let mut doc = Document::new("Test");
        doc.patterns.push(panel(1, "Front Panel", Vec2::ZERO));
        doc.patterns.push(panel(7, "Back", Vec2::new(300.0, -50.0)));
        doc
    }

    #[test]
    fn reads_back_what_the_exporter_writes() {
        let doc = sample();
        let pieces = read(&dxf::write(&doc)).unwrap();
        assert_eq!(pieces.len(), 2);
        for (got, want) in pieces.iter().zip(&doc.patterns) {
            assert_eq!(got.outline.len(), want.outline.len());
            for (a, b) in got.outline.iter().zip(&want.outline) {
                assert_near(a.pos, b.pos);
            }
            assert!((got.seam_allowance - want.seam_allowance).abs() < 0.2);
            assert!((got.grain_angle - want.grain_angle).abs() < 0.01);
            assert_eq!(got.notches.len(), 2);
            for (a, b) in got.notches.iter().zip(&want.notches) {
                assert!((a.at - b.at).abs() < 0.01, "{} != {}", a.at, b.at);
            }
            assert_eq!(got.internal_lines.len(), 2);
            let plain = got
                .internal_lines
                .iter()
                .find(|l| l.fold.is_none())
                .unwrap();
            assert_eq!(plain.points.len(), 3);
            for (a, b) in plain.points.iter().zip(&want.internal_lines[0].points) {
                assert_near(*a, *b);
            }
            let fold = got.internal_lines.iter().find_map(|l| l.fold).unwrap();
            assert_eq!(fold.angle, -90.0);
            assert_eq!(got.darts.len(), 1);
            let (a, b) = (got.darts[0], want.darts[0]);
            assert_near(a.start, b.start);
            assert_near(a.end, b.end);
            assert!((a.width - b.width).abs() < 0.01);
        }
        assert_eq!(pieces[0].name, "Front Panel");
        assert_eq!(pieces[1].name, "Back");
    }

    #[test]
    fn curves_come_back_as_their_flattened_points() {
        let mut doc = Document::new("Curvy");
        let mut p = panel(1, "Curvy", Vec2::ZERO);
        p.outline[1] = OutlinePoint::curve(Vec2::new(120.0, 100.0));
        p.outline
            .insert(1, OutlinePoint::corner(Vec2::new(100.0, 0.0)));
        p.outline[2] = OutlinePoint::curve(Vec2::new(130.0, 100.0));
        doc.patterns.push(p);
        let flat = doc.patterns[0].polyline(TOLERANCE);
        let pieces = read(&dxf::write(&doc)).unwrap();
        let got: Vec<Vec2> = pieces[0].positions().collect();
        assert_eq!(got.len(), flat.len());
        for (a, b) in got.iter().zip(&flat) {
            assert_near(*a, *b);
        }
    }

    /// A small AAMA-style file: one piece per block, numbered layers,
    /// inches, LWPOLYLINE boundary, an unknown entity type in between.
    const AAMA: &str = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n0\nENDSEC\n\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n8\n0\n2\nSLEEVE\n70\n0\n10\n0.0\n20\n0.0\n\
0\nLWPOLYLINE\n8\n1\n90\n4\n70\n1\n10\n0\n20\n0\n10\n4\n20\n0\n10\n4\n20\n2\n10\n0\n20\n2\n\
0\nCIRCLE\n8\n13\n10\n2\n20\n1\n40\n0.1\n\
0\nLINE\n8\n7\n10\n2\n20\n0.5\n11\n2\n21\n1.5\n\
0\nLINE\n8\n4\n10\n1\n20\n0\n11\n1\n21\n0.25\n\
0\nLINE\n8\n8\n10\n1\n20\n1\n11\n3\n21\n1\n\
0\nTEXT\n8\n1\n10\n1\n20\n1.2\n40\n0.2\n1\nPiece Name: Sleeve Left\n\
0\nTEXT\n8\n1\n10\n1\n20\n1.5\n40\n0.2\n1\nUNITS: ENGLISH\n\
0\nENDBLK\n8\n0\n\
0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nINSERT\n8\n0\n2\nSLEEVE\n10\n10\n20\n0\n\
0\nENDSEC\n0\nEOF\n";

    #[test]
    fn reads_aama_blocks_with_numbered_layers() {
        let pieces = read(AAMA).unwrap();
        assert_eq!(pieces.len(), 1);
        let p = &pieces[0];
        assert_eq!(p.name, "Sleeve Left");
        let inch = 25.4;
        let outline: Vec<Vec2> = p.positions().collect();
        assert_eq!(outline.len(), 4);
        // Placed by the INSERT at x = 10 in; y flipped to point down.
        assert_near(outline[0], Vec2::new(10.0 * inch, 0.0));
        assert_near(outline[2], Vec2::new(14.0 * inch, -2.0 * inch));
        assert_eq!(p.grain_angle, 0.0);
        assert_eq!(p.notches.len(), 1);
        assert!((p.notches[0].at - inch).abs() < 0.01);
        assert_eq!(p.internal_lines.len(), 1);
        assert_near(p.internal_lines[0].points[0], Vec2::new(11.0 * inch, -inch));
        assert_eq!(p.seam_allowance, 0.0);
    }

    #[test]
    fn loose_numbered_layers_split_by_boundary() {
        let text = "0\nSECTION\n2\nENTITIES\n\
0\nPOLYLINE\n8\n1\n66\n1\n70\n1\n\
0\nVERTEX\n8\n1\n10\n0\n20\n0\n0\nVERTEX\n8\n1\n10\n100\n20\n0\n\
0\nVERTEX\n8\n1\n10\n100\n20\n100\n0\nVERTEX\n8\n1\n10\n0\n20\n100\n0\nSEQEND\n\
0\nLWPOLYLINE\n8\n1\n90\n3\n70\n1\n10\n200\n20\n0\n10\n300\n20\n0\n10\n250\n20\n80\n\
0\nLINE\n8\n8\n10\n220\n20\n10\n11\n260\n21\n20\n\
0\nLINE\n8\n8\n10\n10\n20\n10\n11\n50\n21\n50\n\
0\nLINE\n8\n8\n10\n20\n20\n10\n11\n30\n21\n50\n\
0\nENDSEC\n0\nEOF\n";
        let pieces = read(text).unwrap();
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].outline.len(), 4);
        assert_eq!(pieces[1].outline.len(), 3);
        assert_eq!(pieces[0].internal_lines.len(), 2);
        assert_eq!(pieces[1].internal_lines.len(), 1);
        assert_eq!(pieces[0].name, "Piece 1");
    }

    #[test]
    fn bulges_become_arcs() {
        // A half-disc: a straight edge and a semicircle back.
        let text = "0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n8\nHALF\n90\n2\n70\n1\n10\n-50\n20\n0\n42\n0\n10\n50\n20\n0\n42\n1\n\
0\nENDSEC\n0\nEOF\n";
        let pieces = read(text).unwrap();
        let points: Vec<Vec2> = pieces[0].positions().collect();
        assert!(points.len() > 10);
        for p in &points[1..] {
            assert!((p.length() - 50.0).abs() < 0.01, "{p:?} is off the circle");
        }
        // The arc from (50, 0) back to (-50, 0) runs anticlockwise
        // through y = 50 in DXF, so y = -50 here.
        assert!(points.iter().any(|p| (p.y + 50.0).abs() < 0.5));
        assert_eq!(pieces[0].name, "Half");
    }

    #[test]
    fn rejects_files_that_are_not_dxf_or_have_no_pieces() {
        assert!(matches!(
            read("hello\nworld\n"),
            Err(ImportError::NotDxf(_))
        ));
        assert!(matches!(
            read("AutoCAD Binary DXF\r\n"),
            Err(ImportError::NotDxf(_))
        ));
        let lines_only =
            "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n1\n10\n0\n20\n0\n11\n1\n21\n1\n0\nENDSEC\n0\nEOF\n";
        assert!(matches!(read(lines_only), Err(ImportError::NoPieces)));
    }

    #[test]
    fn crlf_line_endings_and_odd_spacing_are_fine() {
        let text = dxf::write(&sample())
            .replace('\n', "\r\n")
            .replace("\r\n0\r\n", "\r\n  0\r\n");
        assert_eq!(read(&text).unwrap().len(), 2);
    }

    #[test]
    fn import_is_one_undoable_step() {
        let mut doc = Document::new("Target");
        let mut history = History::new();
        let pieces = read(&dxf::write(&sample())).unwrap();
        history.execute(
            &mut doc,
            Box::new(import_command("Import DXF", pieces, Vec2::new(10.0, 0.0))),
        );
        assert_eq!(doc.patterns.len(), 2);
        assert_near(doc.patterns[0].outline[0].pos, Vec2::new(10.0, 0.0));
        assert_ne!(doc.patterns[0].id, doc.patterns[1].id);
        assert_eq!(history.undo_label().as_deref(), Some("Import DXF"));
        history.undo(&mut doc);
        assert!(doc.patterns.is_empty());
        history.redo(&mut doc);
        assert_eq!(doc.patterns.len(), 2);
    }

    #[test]
    fn imports_from_a_file() {
        let dir = std::env::temp_dir().join(format!("od_core_dxf_import_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("in.dxf");
        dxf::export(&sample(), &path).unwrap();
        assert_eq!(import(&path).unwrap().len(), 2);
        assert!(matches!(
            import(&dir.join("missing.dxf")),
            Err(ImportError::Io(_))
        ));
    }
}
