//! Basic PDF export: one page per pattern piece tracing its (graded)
//! outline and seam allowance to scale, followed by a bill-of-materials
//! summary table — not a full print-layout/tiling system (out of scope for
//! this pass): a piece larger than the page is simply scaled down to fit,
//! with the scale noted on the page.

use printpdf::{
    BuiltinFont, Color, Line, LinePoint, Mm, Op, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Pt, Rgb, TextItem,
};

use od_core::bom::{document_bom, total_area_m2};
use od_core::document::TOLERANCE;
use od_core::geometry::{self, Vec2};
use od_core::{Document, Pattern};

const PAGE_WIDTH_MM: f32 = 210.0;
const PAGE_HEIGHT_MM: f32 = 297.0;
const MARGIN_MM: f32 = 15.0;
/// Vertical space reserved at the top of a piece page for its header text.
const HEADER_MM: f32 = 20.0;

fn ink() -> Color {
    Color::Rgb(Rgb {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        icc_profile: None,
    })
}

/// A left-aligned line of Helvetica text with its baseline at `(x, y)` mm
/// from the bottom-left of the page.
fn text_ops(text: &str, x: f32, y: f32, size: f32) -> Vec<Op> {
    vec![
        Op::StartTextSection,
        Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
            size: Pt(size),
        },
        Op::SetTextCursor {
            pos: Point::new(Mm(x), Mm(y)),
        },
        Op::ShowText {
            items: vec![TextItem::Text(text.to_string())],
        },
        Op::EndTextSection,
    ]
}

/// A closed polyline, mapped from pattern mm (y increasing downward, as
/// drawn in the 2D canvas) into page mm (y increasing upward, as PDF
/// expects) by `to_page`.
fn outline_ops(points: &[Vec2], to_page: impl Fn(Vec2) -> (f32, f32)) -> Vec<Op> {
    if points.len() < 2 {
        return Vec::new();
    }
    let line = Line {
        points: points
            .iter()
            .map(|&p| {
                let (x, y) = to_page(p);
                LinePoint {
                    p: Point::new(Mm(x), Mm(y)),
                    bezier: false,
                }
            })
            .collect(),
        is_closed: true,
    };
    vec![
        Op::SetOutlineColor { col: ink() },
        Op::SetOutlineThickness { pt: Pt(0.75) },
        Op::DrawLine { line },
    ]
}

/// One page tracing `pattern` graded `steps` sizes from the base, scaled to
/// fit the page if it doesn't fit at 1:1.
fn piece_page(pattern: &Pattern, steps: i32, size_name: &str) -> PdfPage {
    let graded = pattern.graded(steps);
    let outline = graded.polyline(TOLERANCE);
    let seam = (graded.seam_allowance > 0.0)
        .then(|| geometry::offset_outline(&outline, graded.seam_allowance))
        .filter(|s| !s.is_empty());

    let traced = seam.as_deref().unwrap_or(&outline);
    let (min, max) = geometry::bounds(traced).unwrap_or((Vec2::ZERO, Vec2::ZERO));
    let size = Vec2::new((max.x - min.x).max(1.0), (max.y - min.y).max(1.0));
    let usable_w = PAGE_WIDTH_MM - 2.0 * MARGIN_MM;
    let usable_h = PAGE_HEIGHT_MM - 2.0 * MARGIN_MM - HEADER_MM;
    // mm of page per mm of pattern; never scaled up, so a small piece still
    // measures true to a ruler.
    let scale = (usable_w / size.x).min(usable_h / size.y).min(1.0);

    let to_page = move |p: Vec2| {
        let x = MARGIN_MM + (p.x - min.x) * scale;
        let y = PAGE_HEIGHT_MM - MARGIN_MM - HEADER_MM - (p.y - min.y) * scale;
        (x, y)
    };

    let mut ops = text_ops(
        &format!("{} — size {size_name}", pattern.name),
        MARGIN_MM,
        PAGE_HEIGHT_MM - MARGIN_MM,
        14.0,
    );
    let scale_note = if scale < 0.999 {
        format!("Scale 1:{:.1} (too large to print to scale)", 1.0 / scale)
    } else {
        "Scale 1:1".to_string()
    };
    ops.extend(text_ops(
        &format!(
            "{scale_note} · area {:.2} m² · cutting line {:.0} mm",
            graded.area() / 1_000_000.0,
            graded.perimeter(),
        ),
        MARGIN_MM,
        PAGE_HEIGHT_MM - MARGIN_MM - 7.0,
        10.0,
    ));
    ops.extend(outline_ops(&outline, to_page));
    if let Some(seam) = &seam {
        ops.extend(outline_ops(seam, to_page));
    }
    PdfPage::new(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), ops)
}

/// A summary page listing every pattern's fabric needed at size `steps`
/// away from the document's base size, plus the total.
fn bom_page(doc: &Document, steps: i32, size_name: &str) -> PdfPage {
    let boms = document_bom(doc, steps);
    let mut y = PAGE_HEIGHT_MM - MARGIN_MM;
    let mut ops = text_ops(
        &format!("{} — bill of materials, size {size_name}", doc.name),
        MARGIN_MM,
        y,
        14.0,
    );
    y -= 12.0;
    ops.extend(text_ops("Piece", MARGIN_MM, y, 10.0));
    ops.extend(text_ops("Area (m²)", MARGIN_MM + 90.0, y, 10.0));
    ops.extend(text_ops("Cutting line (mm)", MARGIN_MM + 130.0, y, 10.0));
    y -= 7.0;
    for piece in &boms {
        ops.extend(text_ops(&piece.name, MARGIN_MM, y, 10.0));
        ops.extend(text_ops(
            &format!("{:.2}", piece.area_m2),
            MARGIN_MM + 90.0,
            y,
            10.0,
        ));
        ops.extend(text_ops(
            &format!("{:.0}", piece.perimeter_mm),
            MARGIN_MM + 130.0,
            y,
            10.0,
        ));
        y -= 7.0;
    }
    y -= 5.0;
    ops.extend(text_ops(
        &format!("Total fabric area: {:.2} m²", total_area_m2(&boms)),
        MARGIN_MM,
        y,
        11.0,
    ));
    PdfPage::new(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), ops)
}

/// The document's bill of materials and pattern pieces, graded to size
/// `steps` away from the base size, as a PDF: one summary page followed by
/// one page per pattern piece.
pub fn write(doc: &Document, steps: i32) -> Vec<u8> {
    let size_name = doc
        .size_chart
        .names
        .get((doc.size_chart.base as i32 + steps).max(0) as usize)
        .map_or(doc.size_chart.base_name(), String::as_str);
    let mut pdf = PdfDocument::new(&doc.name);
    pdf.pages.push(bom_page(doc, steps, size_name));
    for pattern in &doc.patterns {
        pdf.pages.push(piece_page(pattern, steps, size_name));
    }
    pdf.save(&PdfSaveOptions::default(), &mut Vec::new())
}

/// Write the document's PDF summary and pattern pages to `path`.
pub fn export(doc: &Document, steps: i32, path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, write(doc, steps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use od_core::geometry::rectangle;
    use od_core::{OutlinePoint, PatternId};

    fn sample_doc() -> Document {
        let mut doc = Document::new("Test Shirt");
        let mut p = Pattern::new(
            PatternId(1),
            "Front",
            rectangle(Vec2::ZERO, Vec2::new(400.0, 600.0))
                .into_iter()
                .map(OutlinePoint::corner)
                .collect(),
        );
        p.seam_allowance = 10.0;
        doc.patterns.push(p);
        doc
    }

    #[test]
    fn writes_a_page_per_piece_plus_a_bom_summary() {
        let doc = sample_doc();
        let bytes = write(&doc, 0);
        assert!(bytes.starts_with(b"%PDF"));

        let mut warnings = Vec::new();
        let parsed =
            PdfDocument::parse(&bytes, &printpdf::PdfParseOptions::default(), &mut warnings)
                .expect("the written PDF should parse back");
        // One BOM summary page plus one page per pattern piece.
        assert_eq!(parsed.pages.len(), 1 + doc.patterns.len());
    }

    #[test]
    fn grading_changes_the_traced_piece_size() {
        let mut doc = sample_doc();
        doc.patterns[0].grading = od_core::Grading::uniform_scale(&doc.patterns[0].outline, 10.0);
        let base = write(&doc, 0);
        let bigger = write(&doc, 1);
        // Different outlines produce different PDF bytes; a weak check, but
        // enough to show grading actually reaches the exported page.
        assert_ne!(base, bigger);
    }

    #[test]
    fn export_writes_a_readable_file() {
        let doc = sample_doc();
        let dir = std::env::temp_dir().join(format!("od_pdf_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shirt.pdf");
        export(&doc, 0, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        std::fs::remove_file(&path).unwrap();
    }
}
