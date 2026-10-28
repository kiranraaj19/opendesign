//! DXF-AAMA export: 2D patterns (outline, seam allowance, grainline,
//! notches, internal lines) as an ASCII DXF (R12 / `AC1009`) file, the
//! common interchange format between pattern-making and garment CAD tools.
//!
//! This is a practical subset of the AAMA-DXF convention, not a byte-exact
//! reproduction of the full ASTM/AAMA spec: every pattern piece gets its own
//! family of layers, named after the sanitised pattern name plus its id (so
//! two pieces can never collide) — `<PIECE>` for the boundary (cutting
//! line), `<PIECE>_SEAM` for the seam allowance, `<PIECE>_GRAIN` for the
//! grainline, `<PIECE>_NOTCH` for notch ticks, `<PIECE>_INTERNAL` for
//! internal lines, `<PIECE>_FOLD` for fold lines (each labelled with a
//! `TEXT` such as `VALLEY 170`: its kind and angle in degrees) and
//! `<PIECE>_DART` for diamond darts (closed: tip, side, tip, side). Outline
//! darts are part of the boundary. Entities are classic
//! `POLYLINE`/`VERTEX`/`SEQEND`, `LINE` and `TEXT`, which every DXF reader
//! back to R12 understands (newer
//! `LWPOLYLINE`-only readers also accept them). Units are millimetres
//! throughout (`$INSUNITS` 4).

use std::fs;
use std::io;
use std::path::Path;

use crate::document::{Document, Pattern, TOLERANCE};
use crate::geometry::{self, Vec2};

/// Half-length of a notch tick mark, in mm, drawn across the outline.
const NOTCH_HALF_LENGTH: f32 = 3.0;

/// Height of the label on a fold line, in mm.
const LABEL_HEIGHT: f32 = 5.0;

/// The document's patterns as DXF text.
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    write_header(&mut out);
    let mut layers = Vec::new();
    let mut entities = String::new();
    write_entities(&mut entities, doc, &mut layers);
    write_layer_table(&mut out, &layers);
    out.push_str(&entities);
    out.push_str("0\nEOF\n");
    out
}

/// Write the document's patterns to `path` as DXF.
pub fn export(doc: &Document, path: &Path) -> io::Result<()> {
    fs::write(path, write(doc))
}

fn write_header(out: &mut String) {
    out.push_str("0\nSECTION\n2\nHEADER\n");
    out.push_str("9\n$ACADVER\n1\nAC1009\n");
    // 4 = millimetres.
    out.push_str("9\n$INSUNITS\n70\n4\n");
    out.push_str("0\nENDSEC\n");
}

fn write_layer_table(out: &mut String, layers: &[String]) {
    out.push_str("0\nSECTION\n2\nTABLES\n");
    out.push_str("0\nTABLE\n2\nLAYER\n");
    out.push_str(&format!("70\n{}\n", layers.len()));
    for layer in layers {
        out.push_str("0\nLAYER\n");
        out.push_str(&format!("2\n{layer}\n"));
        out.push_str("70\n0\n");
        out.push_str("62\n7\n");
        out.push_str("6\nCONTINUOUS\n");
    }
    out.push_str("0\nENDTAB\n0\nENDSEC\n");
}

/// `name` turned into a DXF-safe layer fragment: upper-case, with runs of
/// anything but letters or digits collapsed to one underscore.
fn sanitize(name: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "PIECE".to_string()
    } else {
        out
    }
}

/// The base layer name for a piece: unique even if two pieces share a name.
fn piece_layer(pattern: &Pattern) -> String {
    format!("{}_{}", sanitize(&pattern.name), pattern.id.0)
}

fn write_polyline(out: &mut String, layer: &str, points: &[Vec2], closed: bool) {
    if points.len() < 2 {
        return;
    }
    out.push_str("0\nPOLYLINE\n");
    out.push_str(&format!("8\n{layer}\n"));
    out.push_str("66\n1\n");
    out.push_str(&format!("70\n{}\n", i32::from(closed)));
    for p in points {
        out.push_str("0\nVERTEX\n");
        out.push_str(&format!("8\n{layer}\n"));
        out.push_str(&format!("10\n{:.4}\n20\n{:.4}\n30\n0.0\n", p.x, p.y));
    }
    out.push_str("0\nSEQEND\n");
}

fn write_text(out: &mut String, layer: &str, at: Vec2, text: &str) {
    out.push_str("0\nTEXT\n");
    out.push_str(&format!("8\n{layer}\n"));
    out.push_str(&format!("10\n{:.4}\n20\n{:.4}\n30\n0.0\n", at.x, at.y));
    out.push_str(&format!("40\n{LABEL_HEIGHT:.1}\n1\n{text}\n"));
}

fn write_line(out: &mut String, layer: &str, a: Vec2, b: Vec2) {
    out.push_str("0\nLINE\n");
    out.push_str(&format!("8\n{layer}\n"));
    out.push_str(&format!("10\n{:.4}\n20\n{:.4}\n30\n0.0\n", a.x, a.y));
    out.push_str(&format!("11\n{:.4}\n21\n{:.4}\n31\n0.0\n", b.x, b.y));
}

/// The grainline's two endpoints, mirroring `pattern2d::draw_grainline`'s
/// layout: a double-headed arrow through the pattern's centre, scaled to
/// its bounds.
fn grainline_endpoints(p: &Pattern) -> Option<(Vec2, Vec2)> {
    let (min, max) = p.bounds()?;
    let size = max - min;
    let center = p.center();
    let (sin, cos) = p.grain_angle.to_radians().sin_cos();
    let dir = Vec2::new(-sin, cos);
    let extent = dir.x.abs() * size.x + dir.y.abs() * size.y;
    let half = dir * (extent * 0.3);
    Some((center - half, center + half))
}

fn write_entities(out: &mut String, doc: &Document, layers: &mut Vec<String>) {
    out.push_str("0\nSECTION\n2\nENTITIES\n");
    for p in &doc.patterns {
        let base = piece_layer(p);
        let outline = p.polyline(TOLERANCE);

        layers.push(base.clone());
        write_polyline(out, &base, &outline, true);

        if p.seam_allowance > 0.0 {
            let seam = geometry::offset_outline(&outline, p.seam_allowance);
            if !seam.is_empty() {
                let seam_layer = format!("{base}_SEAM");
                layers.push(seam_layer.clone());
                write_polyline(out, &seam_layer, &seam, true);
            }
        }

        if let Some((a, b)) = grainline_endpoints(p) {
            let grain_layer = format!("{base}_GRAIN");
            layers.push(grain_layer.clone());
            write_line(out, &grain_layer, a, b);
        }

        if !p.notches.is_empty() {
            let notch_layer = format!("{base}_NOTCH");
            layers.push(notch_layer.clone());
            for notch in &p.notches {
                if let Some((pos, dir)) = geometry::point_at_distance(&outline, notch.at) {
                    let perp = dir.perp();
                    write_line(
                        out,
                        &notch_layer,
                        pos - perp * NOTCH_HALF_LENGTH,
                        pos + perp * NOTCH_HALF_LENGTH,
                    );
                }
            }
        }

        let (folds, plain): (Vec<_>, Vec<_>) =
            p.internal_lines.iter().partition(|l| l.fold.is_some());
        if !plain.is_empty() {
            let internal_layer = format!("{base}_INTERNAL");
            layers.push(internal_layer.clone());
            for line in plain {
                write_polyline(out, &internal_layer, &line.points, false);
            }
        }
        if !folds.is_empty() {
            let fold_layer = format!("{base}_FOLD");
            layers.push(fold_layer.clone());
            for line in folds {
                write_polyline(out, &fold_layer, &line.points, false);
                if let (Some(fold), Some(&at)) = (line.fold, line.points.first()) {
                    let kind = if fold.angle < 0.0 {
                        "MOUNTAIN"
                    } else {
                        "VALLEY"
                    };
                    let label = format!("{kind} {:.0}", fold.angle.abs());
                    write_text(out, &fold_layer, at, &label);
                }
            }
        }

        if !p.darts.is_empty() {
            let dart_layer = format!("{base}_DART");
            layers.push(dart_layer.clone());
            for dart in &p.darts {
                let [a, b] = dart.sides();
                write_polyline(out, &dart_layer, &[dart.start, a, dart.end, b], true);
            }
        }
    }
    out.push_str("0\nENDSEC\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{InternalLine, Notch, OutlinePoint, PatternId};

    fn sample_doc() -> Document {
        let mut doc = Document::new("Test");
        let mut p = Pattern::new(
            PatternId(1),
            "Front Panel",
            vec![
                OutlinePoint::corner(Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(Vec2::new(100.0, 0.0)),
                OutlinePoint::corner(Vec2::new(100.0, 200.0)),
                OutlinePoint::corner(Vec2::new(0.0, 200.0)),
            ],
        );
        p.seam_allowance = 10.0;
        p.grain_angle = 0.0;
        p.notches.push(Notch { at: 50.0 });
        p.internal_lines.push(InternalLine::new(vec![
            Vec2::new(10.0, 10.0),
            Vec2::new(20.0, 20.0),
        ]));
        doc.patterns.push(p);
        doc
    }

    #[test]
    fn exports_a_layer_and_entity_for_every_element() {
        let doc = sample_doc();
        let dxf = write(&doc);
        assert!(dxf.starts_with("0\nSECTION\n2\nHEADER"));
        assert!(dxf.trim_end().ends_with("0\nEOF"));
        for layer in [
            "FRONT_PANEL_1",
            "FRONT_PANEL_1_SEAM",
            "FRONT_PANEL_1_GRAIN",
            "FRONT_PANEL_1_NOTCH",
            "FRONT_PANEL_1_INTERNAL",
        ] {
            assert!(dxf.contains(&format!("2\n{layer}\n")), "{layer} missing");
        }
        assert_eq!(dxf.matches("0\nLAYER\n").count(), 5);
        assert_eq!(dxf.matches("0\nPOLYLINE\n").count(), 3); // boundary, seam, internal
        assert_eq!(dxf.matches("0\nLINE\n").count(), 2); // grainline, notch
    }

    #[test]
    fn boundary_polyline_has_a_vertex_per_outline_point_and_is_closed() {
        let doc = sample_doc();
        let dxf = write(&doc);
        assert_eq!(
            dxf.matches("0\nVERTEX\n8\nFRONT_PANEL_1\n").count(),
            4,
            "one vertex per corner"
        );
        assert!(dxf.contains("8\nFRONT_PANEL_1\n66\n1\n70\n1\n"));
    }

    #[test]
    fn no_seam_allowance_skips_the_seam_layer() {
        let mut doc = sample_doc();
        doc.patterns[0].seam_allowance = 0.0;
        let dxf = write(&doc);
        assert!(!dxf.contains("_SEAM"));
    }

    #[test]
    fn no_notches_or_internal_lines_skips_those_layers() {
        let mut doc = sample_doc();
        doc.patterns[0].notches.clear();
        doc.patterns[0].internal_lines.clear();
        let dxf = write(&doc);
        assert!(!dxf.contains("_NOTCH"));
        assert!(!dxf.contains("_INTERNAL"));
        // Boundary, seam and grain layers remain.
        assert_eq!(dxf.matches("0\nLAYER\n").count(), 3);
    }

    #[test]
    fn two_patterns_get_distinct_layers_even_with_the_same_name() {
        let mut doc = sample_doc();
        doc.patterns.push(Pattern::new(
            PatternId(2),
            "Front Panel",
            vec![
                OutlinePoint::corner(Vec2::new(0.0, 0.0)),
                OutlinePoint::corner(Vec2::new(50.0, 0.0)),
                OutlinePoint::corner(Vec2::new(50.0, 50.0)),
            ],
        ));
        let dxf = write(&doc);
        assert!(dxf.contains("2\nFRONT_PANEL_1\n"));
        assert!(dxf.contains("2\nFRONT_PANEL_2\n"));
    }

    #[test]
    fn fold_lines_and_darts_get_their_own_layers() {
        use crate::document::{Dart, Fold};
        let mut doc = sample_doc();
        let p = &mut doc.patterns[0];
        p.internal_lines.push(InternalLine {
            points: vec![Vec2::new(0.0, 100.0), Vec2::new(100.0, 100.0)],
            fold: Some(Fold::mountain()),
        });
        p.darts.push(Dart {
            start: Vec2::new(50.0, 20.0),
            end: Vec2::new(50.0, 80.0),
            width: 10.0,
        });
        let dxf = write(&doc);
        for layer in ["FRONT_PANEL_1_FOLD", "FRONT_PANEL_1_DART"] {
            assert!(dxf.contains(&format!("2\n{layer}\n")), "{layer} missing");
        }
        // The plain line stays an internal line; the fold is labelled.
        assert_eq!(
            dxf.matches("0\nPOLYLINE\n8\nFRONT_PANEL_1_INTERNAL\n")
                .count(),
            1
        );
        assert!(dxf.contains("0\nTEXT\n8\nFRONT_PANEL_1_FOLD\n"));
        assert!(dxf.contains("1\nMOUNTAIN 170\n"));
        // The dart is a closed diamond: tip, side, tip, side.
        assert!(dxf.contains("8\nFRONT_PANEL_1_DART\n66\n1\n70\n1\n"));
        assert_eq!(dxf.matches("0\nVERTEX\n8\nFRONT_PANEL_1_DART\n").count(), 4);
        assert!(dxf.contains("10\n45.0000\n20\n50.0000\n"));
        assert!(dxf.contains("10\n55.0000\n20\n50.0000\n"));
    }

    #[test]
    fn sanitize_handles_odd_names() {
        assert_eq!(sanitize("  weird!! name--2  "), "WEIRD_NAME_2");
        assert_eq!(sanitize(""), "PIECE");
    }

    #[test]
    fn exports_to_a_file() {
        let dir = std::env::temp_dir().join(format!("od_core_dxf_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("patterns.dxf");
        export(&sample_doc(), &path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("FRONT_PANEL_1"));
    }
}
