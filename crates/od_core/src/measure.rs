//! Garment measurements: named lengths taken on the flat patterns (a
//! straight line between two outline points, or a stretch along one
//! outline), kept in the document and read at every size of the size
//! chart from the graded outlines (see [`Pattern::graded`]).
//!
//! Points are kept as outline parameters (see
//! [`crate::document::FlatPoint::param`]), not positions, so a measurement
//! follows its pattern when it is moved, turned or flipped, and lands on
//! the same place of every graded size.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::document::{Document, PatternId, SeamEdge, TOLERANCE};
use crate::geometry::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MeasurementId(pub u64);

/// A point on a pattern's outline, by outline parameter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OutlineSpot {
    pub pattern: PatternId,
    pub param: f32,
}

/// What a [`Measurement`] measures.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureKind {
    /// The straight distance between two outline points, on one pattern
    /// or two (a shoulder-to-shoulder width across two pieces, say).
    Distance { a: OutlineSpot, b: OutlineSpot },
    /// The length of a stretch of one pattern's outline (an armhole, a
    /// neckline), as an edge like a seam's: `start..end`, wrapping past
    /// the end of the outline if `end` exceeds its point count.
    Along(SeamEdge),
}

/// A named measurement of the garment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub id: MeasurementId,
    pub name: String,
    pub kind: MeasureKind,
}

impl Measurement {
    /// The patterns it is taken on.
    pub fn patterns(&self) -> Vec<PatternId> {
        match self.kind {
            MeasureKind::Distance { a, b } if a.pattern != b.pattern => vec![a.pattern, b.pattern],
            MeasureKind::Distance { a, .. } => vec![a.pattern],
            MeasureKind::Along(edge) => vec![edge.pattern],
        }
    }

    /// The line it measures, in mm, on the patterns graded `steps` sizes
    /// from the base size: two points for a distance, the outline stretch
    /// for a length along it. `None` if a pattern it is on is gone.
    pub fn points_at(&self, doc: &Document, steps: i32) -> Option<Vec<Vec2>> {
        let graded = |id| doc.pattern(id).map(|p| p.graded(steps));
        match self.kind {
            MeasureKind::Distance { a, b } => {
                let at =
                    |s: OutlineSpot| graded(s.pattern).map(|p| p.pos_at_param(s.param, TOLERANCE));
                Some(vec![at(a)?, at(b)?])
            }
            MeasureKind::Along(edge) => Some(graded(edge.pattern)?.edge_points(&edge)),
        }
    }

    /// How long it is in mm at `steps` sizes from the base size. `None`
    /// if a pattern it is on is gone.
    pub fn length_at(&self, doc: &Document, steps: i32) -> Option<f32> {
        let points = self.points_at(doc, steps)?;
        Some(points.windows(2).map(|w| w[0].distance(w[1])).sum())
    }
}

/// One row of a [`chart`]: a measurement's name and its length in mm at
/// each size of the size chart, smallest first.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRow {
    pub id: MeasurementId,
    pub name: String,
    pub lengths: Vec<Option<f32>>,
}

/// Every measurement at every size of the document's size chart.
pub fn chart(doc: &Document) -> Vec<ChartRow> {
    let chart = &doc.size_chart;
    doc.measurements
        .iter()
        .map(|m| ChartRow {
            id: m.id,
            name: m.name.clone(),
            lengths: (0..chart.names.len())
                .map(|i| m.length_at(doc, chart.steps(i)))
                .collect(),
        })
        .collect()
}

/// The [`chart`] as comma-separated values with a header row of size
/// names, lengths in mm to one decimal place (blank where a pattern is
/// gone), for pasting into a spreadsheet.
pub fn chart_csv(doc: &Document) -> String {
    let field = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    let mut out = String::from("Measurement");
    for name in &doc.size_chart.names {
        out.push(',');
        out.push_str(&field(name));
    }
    out.push('\n');
    for row in chart(doc) {
        out.push_str(&field(&row.name));
        for length in row.lengths {
            out.push(',');
            if let Some(mm) = length {
                out.push_str(&format!("{mm:.1}"));
            }
        }
        out.push('\n');
    }
    out
}

impl Document {
    pub fn measurement(&self, id: MeasurementId) -> Option<&Measurement> {
        self.measurements.iter().find(|m| m.id == id)
    }

    /// Reserve a fresh measurement id. Ids are never reused, even after
    /// undo; files saved before measurements existed start counting from
    /// the highest id they hold.
    pub fn allocate_measurement_id(&mut self) -> MeasurementId {
        let max = self.measurements.iter().map(|m| m.id.0).max().unwrap_or(0);
        self.next_measurement_id = self.next_measurement_id.max(max + 1);
        let id = MeasurementId(self.next_measurement_id);
        self.next_measurement_id += 1;
        id
    }
}

/// Keep a measurement in the document. With no name, it is called
/// "Measurement N" after its id.
pub struct AddMeasurement {
    name: Option<String>,
    kind: MeasureKind,
    /// Assigned on first apply so redo recreates the same id.
    id: Option<MeasurementId>,
}

impl AddMeasurement {
    pub fn new(name: Option<String>, kind: MeasureKind) -> Self {
        Self {
            name,
            kind,
            id: None,
        }
    }

    /// Id of the new measurement, available after the command has run.
    pub fn id(&self) -> Option<MeasurementId> {
        self.id
    }
}

impl Command for AddMeasurement {
    fn label(&self) -> String {
        "Add measurement".into()
    }

    fn apply(&mut self, doc: &mut Document) {
        let id = *self.id.get_or_insert_with(|| doc.allocate_measurement_id());
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| format!("Measurement {}", id.0));
        doc.measurements.push(Measurement {
            id,
            name,
            kind: self.kind,
        });
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some(id) = self.id {
            doc.measurements.retain(|m| m.id != id);
        }
    }
}

pub struct RemoveMeasurement {
    id: MeasurementId,
    /// The removed measurement and where it was, kept for undo.
    removed: Option<(usize, Measurement)>,
}

impl RemoveMeasurement {
    pub fn new(id: MeasurementId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for RemoveMeasurement {
    fn label(&self) -> String {
        match &self.removed {
            Some((_, m)) => format!("Remove {}", m.name),
            None => "Remove measurement".into(),
        }
    }

    fn apply(&mut self, doc: &mut Document) {
        if let Some(index) = doc.measurements.iter().position(|m| m.id == self.id) {
            self.removed = Some((index, doc.measurements.remove(index)));
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((index, m)) = self.removed.take() {
            doc.measurements
                .insert(index.min(doc.measurements.len()), m);
        }
    }
}

pub struct RenameMeasurement {
    id: MeasurementId,
    name: String,
}

impl RenameMeasurement {
    pub fn new(id: MeasurementId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

impl Command for RenameMeasurement {
    fn label(&self) -> String {
        format!("Rename to {}", self.name)
    }

    // Swapping makes apply and undo the same operation.
    fn apply(&mut self, doc: &mut Document) {
        if let Some(m) = doc.measurements.iter_mut().find(|m| m.id == self.id) {
            std::mem::swap(&mut m.name, &mut self.name);
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        self.apply(doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Grading, OutlinePoint, Pattern};
    use crate::geometry;
    use crate::{History, MovePattern, RotatePattern};

    /// A 100 × 50 rectangle (corners 0..4 from the top left, clockwise on
    /// screen), graded 10% per size.
    fn doc() -> (Document, PatternId) {
        let mut doc = Document::new("Test");
        let id = doc.allocate_id();
        let mut p = Pattern::new(
            id,
            "Panel",
            geometry::rectangle(Vec2::ZERO, Vec2::new(100.0, 50.0))
                .into_iter()
                .map(OutlinePoint::corner)
                .collect(),
        );
        p.grading = Grading::uniform_scale(&p.outline, 10.0);
        doc.patterns.push(p);
        (doc, id)
    }

    fn spot(pattern: PatternId, param: f32) -> OutlineSpot {
        OutlineSpot { pattern, param }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_distance_and_a_length_along_grade_with_the_size() {
        let (doc, id) = doc();
        // Corner to opposite corner, and the top edge plus the right one.
        let diagonal = Measurement {
            id: MeasurementId(1),
            name: "Diagonal".into(),
            kind: MeasureKind::Distance {
                a: spot(id, 0.0),
                b: spot(id, 2.0),
            },
        };
        let along = Measurement {
            id: MeasurementId(2),
            name: "Top and side".into(),
            kind: MeasureKind::Along(doc.patterns[0].shorter_edge(0.0, 2.0)),
        };
        let d = 100f32.hypot(50.0);
        assert!(close(diagonal.length_at(&doc, 0).unwrap(), d));
        assert!(close(along.length_at(&doc, 0).unwrap(), 150.0));
        // 10% per size from the centre: every length scales by 1.1, 1.2...
        assert!(close(diagonal.length_at(&doc, 1).unwrap(), d * 1.1));
        assert!(close(along.length_at(&doc, 2).unwrap(), 150.0 * 1.2));
        assert!(close(along.length_at(&doc, -1).unwrap(), 150.0 * 0.9));
        // Half way along the bottom edge (param 2.5) to the top left.
        let mid = Measurement {
            kind: MeasureKind::Distance {
                a: spot(id, 2.5),
                b: spot(id, 0.0),
            },
            ..diagonal.clone()
        };
        assert!(close(mid.length_at(&doc, 0).unwrap(), 50f32.hypot(50.0)));
    }

    #[test]
    fn the_chart_has_every_size_and_a_csv_form() {
        let (mut doc, id) = doc();
        let mut history = History::new();
        history.execute(
            &mut doc,
            Box::new(AddMeasurement::new(
                Some("Width, top".into()),
                MeasureKind::Distance {
                    a: spot(id, 0.0),
                    b: spot(id, 1.0),
                },
            )),
        );
        let rows = chart(&doc);
        assert_eq!(rows.len(), 1);
        let lengths: Vec<f32> = rows[0].lengths.iter().map(|l| l.unwrap()).collect();
        assert_eq!(lengths.len(), 4); // S, M, L, XL
        for (got, want) in lengths.iter().zip([90.0, 100.0, 110.0, 120.0]) {
            assert!(close(*got, want), "{lengths:?}");
        }
        assert_eq!(
            chart_csv(&doc),
            "Measurement,S,M,L,XL\n\"Width, top\",90.0,100.0,110.0,120.0\n"
        );
    }

    #[test]
    fn measurements_follow_their_pattern_and_outlive_it_unmeasured() {
        let (mut doc, id) = doc();
        let mut history = History::new();
        history.execute(
            &mut doc,
            Box::new(AddMeasurement::new(
                None,
                MeasureKind::Distance {
                    a: spot(id, 0.0),
                    b: spot(id, 1.0),
                },
            )),
        );
        let m = doc.measurements[0].clone();
        assert_eq!(m.name, "Measurement 1");
        // Moving or turning the piece moves the measured points with it.
        history.execute(
            &mut doc,
            Box::new(MovePattern::new(id, Vec2::new(30.0, 7.0))),
        );
        history.execute(&mut doc, Box::new(RotatePattern::new(id, 37.0)));
        let points = m.points_at(&doc, 0).unwrap();
        assert_eq!(points[0], doc.patterns[0].outline[0].pos);
        assert!(close(m.length_at(&doc, 0).unwrap(), 100.0));
        // A deleted pattern leaves the measurement with no length.
        history.execute(&mut doc, Box::new(crate::DeletePattern::new(id)));
        assert_eq!(m.length_at(&doc, 0), None);
        assert_eq!(chart(&doc)[0].lengths, vec![None; 4]);
        assert!(chart_csv(&doc).ends_with("Measurement 1,,,,\n"));
    }

    #[test]
    fn add_rename_and_remove_undo_exactly() {
        let (mut doc, id) = doc();
        let original = doc.clone();
        let mut history = History::new();
        let kind = MeasureKind::Along(SeamEdge {
            pattern: id,
            start: 3.0,
            end: 5.0,
            forward: true,
        });
        history.execute(&mut doc, Box::new(AddMeasurement::new(None, kind)));
        history.execute(&mut doc, Box::new(AddMeasurement::new(None, kind)));
        let [first, second] = [doc.measurements[0].id, doc.measurements[1].id];
        assert_ne!(first, second);
        // The wrapped stretch: the left side and the top.
        assert!(close(
            doc.measurements[0].length_at(&doc, 0).unwrap(),
            150.0
        ));
        let added = doc.clone();
        history.execute(&mut doc, Box::new(RenameMeasurement::new(first, "Hem")));
        assert_eq!(doc.measurement(first).unwrap().name, "Hem");
        history.execute(&mut doc, Box::new(RemoveMeasurement::new(first)));
        assert_eq!(doc.measurements.len(), 1);
        history.undo(&mut doc);
        history.undo(&mut doc);
        assert_eq!(doc, added);
        history.undo(&mut doc);
        history.undo(&mut doc);
        assert!(doc.measurements.is_empty());
        // Redo brings back the same ids; a new one is never reused.
        history.redo(&mut doc);
        assert_eq!(doc.measurements[0].id, first);
        history.undo(&mut doc);
        assert_eq!(doc.measurements, original.measurements);
        history.execute(&mut doc, Box::new(AddMeasurement::new(None, kind)));
        assert!(doc.measurements[0].id.0 > second.0);
    }

    #[test]
    fn files_without_measurements_still_load() {
        let (doc, _) = doc();
        let mut json: serde_json::Value =
            serde_json::from_str(&crate::project::to_json(&doc)).unwrap();
        let body = json["document"].as_object_mut().unwrap();
        body.remove("measurements");
        body.remove("next_measurement_id");
        let loaded = crate::project::from_json(&json.to_string()).unwrap();
        assert!(loaded.measurements.is_empty());
        // And measurements survive a save.
        let mut doc = doc;
        let id = doc.patterns[0].id;
        AddMeasurement::new(
            Some("Top".into()),
            MeasureKind::Distance {
                a: spot(id, 0.0),
                b: spot(id, 1.0),
            },
        )
        .apply(&mut doc);
        let back = crate::project::from_json(&crate::project::to_json(&doc)).unwrap();
        assert_eq!(back.measurements, doc.measurements);
    }
}
