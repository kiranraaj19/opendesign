//! The tools the assistant can call: their names, descriptions and input
//! schemas for `tools/list`, and what each does for `tools/call`.

use std::path::PathBuf;

use od_core::document::TOLERANCE;
use od_core::edit::{self, PleatKind};
use od_core::{
    project, AddDart, AddInternalLine, AddPattern, AddSewingLine, AddTack, AddTrim, Axis, Dart,
    DeletePattern, DeletePoint, DuplicatePattern, Fabric, FabricTexture, Fold, InsertPoint,
    MirrorPattern, MovePattern, MovePoint, OutlinePoint, PatternId, PatternValue, PointKind, Pose,
    RenamePattern, RotatePattern, SetArrangement, SetFabric, SetOutline, SetPatternValue,
    SetPointKind, SetPose, TackPoint, TackTarget, TrimKind, Vec2, Zipper,
};
use od_render::avatar::{AvatarKind, Measurements};
use serde_json::{json, Map, Value};

use crate::garment;
use crate::render::{self, View};
use crate::session::{Limb, Session, DT, SUBSTEPS};

/// What a tool call gives back: text, pictures, and whether it failed.
pub struct Outcome {
    content: Vec<Value>,
    is_error: bool,
}

impl Outcome {
    fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![json!({ "type": "text", "text": text.into() })],
            is_error: false,
        }
    }

    fn json(value: Value) -> Self {
        Self::text(serde_json::to_string_pretty(&value).unwrap_or_default())
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::text(message)
        }
    }

    fn with_png(mut self, png: &[u8]) -> Self {
        self.content.push(json!({
            "type": "image",
            "data": render::base64(png),
            "mimeType": "image/png",
        }));
        self
    }

    pub fn into_json(self) -> Value {
        json!({ "content": self.content, "isError": self.is_error })
    }
}

type Result<T> = std::result::Result<T, String>;

/// Cloth resolution for drapes, in mm: a little coarser than the app's
/// default, as a drape here is judged from a still and needs to be quick.
const DEFAULT_PARTICLE_DISTANCE: f32 = 25.0;

/// Longest a drape call runs before answering, in seconds: well inside the
/// time clients wait for a tool. A longer drape carries on next call.
const DEFAULT_WAIT: f32 = 45.0;

/// Every tool, for `tools/list`.
pub fn list() -> Vec<Value> {
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        })
    };
    let pattern = json!({ "type": "integer", "description": "A pattern id from get_document." });
    let side = json!({
        "type": "array",
        "description": "Edges sewn in order, each [pattern, from_point, to_point]: the outline from one point to the other, the shorter way round. A side of several edges (a sleeve cap round two armholes) is sewn as one.",
        "items": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
    });
    vec![
        tool(
            "create_garment",
            "Make a complete garment sized to the avatar: pieces drafted, sewn together, given a fabric and placed around the body ready to drape. Types: tshirt (front, back, two sleeves; long sleeves via sleeve_length_mm), dress (a longer, flared tshirt), skirt (front and back panels). Returns the piece ids. Follow with drape and render.",
            json!({
                "type": { "type": "string", "enum": ["tshirt", "dress", "skirt"] },
                "fabric": { "type": "string", "enum": ["cotton", "silk", "denim"], "description": "Default cotton." },
                "color": { "type": "string", "description": "Hex colour, e.g. \"#8B1E3F\"." },
                "ease_mm": { "type": "number", "description": "Extra girth beyond the body, in mm. Default 120 (tops) or 60 (skirts)." },
                "length_mm": { "type": "number", "description": "From the shoulder (tshirt, dress) or waist (skirt) down." },
                "sleeve_length_mm": { "type": "number", "description": "From the shoulder down the arm. Default 330 (short)." },
                "flare_mm": { "type": "number", "description": "Extra hem width on each side seam (skirt, dress). Default 60 (skirt) or 120 (dress)." },
            }),
            &["type"],
        ),
        tool(
            "get_document",
            "The whole project at a glance: the avatar, every pattern piece (id, name, size, fabric, whether it is placed on the body), every seam (its edges, side lengths and how far they differ) and the drape's state.",
            json!({}),
            &[],
        ),
        tool(
            "get_pattern",
            "One piece in detail: every outline point (index, x, y, corner or curve), the edges between neighbouring corners with their lengths, its internal lines, darts and trims, seam allowance, grainline and fabric, and the seams that use it.",
            json!({ "pattern": pattern }),
            &["pattern"],
        ),
        tool(
            "add_pattern",
            "Draft a new pattern piece from its outline, in mm with y down, going round the piece in order. Mark curve control points with \"curve\": the outline bends towards them. Returns its id. Place it on the body with arrange before draping.",
            json!({
                "name": { "type": "string" },
                "points": {
                    "type": "array",
                    "description": "Outline points: [x, y] for a corner, [x, y, \"curve\"] for a curve control point. At least 3.",
                    "items": { "type": "array" },
                },
            }),
            &["name", "points"],
        ),
        tool(
            "edit",
            "Change pieces: a list of operations done in order, each its own undo step. Every operation has \"op\" and \"pattern\". Ops:\n\
             move {dx, dy}; rotate {degrees}; mirror {axis: vertical|horizontal}; delete; rename {name}; duplicate {mirror: bool};\n\
             seam_allowance {mm}; grain {degrees}; fabric {preset?: cotton|silk|denim, color?: \"#rrggbb\", texture?: flat|plain|twill, weight_gsm?};\n\
             move_point {index, x, y}; insert_point {index, x, y, curve?} (goes before index); delete_point {index}; point_kind {index, curve: bool};\n\
             round_corner {index, distance}; edge_length {index, length} (from corner index to the next corner);\n\
             outline_dart {x, y, width, depth} (a V cut into the outline nearest x,y, its legs sewn); dart {start: [x,y], end: [x,y], width} (a diamond dart inside);\n\
             internal_line {points: [[x,y],...], fold?: valley|mountain}; pleats {from: [x,y], to: [x,y], kind?: knife|box, count?, depth?} (fold lines across the piece, tacked at the top);\n\
             button {x, y}; buttonhole {x, y, angle?}; tack_to {pattern_point: [x,y], other_pattern, other_point: [x,y]}.",
            json!({
                "operations": { "type": "array", "items": { "type": "object" } },
            }),
            &["operations"],
        ),
        tool(
            "sew",
            "Sew seams. Each seam joins side a to side b, start to start and end to end, so give both sides running the same way along the garment (e.g. both neck to shoulder). Use get_pattern for point indices. Optionally make a seam a zipper.",
            json!({
                "seams": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "a": side.clone(),
                            "b": side,
                            "zipper": { "type": "boolean" },
                        },
                        "required": ["a", "b"],
                    },
                },
            }),
            &["seams"],
        ),
        tool(
            "arrange",
            "Place pieces around the avatar before draping, each upright and facing out from the body: at a named spot (front, back, left_arm, right_arm, left_leg, right_leg; the avatar's own left and right), optionally at a given height in metres, or at an explicit position. Pieces with no place are put on a ring around the body, which drapes poorly: arrange every piece you sew.",
            json!({
                "pieces": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "pattern": { "type": "integer" },
                            "spot": { "type": "string", "enum": ["front", "back", "left_arm", "right_arm", "left_leg", "right_leg"] },
                            "height": { "type": "number", "description": "Height of the piece's centre above the floor, in metres." },
                            "position": { "type": "array", "items": { "type": "number" }, "description": "[x, y, z] in metres; the avatar faces +z, its left is +x." },
                            "facing": { "type": "string", "enum": ["front", "back", "left", "right"], "description": "With position: which way the piece faces." },
                            "wrap": { "type": "boolean", "description": "At an arm or leg spot: start the drape with the piece rolled into a tube round the limb (a sleeve or trouser leg whose side edges are sewn together). Default true." },
                        },
                        "required": ["pattern"],
                    },
                },
            }),
            &["pieces"],
        ),
        tool(
            "set_avatar",
            "Choose the body and its size (cm), and pose it: a built-in pose (Rest, Arms down, A-pose, T-pose, Arms forward, Walking, Hands on hips) or joint angles in degrees, e.g. {\"left_shoulder\": {\"spread\": 30}}. Joints: pelvis, spine, neck, left/right_shoulder, _elbow, _wrist, _hip, _knee, _ankle; angles bend (forward), spread (out from the body) and twist.",
            json!({
                "body": { "type": "string", "enum": ["female", "male", "kid"] },
                "height": { "type": "number" },
                "chest": { "type": "number" },
                "waist": { "type": "number" },
                "hip": { "type": "number" },
                "pose": { "description": "A built-in pose name, or an object of joint angles." },
            }),
            &[],
        ),
        tool(
            "drape",
            "Simulate the cloth: the seams pull the arranged pieces together round the avatar over the first 1.5 s (gravity held off meanwhile), then the garment settles. Continues the current drape if nothing changed since, else starts again. Each call answers within max_wait_seconds, so a long drape may take several calls: check the notes. Returns how stretched the cloth is; render to see it.",
            json!({
                "seconds": { "type": "number", "description": "Simulated time to run. Default 3 (enough for most garments); 4-5 for long or full ones." },
                "particle_distance": { "type": "number", "description": "Cloth resolution in mm. Default 25; 35 is faster, 15 finer and much slower." },
                "max_wait_seconds": { "type": "number", "description": "Stop and answer after this long. Default 45." },
                "reset": { "type": "boolean", "description": "Start again from the flat pieces." },
                "cloth_collision": { "type": "boolean", "description": "Keep layers of cloth from passing through each other. Default true; false is faster for a quick look." },
            }),
            &[],
        ),
        tool(
            "render",
            "A picture of the garment, returned as an image: the drape if there is one, else the flat pieces around the avatar. Views: front, back, left, right, top, three_quarter, or pattern (every piece laid flat as drafted). style=strain colours the cloth blue (relaxed) to red (stretched 10% or more), to find what's too tight.",
            json!({
                "view": { "type": "string", "enum": ["front", "back", "left", "right", "top", "three_quarter", "pattern"] },
                "width": { "type": "integer", "description": "Default 768." },
                "height": { "type": "integer", "description": "Default 1024." },
                "style": { "type": "string", "enum": ["fabric", "strain"] },
                "avatar": { "type": "boolean", "description": "Show the avatar. Default true." },
                "background": { "type": "string", "enum": ["light", "dark", "black"], "description": "Default light." },
                "turn": { "type": "number", "description": "Turn the camera this many degrees around the garment." },
                "save_to": { "type": "string", "description": "Also write the PNG to this file." },
            }),
            &[],
        ),
        tool(
            "export",
            "Write the work out: obj or gltf (the draped garment's 3D mesh; drape first), dxf (the flat patterns, DXF-AAMA) or odproj (the project, as save_project).",
            json!({
                "format": { "type": "string", "enum": ["obj", "gltf", "dxf", "odproj"] },
                "path": { "type": "string" },
            }),
            &["format", "path"],
        ),
        tool(
            "new_project",
            "Start an empty project (unsaved work is dropped).",
            json!({ "name": { "type": "string" } }),
            &[],
        ),
        tool(
            "open_project",
            "Open an OpenDesign project (.odproj).",
            json!({ "path": { "type": "string" } }),
            &["path"],
        ),
        tool(
            "save_project",
            "Save the project as .odproj, which opens in the OpenDesign app. Without a path, saves where it was opened or last saved.",
            json!({ "path": { "type": "string" } }),
            &[],
        ),
        tool(
            "undo",
            "Undo the last changes.",
            json!({ "steps": { "type": "integer", "description": "Default 1." } }),
            &[],
        ),
        tool(
            "redo",
            "Redo undone changes.",
            json!({ "steps": { "type": "integer", "description": "Default 1." } }),
            &[],
        ),
    ]
}

/// Run tool `name`, or `None` if there is no such tool.
pub fn call(s: &mut Session, name: &str, args: &Value) -> Option<Outcome> {
    let result = match name {
        "create_garment" => create_garment(s, args),
        "get_document" => Ok(Outcome::json(document_summary(s))),
        "get_pattern" => get_pattern(s, args),
        "add_pattern" => add_pattern(s, args),
        "edit" => edit_patterns(s, args),
        "sew" => sew(s, args),
        "arrange" => arrange(s, args),
        "set_avatar" => set_avatar(s, args),
        "drape" => drape(s, args),
        "render" => render_tool(s, args),
        "export" => export(s, args),
        "new_project" => {
            let name = str_arg(args, "name").unwrap_or("Untitled");
            s.replace(od_core::Document::new(name), None);
            Ok(Outcome::text(format!("Started a new project, {name}.")))
        }
        "open_project" => open_project(s, args),
        "save_project" => save_project(s, args),
        "undo" | "redo" => {
            let steps = args
                .get("steps")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .max(1);
            let mut done = Vec::new();
            for _ in 0..steps {
                let label = if name == "undo" {
                    s.history.undo_label()
                } else {
                    s.history.redo_label()
                };
                let did = if name == "undo" {
                    s.history.undo(&mut s.doc)
                } else {
                    s.history.redo(&mut s.doc)
                };
                if !did {
                    break;
                }
                done.extend(label);
            }
            Ok(Outcome::text(if done.is_empty() {
                format!("Nothing to {name}.")
            } else {
                format!(
                    "{}: {}.",
                    if name == "undo" { "Undid" } else { "Redid" },
                    done.join(", ")
                )
            }))
        }
        _ => return None,
    };
    Some(result.unwrap_or_else(Outcome::error))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn num(args: &Value, key: &str) -> Option<f32> {
    args.get(key).and_then(Value::as_f64).map(|v| v as f32)
}

fn need_num(args: &Value, key: &str) -> Result<f32> {
    num(args, key).ok_or_else(|| format!("\"{key}\" (a number) is needed"))
}

fn need_index(args: &Value, key: &str) -> Result<usize> {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .ok_or_else(|| format!("\"{key}\" (a point index) is needed"))
}

fn pattern_arg(s: &Session, args: &Value, key: &str) -> Result<PatternId> {
    let id = args
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("\"{key}\" (a pattern id) is needed"))?;
    let id = PatternId(id);
    if s.pattern_exists(id) {
        Ok(id)
    } else {
        Err(format!(
            "there is no pattern {}; get_document lists them",
            id.0
        ))
    }
}

fn point_arg(value: Option<&Value>, what: &str) -> Result<Vec2> {
    let v = value.and_then(Value::as_array);
    match v.map(|a| {
        (
            a.first().and_then(Value::as_f64),
            a.get(1).and_then(Value::as_f64),
        )
    }) {
        Some((Some(x), Some(y))) => Ok(Vec2::new(x as f32, y as f32)),
        _ => Err(format!("\"{what}\" should be [x, y] in mm")),
    }
}

fn color_arg(value: &str) -> Result<[u8; 3]> {
    let hex = value.trim_start_matches('#');
    let parse = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("zz"), 16);
    match (hex.len(), parse(0), parse(2), parse(4)) {
        (6, Ok(r), Ok(g), Ok(b)) => Ok([r, g, b]),
        _ => Err(format!("colour \"{value}\" should look like \"#8B1E3F\"")),
    }
}

fn preset(name: &str) -> Result<Fabric> {
    match name {
        "cotton" => Ok(Fabric::cotton()),
        "silk" => Ok(Fabric::silk()),
        "denim" => Ok(Fabric::denim()),
        _ => Err(format!(
            "no fabric called \"{name}\"; try cotton, silk or denim"
        )),
    }
}

fn create_garment(s: &mut Session, args: &Value) -> Result<Outcome> {
    let kind = str_arg(args, "type")
        .and_then(garment::Kind::parse)
        .ok_or("\"type\" should be tshirt, dress or skirt")?;
    let mut fabric = preset(str_arg(args, "fabric").unwrap_or("cotton"))?;
    if let Some(color) = str_arg(args, "color") {
        fabric.color = color_arg(color)?;
    }
    let skirt = kind == garment::Kind::Skirt;
    let options = garment::Options {
        kind,
        fabric,
        ease: num(args, "ease_mm").unwrap_or(if skirt { 60.0 } else { 120.0 }),
        length: num(args, "length_mm"),
        sleeve_length: num(args, "sleeve_length_mm").unwrap_or(330.0),
        flare: num(args, "flare_mm").unwrap_or(match kind {
            garment::Kind::Skirt => 60.0,
            garment::Kind::Dress => 120.0,
            garment::Kind::TShirt => 0.0,
        }),
    };
    let made = garment::create(s, &options);
    let pieces: Map<String, Value> = made
        .pieces
        .iter()
        .map(|(role, id)| (role.to_string(), json!(id.0)))
        .collect();
    Ok(Outcome::json(json!({
        "pieces": pieces,
        "seams": made.seams,
        "notes": made.notes,
        "next": "drape, then render to look at it",
    })))
}

/// Everything at a glance, for get_document.
fn document_summary(s: &mut Session) -> Value {
    let doc = &s.doc;
    let patterns: Vec<Value> = doc
        .patterns
        .iter()
        .map(|p| {
            let (min, max) = p.bounds().unwrap_or_default();
            json!({
                "id": p.id.0,
                "name": p.name,
                "points": p.outline.len(),
                "bounds_mm": [[min.x.round(), min.y.round()], [max.x.round(), max.y.round()]],
                "area_cm2": (p.area() / 100.0).round(),
                "fabric": p.fabric.name,
                "color": format!("#{:02X}{:02X}{:02X}", p.fabric.color[0], p.fabric.color[1], p.fabric.color[2]),
                "arranged": p.arrangement.is_some(),
                "hidden": p.hidden,
            })
        })
        .collect();
    let seams: Vec<Value> = doc
        .sewing_lines
        .iter()
        .map(|line| {
            let lengths = line.side_lengths(doc);
            json!({
                "id": line.id.0,
                "a": line.a.iter().map(|e| edge_json(doc, e)).collect::<Vec<_>>(),
                "b": line.b.iter().map(|e| edge_json(doc, e)).collect::<Vec<_>>(),
                "lengths_mm": lengths.map(|(a, b)| [a.round(), b.round()]),
                "mismatch_pct": mismatch(lengths).map(|m| (m * 1000.0).round() / 10.0),
                "zipper": line.details.zipper.is_some(),
            })
        })
        .collect();
    let avatar = s.avatar;
    let pose = serde_json::to_value(&doc.avatar_pose).unwrap_or(Value::Null);
    let drape = s.drape.as_ref().map(|d| {
        json!({
            "seconds": d.time,
            "up_to_date": d.doc == s.doc,
        })
    });
    json!({
        "name": s.doc.name,
        "path": s.path.as_ref().map(|p| p.display().to_string()),
        "unsaved_changes": s.history.is_dirty(),
        "avatar": {
            "body": avatar.kind.label(),
            "height_cm": avatar.measurements.height,
            "chest_cm": avatar.measurements.chest,
            "waist_cm": avatar.measurements.waist,
            "hip_cm": avatar.measurements.hip,
            "pose": pose,
        },
        "patterns": patterns,
        "seams": seams,
        "tacks": s.doc.tacks.len(),
        "drape": drape,
    })
}

/// How much longer side a is than side b, as a fraction of the shorter.
fn mismatch(lengths: Option<(f32, f32)>) -> Option<f32> {
    let (a, b) = lengths?;
    (a.min(b) > 1e-3).then(|| (a - b) / a.min(b))
}

/// A seam edge as [pattern, from point, to point] in sewing order, when
/// its ends fall on outline points, else as outline parameters.
fn edge_json(doc: &od_core::Document, e: &od_core::SeamEdge) -> Value {
    let n = doc.pattern(e.pattern).map_or(1, |p| p.outline.len()).max(1) as f32;
    let (first, last) = (e.first().rem_euclid(n), e.last().rem_euclid(n));
    let whole = |v: f32| (v - v.round()).abs() < 1e-3;
    if whole(first) && whole(last) {
        json!([
            e.pattern.0,
            first.round() as u32 % n as u32,
            last.round() as u32 % n as u32
        ])
    } else {
        json!([
            e.pattern.0,
            (first * 100.0).round() / 100.0,
            (last * 100.0).round() / 100.0
        ])
    }
}

fn get_pattern(s: &mut Session, args: &Value) -> Result<Outcome> {
    let id = pattern_arg(s, args, "pattern")?;
    let p = s.doc.pattern(id).expect("checked");
    let n = p.outline.len();
    let points: Vec<Value> = p
        .outline
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut v = json!({ "i": i, "x": round1(q.pos.x), "y": round1(q.pos.y) });
            if q.kind == PointKind::Curve {
                v["curve"] = json!(true);
            }
            v
        })
        .collect();
    let corners: Vec<usize> = (0..n)
        .filter(|&i| p.outline[i].kind == PointKind::Corner)
        .collect();
    let edges: Vec<Value> = corners
        .iter()
        .enumerate()
        .map(|(k, &from)| {
            let to = corners[(k + 1) % corners.len()];
            let end = if to > from { to } else { to + n } as f32;
            json!({
                "from": from,
                "to": to,
                "length_mm": round1(p.segment_length(from as f32, end, TOLERANCE)),
            })
        })
        .collect();
    let seams: Vec<u64> = s
        .doc
        .sewing_lines
        .iter()
        .filter(|l| l.touches(id))
        .map(|l| l.id.0)
        .collect();
    let lines: Vec<Value> = p
        .internal_lines
        .iter()
        .map(|l| {
            json!({
                "points": l.points.iter().map(|q| [round1(q.x), round1(q.y)]).collect::<Vec<_>>(),
                "fold": l.fold.map(|f| if f.angle < 0.0 { "mountain" } else { "valley" }),
            })
        })
        .collect();
    Ok(Outcome::json(json!({
        "id": id.0,
        "name": p.name,
        "points": points,
        "edges": edges,
        "perimeter_mm": round1(p.perimeter()),
        "area_cm2": round1(p.area() / 100.0),
        "seam_allowance_mm": p.seam_allowance,
        "grain_degrees": p.grain_angle,
        "fabric": {
            "name": p.fabric.name,
            "color": format!("#{:02X}{:02X}{:02X}", p.fabric.color[0], p.fabric.color[1], p.fabric.color[2]),
            "weight_gsm": (p.fabric.mass_per_area * 1000.0).round(),
            "texture": format!("{:?}", p.fabric.texture).to_lowercase(),
        },
        "internal_lines": lines,
        "darts": p.darts.iter().map(|d| json!({ "start": [round1(d.start.x), round1(d.start.y)], "end": [round1(d.end.x), round1(d.end.y)], "width": d.width })).collect::<Vec<_>>(),
        "trims": p.trims.iter().map(|t| json!({ "kind": format!("{:?}", t.kind).to_lowercase(), "at": [round1(t.pos.x), round1(t.pos.y)] })).collect::<Vec<_>>(),
        "seams": seams,
        "arranged": p.arrangement.is_some(),
    })))
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn add_pattern(s: &mut Session, args: &Value) -> Result<Outcome> {
    let name = str_arg(args, "name").ok_or("\"name\" is needed")?;
    let points = args
        .get("points")
        .and_then(Value::as_array)
        .ok_or("\"points\" should be a list of [x, y] or [x, y, \"curve\"]")?;
    let outline: Vec<OutlinePoint> = points
        .iter()
        .map(|v| {
            let pos = point_arg(Some(v), "points")?;
            let curve = v.get(2).and_then(Value::as_str) == Some("curve");
            Ok(OutlinePoint {
                pos,
                kind: if curve {
                    PointKind::Curve
                } else {
                    PointKind::Corner
                },
            })
        })
        .collect::<Result<_>>()?;
    if outline.len() < 3 {
        return Err("a pattern needs at least 3 points".into());
    }
    s.execute(AddPattern::with_outline(name, outline));
    let id = s.doc.patterns.last().expect("just added").id;
    Ok(Outcome::json(json!({ "id": id.0, "name": name })))
}

fn edit_patterns(s: &mut Session, args: &Value) -> Result<Outcome> {
    let ops = args
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("\"operations\" should be a list")?;
    let mut done = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match edit_one(s, op) {
            Ok(what) => done.push(what),
            Err(e) => {
                return Err(format!(
                    "operation {i} failed: {e}. {} before it were done{}",
                    done.len(),
                    if done.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", done.join("; "))
                    }
                ))
            }
        }
    }
    Ok(Outcome::text(format!("Done: {}.", done.join("; "))))
}

/// One edit operation; says what it did.
fn edit_one(s: &mut Session, op: &Value) -> Result<String> {
    let kind = str_arg(op, "op").ok_or("each operation needs an \"op\"")?;
    let id = pattern_arg(s, op, "pattern")?;
    let pattern = || s.doc.pattern(id).expect("checked").clone();
    Ok(match kind {
        "move" => {
            let d = Vec2::new(num(op, "dx").unwrap_or(0.0), num(op, "dy").unwrap_or(0.0));
            s.execute(MovePattern::new(id, d));
            format!("moved {}", id.0)
        }
        "rotate" => {
            s.execute(RotatePattern::new(id, need_num(op, "degrees")?));
            format!("rotated {}", id.0)
        }
        "mirror" => {
            let axis = match str_arg(op, "axis").unwrap_or("vertical") {
                "horizontal" => Axis::Horizontal,
                _ => Axis::Vertical,
            };
            s.execute(MirrorPattern::new(id, axis));
            format!("mirrored {}", id.0)
        }
        "delete" => {
            s.execute(DeletePattern::new(id));
            format!("deleted {}", id.0)
        }
        "rename" => {
            let name = str_arg(op, "name").ok_or("\"name\" is needed")?;
            s.execute(RenamePattern::new(id, name));
            format!("renamed {} {name}", id.0)
        }
        "duplicate" => {
            let mirror = op.get("mirror").and_then(Value::as_bool).unwrap_or(false);
            s.execute(DuplicatePattern::new(id, mirror.then_some(Axis::Vertical)));
            let copy = s.doc.patterns.last().expect("just added").id;
            format!("copied {} as {}", id.0, copy.0)
        }
        "seam_allowance" => {
            s.execute(SetPatternValue::new(
                id,
                PatternValue::SeamAllowance,
                need_num(op, "mm")?,
            ));
            format!("set {}'s seam allowance", id.0)
        }
        "grain" => {
            s.execute(SetPatternValue::new(
                id,
                PatternValue::GrainAngle,
                need_num(op, "degrees")?,
            ));
            format!("set {}'s grainline", id.0)
        }
        "fabric" => {
            let mut fabric = match str_arg(op, "preset") {
                Some(name) => preset(name)?,
                None => pattern().fabric,
            };
            if let Some(color) = str_arg(op, "color") {
                fabric.color = color_arg(color)?;
            }
            if let Some(texture) = str_arg(op, "texture") {
                fabric.texture = match texture {
                    "flat" => FabricTexture::Flat,
                    "plain" => FabricTexture::Plain,
                    "twill" => FabricTexture::Twill,
                    _ => return Err("texture should be flat, plain or twill".into()),
                };
            }
            if let Some(gsm) = num(op, "weight_gsm") {
                fabric.mass_per_area = gsm.clamp(10.0, 2000.0) / 1000.0;
            }
            s.execute(SetFabric::new(id, fabric));
            format!("set {}'s fabric", id.0)
        }
        "move_point" => {
            let i = need_index(op, "index")?;
            check_index(&pattern(), i)?;
            let to = Vec2::new(need_num(op, "x")?, need_num(op, "y")?);
            s.execute(MovePoint::new(id, i, to));
            format!("moved point {i} of {}", id.0)
        }
        "insert_point" => {
            let i = need_index(op, "index")?;
            let at = Vec2::new(need_num(op, "x")?, need_num(op, "y")?);
            let curve = op.get("curve").and_then(Value::as_bool).unwrap_or(false);
            let point = if curve {
                OutlinePoint::curve(at)
            } else {
                OutlinePoint::corner(at)
            };
            s.execute(InsertPoint::new(id, i.min(pattern().outline.len()), point));
            format!("inserted a point in {}", id.0)
        }
        "delete_point" => {
            let i = need_index(op, "index")?;
            check_index(&pattern(), i)?;
            s.execute(DeletePoint::new(id, i));
            format!("deleted point {i} of {}", id.0)
        }
        "point_kind" => {
            let i = need_index(op, "index")?;
            check_index(&pattern(), i)?;
            let curve = op.get("curve").and_then(Value::as_bool).unwrap_or(true);
            let kind = if curve {
                PointKind::Curve
            } else {
                PointKind::Corner
            };
            s.execute(SetPointKind::new(id, i, kind));
            format!(
                "made point {i} of {} a {}",
                id.0,
                if curve { "curve" } else { "corner" }
            )
        }
        "round_corner" => {
            let i = need_index(op, "index")?;
            let outline = edit::round_corner(&pattern().outline, i, need_num(op, "distance")?)
                .ok_or("that point can't be rounded (is it a corner, and the distance shorter than its edges?)")?;
            s.execute(SetOutline::new(id, "Round corner", outline));
            format!("rounded corner {i} of {}", id.0)
        }
        "edge_length" => {
            let i = need_index(op, "index")?;
            let outline = edit::with_edge_length(&pattern(), i, need_num(op, "length")?)
                .ok_or("that edge's length can't be set (is index a corner?)")?;
            s.execute(SetOutline::new(id, "Edge length", outline));
            format!("set an edge length on {}", id.0)
        }
        "outline_dart" => {
            let at = Vec2::new(need_num(op, "x")?, need_num(op, "y")?);
            let batch = edit::add_outline_dart(
                &pattern(),
                at,
                need_num(op, "width")?,
                need_num(op, "depth")?,
            )
            .ok_or("the dart doesn't fit there (too wide or deep for the piece?)")?;
            s.execute(batch);
            format!("cut a dart into {}", id.0)
        }
        "dart" => {
            let dart = Dart {
                start: point_arg(op.get("start"), "start")?,
                end: point_arg(op.get("end"), "end")?,
                width: need_num(op, "width")?,
            };
            s.execute(AddDart::new(id, dart));
            format!("added a dart to {}", id.0)
        }
        "internal_line" => {
            let points = op
                .get("points")
                .and_then(Value::as_array)
                .ok_or("\"points\" should be a list of [x, y]")?
                .iter()
                .map(|v| point_arg(Some(v), "points"))
                .collect::<Result<Vec<_>>>()?;
            if points.len() < 2 {
                return Err("a line needs at least 2 points".into());
            }
            match str_arg(op, "fold") {
                Some("valley") => s.execute(AddInternalLine::folded(id, points, Fold::valley())),
                Some("mountain") => {
                    s.execute(AddInternalLine::folded(id, points, Fold::mountain()))
                }
                _ => s.execute(AddInternalLine::new(id, points)),
            }
            format!("added a line to {}", id.0)
        }
        "pleats" => {
            let (a, b) = (
                point_arg(op.get("from"), "from")?,
                point_arg(op.get("to"), "to")?,
            );
            let kind = match str_arg(op, "kind").unwrap_or("knife") {
                "box" => PleatKind::Box,
                _ => PleatKind::Knife,
            };
            let count = op.get("count").and_then(Value::as_u64).unwrap_or(1) as usize;
            let depth = num(op, "depth").unwrap_or(30.0);
            let p = pattern();
            let lines =
                edit::pleat_lines(&p, a, b, kind, count, depth, od_core::Fold::DEFAULT_ANGLE);
            if lines.is_empty() {
                return Err("no pleat lines fit inside the piece there".into());
            }
            let tacks = edit::pleat_tacks(&p, a, b, kind, count, depth);
            let mut commands: Vec<Box<dyn od_core::Command>> = Vec::new();
            for line in lines {
                commands.push(match line.fold {
                    Some(fold) => Box::new(AddInternalLine::folded(id, line.points, fold)),
                    None => Box::new(AddInternalLine::new(id, line.points)),
                });
            }
            let point = |at| TackPoint { pattern: id, at };
            for [x, y] in tacks {
                commands.push(Box::new(AddTack::new(
                    point(x),
                    TackTarget::Pattern(point(y)),
                )));
            }
            s.execute(od_core::Batch::new("Add pleats", commands));
            format!("pleated {}", id.0)
        }
        "button" => {
            let at = Vec2::new(need_num(op, "x")?, need_num(op, "y")?);
            s.execute(AddTrim::new(id, TrimKind::Button, at));
            format!("added a button to {}", id.0)
        }
        "buttonhole" => {
            let at = Vec2::new(need_num(op, "x")?, need_num(op, "y")?);
            let trim = AddTrim::new(id, TrimKind::Buttonhole, at)
                .at_angle(num(op, "angle").unwrap_or(0.0));
            s.execute(trim);
            format!("added a buttonhole to {}", id.0)
        }
        "tack_to" => {
            let other = pattern_arg(s, op, "other_pattern")?;
            let a = TackPoint {
                pattern: id,
                at: point_arg(op.get("pattern_point"), "pattern_point")?,
            };
            let b = TackPoint {
                pattern: other,
                at: point_arg(op.get("other_point"), "other_point")?,
            };
            s.execute(AddTack::new(a, TackTarget::Pattern(b)));
            format!("tacked {} to {}", id.0, other.0)
        }
        other => return Err(format!("there is no \"{other}\" operation")),
    })
}

fn check_index(p: &od_core::Pattern, i: usize) -> Result<()> {
    if i < p.outline.len() {
        Ok(())
    } else {
        Err(format!(
            "pattern {} has points 0 to {}",
            p.id.0,
            p.outline.len().saturating_sub(1)
        ))
    }
}

fn sew(s: &mut Session, args: &Value) -> Result<Outcome> {
    let seams = args
        .get("seams")
        .and_then(Value::as_array)
        .ok_or("\"seams\" should be a list")?;
    let side = |s: &Session, v: Option<&Value>, name: &str| -> Result<Vec<od_core::SeamEdge>> {
        let edges = v
            .and_then(Value::as_array)
            .filter(|e| !e.is_empty())
            .ok_or_else(|| format!("side \"{name}\" should be a list of [pattern, from, to]"))?;
        edges
            .iter()
            .map(|e| {
                let n = |i: usize| e.get(i).and_then(Value::as_u64);
                let (Some(p), Some(from), Some(to)) = (n(0), n(1), n(2)) else {
                    return Err(format!(
                        "each edge of side \"{name}\" is [pattern, from, to]"
                    ));
                };
                let id = PatternId(p);
                let pattern = s
                    .doc
                    .pattern(id)
                    .ok_or_else(|| format!("there is no pattern {p}"))?;
                let len = pattern.outline.len() as u64;
                if from >= len || to >= len || from == to {
                    return Err(format!(
                        "pattern {p} has points 0 to {}; from and to must differ",
                        len - 1
                    ));
                }
                Ok(pattern.shorter_edge(from as f32, to as f32))
            })
            .collect()
    };
    let mut made = Vec::new();
    for seam in seams {
        let a = side(s, seam.get("a"), "a")?;
        let b = side(s, seam.get("b"), "b")?;
        let mut add = AddSewingLine::new(a, b);
        if seam.get("zipper").and_then(Value::as_bool).unwrap_or(false) {
            add = add.zipped(Zipper::default());
        }
        s.execute(add);
        let line = s.doc.sewing_lines.last().expect("just added");
        let lengths = line.side_lengths(&s.doc);
        made.push(json!({
            "id": line.id.0,
            "lengths_mm": lengths.map(|(a, b)| [a.round(), b.round()]),
        }));
    }
    Ok(Outcome::json(json!({ "seams": made })))
}

fn arrange(s: &mut Session, args: &Value) -> Result<Outcome> {
    use od_render::glam::Vec3;
    let pieces = args
        .get("pieces")
        .and_then(Value::as_array)
        .ok_or("\"pieces\" should be a list")?;
    let spots = s.arrangement_points();
    let mut done = Vec::new();
    for piece in pieces {
        let id = pattern_arg(s, piece, "pattern")?;
        let (center, outward) = if let Some(position) = piece.get("position") {
            let v: Vec<f32> = position
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_f64)
                        .map(|v| v as f32)
                        .collect()
                })
                .unwrap_or_default();
            let [x, y, z] = v[..] else {
                return Err("\"position\" should be [x, y, z] in metres".into());
            };
            let outward = match str_arg(piece, "facing").unwrap_or("front") {
                "back" => Vec3::NEG_Z,
                "left" => Vec3::X,
                "right" => Vec3::NEG_X,
                _ => Vec3::Z,
            };
            (Vec3::new(x, y, z), outward)
        } else {
            let spot =
                str_arg(piece, "spot").ok_or("give each piece a \"spot\" or a \"position\"")?;
            let name = match spot {
                "front" => "Front torso",
                "back" => "Back torso",
                "left_arm" => "Left arm",
                "right_arm" => "Right arm",
                "left_leg" => "Left leg",
                "right_leg" => "Right leg",
                _ => return Err(format!("no spot called \"{spot}\"")),
            };
            let point = spots
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| format!("the avatar has no {name} spot in this pose"))?;
            let mut center = point.position;
            if let Some(height) = num(piece, "height") {
                center.y = height;
            }
            (center, point.outward)
        };
        let limb = match str_arg(piece, "spot") {
            Some("left_arm") => Some(Limb::LeftArm),
            Some("right_arm") => Some(Limb::RightArm),
            Some("left_leg") => Some(Limb::LeftLeg),
            Some("right_leg") => Some(Limb::RightLeg),
            _ => None,
        };
        let wrap = piece.get("wrap").and_then(Value::as_bool).unwrap_or(true);
        match limb.filter(|_| wrap) {
            Some(limb) => s.wraps.insert(id, limb),
            None => s.wraps.remove(&id),
        };
        let right = Vec3::Y.cross(outward).normalize();
        s.execute(SetArrangement::new(
            id,
            Some(od_core::Arrangement {
                center: center.into(),
                right: right.into(),
                up: Vec3::Y.into(),
            }),
        ));
        done.push(id.0.to_string());
    }
    Ok(Outcome::text(format!("Placed {}.", done.join(", "))))
}

fn set_avatar(s: &mut Session, args: &Value) -> Result<Outcome> {
    if let Some(body) = str_arg(args, "body") {
        let kind = match body {
            "female" => AvatarKind::Female,
            "male" => AvatarKind::Male,
            "kid" => AvatarKind::Kid,
            _ => return Err("body should be female, male or kid".into()),
        };
        if kind != s.avatar.kind {
            s.avatar.kind = kind;
            s.avatar.measurements = kind.default_measurements();
        }
    }
    let m = &mut s.avatar.measurements;
    for (key, value) in [
        ("height", &mut m.height),
        ("chest", &mut m.chest),
        ("waist", &mut m.waist),
        ("hip", &mut m.hip),
    ] {
        if let Some(v) = num(args, key) {
            *value = v;
        }
    }
    *m = Measurements::clamped(*m);
    match args.get("pose") {
        Some(Value::String(name)) => {
            let (label, pose) = od_core::pose::builtin_poses()
                .into_iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no built-in pose called \"{name}\""))?;
            s.execute(SetPose::new(label, pose));
        }
        Some(angles @ Value::Object(_)) => {
            let pose: Pose = serde_json::from_value(angles.clone())
                .map_err(|e| format!("couldn't read the joint angles: {e}"))?;
            s.execute(SetPose::new("Pose", pose.clamped()));
        }
        _ => {}
    }
    let m = s.avatar.measurements;
    Ok(Outcome::text(format!(
        "Avatar: {}, {} cm tall, chest {} / waist {} / hip {} cm{}.",
        s.avatar.kind.label(),
        m.height,
        m.chest,
        m.waist,
        m.hip,
        if s.doc.avatar_pose.is_rest() {
            ", resting".to_owned()
        } else {
            ", posed".to_owned()
        }
    )))
}

fn drape(s: &mut Session, args: &Value) -> Result<Outcome> {
    if s.doc.patterns.is_empty() {
        return Err("there is nothing to drape; create_garment or add_pattern first".into());
    }
    let seconds = num(args, "seconds").unwrap_or(3.0).clamp(0.1, 30.0);
    let distance = num(args, "particle_distance")
        .unwrap_or(DEFAULT_PARTICLE_DISTANCE)
        .clamp(5.0, 60.0);
    let budget = num(args, "max_wait_seconds")
        .unwrap_or(DEFAULT_WAIT)
        .clamp(1.0, 600.0);
    let reset = args.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let unarranged: Vec<u64> = s
        .doc
        .patterns
        .iter()
        .filter(|p| !p.hidden && p.arrangement.is_none())
        .map(|p| p.id.0)
        .collect();
    let collision = args
        .get("cloth_collision")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let started = std::time::Instant::now();
    let state = s.drape_for(reset, distance);
    state.drape.set_cloth_collision(collision);
    let fresh = state.time == 0.0;
    let steps = (seconds / DT).round() as u32;
    let mut done = 0;
    while done < steps && started.elapsed().as_secs_f32() < budget {
        state.drape.step(DT, SUBSTEPS);
        done += 1;
    }
    state.time += done as f32 * DT;
    let cut_short = done < steps;
    let strain = state.drape.strain();
    let max = strain.iter().copied().fold(0.0_f32, f32::max);
    let mean = strain.iter().sum::<f32>() / strain.len().max(1) as f32;
    let tight = strain.iter().filter(|&&v| v > 0.05).count() as f32 / strain.len().max(1) as f32;
    let mut notes = Vec::new();
    if cut_short {
        notes.push(format!(
            "stopped after {:.1} of the {seconds} seconds asked for, to answer within {budget:.0} s; call drape again to carry on",
            done as f32 * DT
        ));
    }
    if state.drape.sewing_in() {
        notes.push("the seams are still closing; drape on before judging the fit".into());
    }
    if !unarranged.is_empty() {
        notes.push(format!(
            "pieces {unarranged:?} have no place on the body and start on a ring around it; arrange them for a better drape"
        ));
    }
    if max > 0.3 && !state.drape.sewing_in() {
        notes.push("some cloth is stretched over 30%: a seam may be twisted or pulling across the body, or the garment is too small".into());
    }
    Ok(Outcome::json(json!({
        "started_fresh": fresh,
        "simulated_seconds": round1(state.time),
        "took_seconds": round1(started.elapsed().as_secs_f32()),
        "particles": strain.len(),
        "stretch": {
            "max_pct": round1(max * 100.0),
            "mean_pct": (mean * 1000.0).round() / 10.0,
            "share_over_5pct": (tight * 1000.0).round() / 10.0,
        },
        "notes": notes,
    })))
}

fn render_tool(s: &mut Session, args: &Value) -> Result<Outcome> {
    let view = match str_arg(args, "view") {
        Some(name) => View::parse(name).ok_or_else(|| format!("no view called \"{name}\""))?,
        None => View::parse("front").expect("known"),
    };
    let dim = |key: &str, default: u32| {
        args.get(key)
            .and_then(Value::as_u64)
            .map_or(default, |v| v as u32)
            .clamp(64, 2048)
    };
    let req = render::Request {
        view,
        size: [dim("width", 768), dim("height", 1024)],
        avatar: args.get("avatar").and_then(Value::as_bool).unwrap_or(true),
        background: match str_arg(args, "background") {
            Some(name) => render::Background::parse(name).ok_or_else(|| {
                format!("no background called \"{name}\"; try light, dark or black")
            })?,
            None => render::Background::Light,
        },
        strain: str_arg(args, "style") == Some("strain"),
        turn: num(args, "turn").unwrap_or(0.0),
    };
    let (png, mut text) = render::render(s, &req)?;
    if let Some(path) = str_arg(args, "save_to") {
        std::fs::write(path, &png).map_err(|e| format!("couldn't write {path}: {e}"))?;
        text.push_str(&format!(" Saved to {path}."));
    }
    Ok(Outcome::text(text).with_png(&png))
}

fn export(s: &mut Session, args: &Value) -> Result<Outcome> {
    let path = PathBuf::from(str_arg(args, "path").ok_or("\"path\" is needed")?);
    match str_arg(args, "format").unwrap_or("") {
        "dxf" => od_core::dxf::export(&s.doc, &path).map_err(|e| e.to_string())?,
        "odproj" => {
            project::save(&s.doc, &path).map_err(|e| e.to_string())?;
            s.history.mark_saved();
            s.path = Some(path.clone());
        }
        format @ ("obj" | "gltf") => {
            let state = s
                .current_drape()
                .ok_or("drape the garment first: obj and gltf export the draped shape")?;
            let parts: Vec<_> = state
                .drape
                .placed_patterns()
                .into_iter()
                .map(|p| p.mesh)
                .collect();
            let mesh = od_render::mesh::Mesh::merged(&parts);
            let written = if format == "obj" {
                od_render::export::write_obj(&mesh, &path)
            } else {
                od_render::export::write_gltf(&mesh, &path)
            };
            written.map_err(|e| e.to_string())?;
        }
        other => {
            return Err(format!(
                "can't export \"{other}\"; try obj, gltf, dxf or odproj"
            ))
        }
    }
    Ok(Outcome::text(format!("Wrote {}.", path.display())))
}

fn open_project(s: &mut Session, args: &Value) -> Result<Outcome> {
    let path = PathBuf::from(str_arg(args, "path").ok_or("\"path\" is needed")?);
    let doc = project::load(&path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?;
    let count = doc.patterns.len();
    s.replace(doc, Some(path.clone()));
    Ok(Outcome::text(format!(
        "Opened {} ({count} pieces).",
        path.display()
    )))
}

fn save_project(s: &mut Session, args: &Value) -> Result<Outcome> {
    let path = match str_arg(args, "path") {
        Some(p) => PathBuf::from(p),
        None => s
            .path
            .clone()
            .ok_or("this project hasn't been saved yet; give a path")?,
    };
    project::save(&s.doc, &path).map_err(|e| format!("couldn't save {}: {e}", path.display()))?;
    s.history.mark_saved();
    s.path = Some(path.clone());
    Ok(Outcome::text(format!("Saved {}.", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Session, name: &str, args: Value) -> Value {
        let out = call(s, name, &args).expect("a known tool").into_json();
        assert_eq!(out["isError"], false, "{name} failed: {out}");
        out
    }

    fn text(out: &Value) -> Value {
        serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn fails(s: &mut Session, name: &str, args: Value) -> String {
        let out = call(s, name, &args).expect("a known tool").into_json();
        assert_eq!(out["isError"], true, "{name} should fail: {out}");
        out["content"][0]["text"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_tshirt_comes_sewn_matched_and_placed_on_the_body() {
        let mut s = Session::new();
        let made = text(&run(
            &mut s,
            "create_garment",
            json!({ "type": "tshirt", "color": "#336699" }),
        ));
        assert_eq!(made["seams"], 8);
        assert_eq!(s.doc.patterns.len(), 4);
        assert!(s.doc.patterns.iter().all(|p| p.arrangement.is_some()));
        assert!(s
            .doc
            .patterns
            .iter()
            .all(|p| p.fabric.color == [0x33, 0x66, 0x99]));
        assert_eq!(s.wraps.len(), 2, "both sleeves start round the arms");
        // Every seam's sides about the same length (the sleeve cap eased in
        // a little).
        for line in &s.doc.sewing_lines {
            let m = mismatch(line.side_lengths(&s.doc)).unwrap();
            assert!(
                m.abs() < 0.05,
                "seam {} is {:.1}% off: {:?} {:?}",
                line.id.0,
                m * 100.0,
                line.side_lengths(&s.doc),
                line
            );
        }
        // Front pieces in front of the body, back pieces behind.
        let z = |name: &str| {
            s.doc
                .patterns
                .iter()
                .find(|p| p.name.ends_with(name))
                .unwrap()
                .arrangement
                .unwrap()
                .center[2]
        };
        assert!(z("front") > 0.0 && z("back") < 0.0);

        let skirt = text(&run(
            &mut s,
            "create_garment",
            json!({ "type": "skirt", "fabric": "denim" }),
        ));
        assert_eq!(skirt["seams"], 2);
        assert_eq!(s.doc.patterns.len(), 6);
        // New pieces are laid out beside the old ones, not on top.
        let right_edge = |i: usize| s.doc.patterns[i].bounds().unwrap().1.x;
        let left_edge = |i: usize| s.doc.patterns[i].bounds().unwrap().0.x;
        assert!(left_edge(4) > right_edge(3));
        assert!(fails(&mut s, "create_garment", json!({ "type": "cape" })).contains("tshirt"));
    }

    #[test]
    fn pieces_are_drafted_edited_sewn_and_undone() {
        let mut s = Session::new();
        let square = json!([[0, 0], [200, 0], [200, 300], [0, 300]]);
        let a = text(&run(
            &mut s,
            "add_pattern",
            json!({ "name": "A", "points": square }),
        ))["id"]
            .as_u64()
            .unwrap();
        let b = text(&run(
            &mut s,
            "add_pattern",
            json!({ "name": "B", "points": square }),
        ))["id"]
            .as_u64()
            .unwrap();
        let curved = json!([[0, 0], [100, -40, "curve"], [200, 0], [100, 100]]);
        run(
            &mut s,
            "add_pattern",
            json!({ "name": "C", "points": curved }),
        );
        assert_eq!(s.doc.patterns[2].outline[1].kind, PointKind::Curve);

        let edits = json!({ "operations": [
            { "op": "move", "pattern": b, "dx": 300, "dy": 0 },
            { "op": "fabric", "pattern": a, "preset": "silk", "color": "#112233", "weight_gsm": 90 },
            { "op": "outline_dart", "pattern": a, "x": 100, "y": 300, "width": 30, "depth": 80 },
            { "op": "internal_line", "pattern": a, "points": [[20, 20], [180, 20]], "fold": "valley" },
            { "op": "seam_allowance", "pattern": a, "mm": 15 },
        ]});
        run(&mut s, "edit", edits);
        let p = s.doc.pattern(PatternId(a)).unwrap();
        assert_eq!(p.outline.len(), 7, "the dart's V adds three points");
        assert_eq!(p.fabric.name, "Silk");
        assert!((p.fabric.mass_per_area - 0.09).abs() < 1e-6);
        assert!(p.internal_lines[0].fold.is_some());
        assert_eq!(s.doc.sewing_lines.len(), 1, "the dart's legs");

        // A's right side to B's left side, both top to bottom.
        let sewn = text(&run(
            &mut s,
            "sew",
            json!({ "seams": [{ "a": [[b, 0, 3]], "b": [[a, 1, 2]] }] }),
        ));
        assert_eq!(sewn["seams"][0]["lengths_mm"], json!([300.0, 300.0]));
        let pattern = text(&run(&mut s, "get_pattern", json!({ "pattern": a })));
        assert_eq!(pattern["points"].as_array().unwrap().len(), 7);
        assert!(pattern["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["length_mm"] == 300.0));

        // Bad input is explained, not obeyed.
        assert!(fails(
            &mut s,
            "sew",
            json!({ "seams": [{ "a": [[a, 0, 9]], "b": [[b, 0, 1]] }] })
        )
        .contains("points 0 to"));
        let half = fails(
            &mut s,
            "edit",
            json!({ "operations": [
                { "op": "rename", "pattern": b, "name": "Back" },
                { "op": "spin", "pattern": b },
            ]}),
        );
        assert!(
            half.contains("operation 1") && half.contains("renamed"),
            "{half}"
        );
        assert!(fails(&mut s, "get_pattern", json!({ "pattern": 77 })).contains("77"));

        // Undo walks it all back.
        run(&mut s, "undo", json!({ "steps": 100 }));
        assert!(s.doc.patterns.is_empty());
        run(&mut s, "redo", json!({ "steps": 3 }));
        assert_eq!(s.doc.patterns.len(), 3);
    }

    #[test]
    fn avatar_and_arrangement() {
        let mut s = Session::new();
        run(
            &mut s,
            "set_avatar",
            json!({ "body": "male", "height": 185, "pose": "T-pose" }),
        );
        assert_eq!(s.avatar.kind, AvatarKind::Male);
        assert_eq!(s.avatar.measurements.height, 185.0);
        assert!(!s.doc.avatar_pose.is_rest());
        run(
            &mut s,
            "set_avatar",
            json!({ "pose": { "left_elbow": { "bend": 40 } } }),
        );
        assert_eq!(s.doc.avatar_pose.get(od_core::Joint::LeftElbow).bend, 40.0);
        assert!(fails(&mut s, "set_avatar", json!({ "pose": "Cartwheel" })).contains("Cartwheel"));

        let id = text(&run(
            &mut s,
            "add_pattern",
            json!({ "name": "Sleeve", "points": [[0, 0], [300, 0], [300, 400], [0, 400]] }),
        ))["id"]
            .clone();
        run(
            &mut s,
            "arrange",
            json!({ "pieces": [{ "pattern": id, "spot": "left_arm" }] }),
        );
        let p = &s.doc.patterns[0];
        assert!(
            p.arrangement.unwrap().center[0] > 0.0,
            "the avatar's left is +x"
        );
        assert_eq!(s.wraps.get(&p.id), Some(&Limb::LeftArm));
        run(
            &mut s,
            "arrange",
            json!({ "pieces": [{ "pattern": id, "position": [0, 1, 0.3], "facing": "front" }] }),
        );
        assert!(s.wraps.is_empty());
        assert_eq!(
            s.doc.patterns[0].arrangement.unwrap().center,
            [0.0, 1.0, 0.3]
        );
    }

    #[test]
    fn a_drape_runs_in_slices_and_carries_on() {
        let mut s = Session::new();
        assert!(fails(&mut s, "drape", json!({})).contains("nothing to drape"));
        run(&mut s, "create_garment", json!({ "type": "skirt" }));
        let first = text(&run(
            &mut s,
            "drape",
            json!({ "seconds": 0.1, "particle_distance": 40 }),
        ));
        assert_eq!(first["started_fresh"], true);
        assert!(first["particles"].as_u64().unwrap() > 100);
        assert!(first["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("still closing")));
        let more = text(&run(
            &mut s,
            "drape",
            json!({ "seconds": 0.1, "particle_distance": 40 }),
        ));
        assert_eq!(more["started_fresh"], false);
        assert!(more["simulated_seconds"].as_f64().unwrap() > 0.15);
        // A tiny budget answers early and says so.
        let cut = text(&run(
            &mut s,
            "drape",
            json!({ "seconds": 20, "particle_distance": 40, "max_wait_seconds": 1 }),
        ));
        assert!(
            cut["notes"][0]
                .as_str()
                .unwrap()
                .contains("call drape again"),
            "{cut}"
        );
        // Any edit starts the next drape afresh.
        run(
            &mut s,
            "edit",
            json!({ "operations": [{ "op": "rename", "pattern": 1, "name": "Front" }] }),
        );
        let again = text(&run(
            &mut s,
            "drape",
            json!({ "seconds": 0.05, "particle_distance": 40 }),
        ));
        assert_eq!(again["started_fresh"], true);
    }

    #[test]
    fn projects_save_open_and_export() {
        let dir = std::env::temp_dir().join(format!("od_mcp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("look.odproj");
        let mut s = Session::new();
        assert!(fails(&mut s, "save_project", json!({})).contains("path"));
        run(&mut s, "create_garment", json!({ "type": "skirt" }));
        run(
            &mut s,
            "save_project",
            json!({ "path": path.to_str().unwrap() }),
        );
        assert!(!s.history.is_dirty());
        run(
            &mut s,
            "export",
            json!({ "format": "dxf", "path": dir.join("look.dxf").to_str().unwrap() }),
        );
        assert!(fails(
            &mut s,
            "export",
            json!({ "format": "obj", "path": dir.join("look.obj").to_str().unwrap() })
        )
        .contains("drape"));

        let mut fresh = Session::new();
        run(
            &mut fresh,
            "open_project",
            json!({ "path": path.to_str().unwrap() }),
        );
        assert_eq!(fresh.doc, s.doc);
        run(&mut fresh, "new_project", json!({ "name": "Next" }));
        assert!(fresh.doc.patterns.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Renders when this machine has any GPU adapter, software ones
    /// included; otherwise only checks the error says why.
    #[test]
    fn renders_come_back_as_png_images() {
        let mut s = Session::new();
        run(&mut s, "create_garment", json!({ "type": "skirt" }));
        let out = call(
            &mut s,
            "render",
            &json!({ "view": "pattern", "width": 120, "height": 80 }),
        )
        .unwrap()
        .into_json();
        if out["isError"] == true {
            let why = out["content"][0]["text"].as_str().unwrap();
            assert!(why.contains("GPU"), "{why}");
            eprintln!("no GPU adapter; skipping the render check");
            return;
        }
        let image = &out["content"][1];
        assert_eq!(image["type"], "image");
        assert_eq!(image["mimeType"], "image/png");
        assert!(
            image["data"].as_str().unwrap().starts_with("iVBORw0KGgo"),
            "PNG signature"
        );
        let flat = call(
            &mut s,
            "render",
            &json!({ "view": "three_quarter", "width": 64, "height": 96, "style": "strain" }),
        )
        .unwrap()
        .into_json();
        assert_eq!(flat["isError"], false);
        assert!(fails(&mut s, "render", json!({ "view": "sideways" })).contains("sideways"));
    }
}
