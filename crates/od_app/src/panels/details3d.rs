//! Garment details over the 3D view: each zipper's two sides, on the
//! flat pieces before draping and along the cloth while it drapes
//! (dashed while the zipper is open), and buttonhole slits on the draped
//! cloth (drawn, like the other trims, only while draping).

use od_sim::details::EdgeKind;

use crate::app::Session;
use crate::panels::details::zipper_color;
use crate::panels::viewport3d::{flat_frames, Overlay};
use crate::theme;

/// Lines to draw over the 3D view for the document's details.
pub fn overlays(s: &Session, pal: &theme::Palette) -> Vec<Overlay> {
    let color = zipper_color(pal);
    if let Some(drape) = &s.viewport.drape {
        let holes = drape
            .buttonhole_positions()
            .into_iter()
            .map(|ends| Overlay {
                points: ends.to_vec(),
                color: pal.selected,
                dashed: false,
            });
        return drape
            .edge_detail_positions()
            .into_iter()
            .map(|(kind, points)| match kind {
                EdgeKind::Zipper { closed, .. } => Overlay {
                    points,
                    color,
                    dashed: !closed,
                },
            })
            .chain(holes)
            .collect();
    }
    let frames = flat_frames(s);
    let mut out = Vec::new();
    for line in &s.doc.sewing_lines {
        let Some(zipper) = line.details.zipper else {
            continue;
        };
        for edge in line.edges() {
            let (Some(pattern), Some(frame)) =
                (s.doc.pattern(edge.pattern), frames.get(&edge.pattern))
            else {
                continue;
            };
            out.push(Overlay {
                points: pattern
                    .edge_points(edge)
                    .into_iter()
                    .map(|p| frame.world(p))
                    .collect(),
                color,
                dashed: !zipper.closed,
            });
        }
    }
    out
}
