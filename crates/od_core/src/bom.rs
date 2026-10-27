//! Bill of materials: how much fabric each pattern piece needs, at whatever
//! size it is graded to. Reuses the same measurements the properties panel
//! already shows for the base size (`Pattern::area`/`perimeter`), just
//! applied to a [`Pattern::graded`] copy, since graded sizes use different
//! amounts of fabric.

use crate::document::{Document, Pattern, PatternId};

/// Fabric needed for one piece at one size.
#[derive(Debug, Clone, PartialEq)]
pub struct PieceBom {
    pub pattern: PatternId,
    pub name: String,
    /// Area inside the cutting line (outline, not the seam allowance), m².
    pub area_m2: f32,
    /// Length of the cutting line, mm.
    pub perimeter_mm: f32,
}

/// `pattern`'s area and perimeter graded `steps` sizes away from the
/// document's base size (see [`crate::document::SizeChart::steps`]).
pub fn piece_bom(pattern: &Pattern, steps: i32) -> PieceBom {
    let graded = pattern.graded(steps);
    PieceBom {
        pattern: pattern.id,
        name: pattern.name.clone(),
        area_m2: graded.area() / 1_000_000.0,
        perimeter_mm: graded.perimeter(),
    }
}

/// The whole document's bill of materials at size `steps` away from base,
/// one entry per pattern, in document order.
pub fn document_bom(doc: &Document, steps: i32) -> Vec<PieceBom> {
    doc.patterns.iter().map(|p| piece_bom(p, steps)).collect()
}

/// Total fabric area across every piece, m² — the number a yardage
/// estimate would scale up from (allowing for cutting layout waste, which
/// this doesn't model).
pub fn total_area_m2(boms: &[PieceBom]) -> f32 {
    boms.iter().map(|b| b.area_m2).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Grading, OutlinePoint, Pattern};
    use crate::geometry::{rectangle, Vec2};

    fn rect_pattern(id: u64, w: f32, h: f32) -> Pattern {
        Pattern::new(
            PatternId(id),
            "Panel",
            rectangle(Vec2::ZERO, Vec2::new(w, h))
                .into_iter()
                .map(OutlinePoint::corner)
                .collect(),
        )
    }

    #[test]
    fn bom_matches_area_and_perimeter_at_the_base_size() {
        let p = rect_pattern(1, 1000.0, 2000.0);
        let bom = piece_bom(&p, 0);
        assert_eq!(bom.area_m2, 2.0);
        assert_eq!(bom.perimeter_mm, 6000.0);
        assert_eq!(bom.name, "Panel");
    }

    #[test]
    fn grading_changes_fabric_needed_at_other_sizes() {
        let mut p = rect_pattern(1, 1000.0, 2000.0);
        p.grading = Grading::uniform_scale(&p.outline, 10.0);
        let base = piece_bom(&p, 0);
        let bigger = piece_bom(&p, 1);
        let smaller = piece_bom(&p, -1);
        assert!(bigger.area_m2 > base.area_m2);
        assert!(smaller.area_m2 < base.area_m2);
    }

    #[test]
    fn document_bom_covers_every_pattern_in_order() {
        let mut doc = Document::new("Test");
        doc.patterns.push(rect_pattern(1, 1000.0, 500.0));
        doc.patterns.push(rect_pattern(2, 300.0, 300.0));
        let boms = document_bom(&doc, 0);
        assert_eq!(boms.len(), 2);
        assert_eq!(boms[0].pattern, PatternId(1));
        assert_eq!(boms[1].pattern, PatternId(2));
        assert!((total_area_m2(&boms) - (0.5 + 0.09)).abs() < 1e-4);
    }
}
