//! Persisted construction guides (`tables/guides.json`, `.ymodel` minor 26).
//!
//! A guide is a **non-entity** infinite construction line: it has no layer,
//! draw order or export representation, and it is not a pick target. The model
//! stores one point and a normalized direction; the renderer draws a bounded
//! segment derived from the drawing's own extents (this build has no infinite
//! line primitive in the render batch).
//!
//! Guides are shared by Tape Measure (`DIST GUIDE`) and the Protractor
//! (`PROTRACTOR GUIDE`). The table is capped at [`GUIDE_CAP`] rows; a new row at
//! the cap is refused by name instead of evicting one.

use std::fmt;

use serde_json::{Map, Value};

use crate::Point3;

/// How many guides one document retains.
pub const GUIDE_CAP: usize = 64;

/// Stable refusal markers.
pub mod markers {
    /// The guide table holds more rows than [`super::GUIDE_CAP`].
    pub const GUIDE_TABLE_TOO_LARGE: &str = "guide.table_too_large";
    /// A new guide would exceed [`super::GUIDE_CAP`].
    pub const GUIDE_CAP_EXCEEDED: &str = "guide.cap_exceeded";
    /// A point/direction is not finite or the direction is zero.
    pub const GUIDE_INVALID: &str = "guide.invalid";
}

/// One infinite construction line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuideLine {
    /// A point on the line.
    pub point: Point3,
    /// Unit direction (normalized by [`Self::new`] / the decoder).
    pub direction: Point3,
}

impl GuideLine {
    /// A guide through `point` toward `direction` (normalized).
    ///
    /// `None` for non-finite input or a zero direction; callers turn that into
    /// their own named refusal.
    pub fn new(point: Point3, direction: Point3) -> Option<Self> {
        if !finite(point) || !finite(direction) {
            return None;
        }
        let length = length(direction);
        if !(length.is_finite() && length > 1e-12) {
            return None;
        }
        Some(GuideLine {
            point,
            direction: Point3 {
                x: direction.x / length,
                y: direction.y / length,
                z: direction.z / length,
            },
        })
    }

    /// A guide through two distinct points.
    pub fn through(start: Point3, end: Point3) -> Option<Self> {
        Self::new(
            start,
            Point3 {
                x: end.x - start.x,
                y: end.y - start.y,
                z: end.z - start.z,
            },
        )
    }

    /// The row's validation point.
    pub fn validate(&self) -> Result<(), GuideError> {
        if !finite(self.point) || !finite(self.direction) {
            return Err(GuideError::Invalid(
                markers::GUIDE_INVALID,
                "a guide needs finite point and direction coordinates".into(),
            ));
        }
        let length = length(self.direction);
        if !(length.is_finite() && (length - 1.0).abs() <= 1e-9) {
            return Err(GuideError::Invalid(
                markers::GUIDE_INVALID,
                "a guide direction is stored normalized".into(),
            ));
        }
        Ok(())
    }

    /// Encode one row.
    pub fn to_json(&self) -> Value {
        let mut row = Map::new();
        row.insert("point".into(), encode_point(self.point));
        row.insert("direction".into(), encode_point(self.direction));
        Value::Object(row)
    }

    /// Decode one row strictly (unknown keys are refused).
    pub fn from_json(value: &Value) -> Result<Self, GuideError> {
        let row = value.as_object().ok_or_else(|| {
            GuideError::Invalid(markers::GUIDE_INVALID, "row: expected a JSON object".into())
        })?;
        if row.len() != 2 {
            return Err(GuideError::Invalid(
                markers::GUIDE_INVALID,
                "row: expected exactly `point` and `direction`".into(),
            ));
        }
        let point = decode_point(row.get("point"), "point")?;
        let direction = decode_point(row.get("direction"), "direction")?;
        let guide = GuideLine::new(point, direction).ok_or_else(|| {
            GuideError::Invalid(
                markers::GUIDE_INVALID,
                "direction: expected a finite, non-zero vector".into(),
            )
        })?;
        guide.validate()?;
        Ok(guide)
    }
}

/// One construction **point** guide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuidePoint {
    /// The marked world point.
    pub point: Point3,
}

impl GuidePoint {
    /// A point guide; `None` for non-finite coordinates.
    pub fn new(point: Point3) -> Option<Self> {
        finite(point).then_some(GuidePoint { point })
    }

    /// The row's validation point.
    pub fn validate(&self) -> Result<(), GuideError> {
        if !finite(self.point) {
            return Err(GuideError::Invalid(
                markers::GUIDE_INVALID,
                "a guide point needs finite coordinates".into(),
            ));
        }
        Ok(())
    }

    /// Encode one row (a point row has no `direction`).
    pub fn to_json(&self) -> Value {
        let mut row = Map::new();
        row.insert("point".into(), encode_point(self.point));
        Value::Object(row)
    }
}

/// One guide-table row: an infinite line or a point marker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    /// An infinite construction line.
    Line(GuideLine),
    /// A point marker.
    Point(GuidePoint),
}

impl Guide {
    /// The guide's anchor point.
    pub fn point(&self) -> Point3 {
        match self {
            Guide::Line(line) => line.point,
            Guide::Point(point) => point.point,
        }
    }

    /// Validate this row.
    pub fn validate(&self) -> Result<(), GuideError> {
        match self {
            Guide::Line(line) => line.validate(),
            Guide::Point(point) => point.validate(),
        }
    }

    /// Encode this row.
    pub fn to_json(&self) -> Value {
        match self {
            Guide::Line(line) => line.to_json(),
            Guide::Point(point) => point.to_json(),
        }
    }

    /// Decode one row: `direction` present = line, absent = point.
    pub fn from_json(value: &Value) -> Result<Self, GuideError> {
        let row = value.as_object().ok_or_else(|| {
            GuideError::Invalid(markers::GUIDE_INVALID, "row: expected a JSON object".into())
        })?;
        if row.contains_key("direction") {
            GuideLine::from_json(value).map(Guide::Line)
        } else if row.len() == 1 && row.contains_key("point") {
            let point = decode_point(row.get("point"), "point")?;
            GuidePoint::new(point).map(Guide::Point).ok_or_else(|| {
                GuideError::Invalid(
                    markers::GUIDE_INVALID,
                    "point: expected finite coordinates".into(),
                )
            })
        } else {
            Err(GuideError::Invalid(
                markers::GUIDE_INVALID,
                "row: expected `point` plus optional `direction`".into(),
            ))
        }
    }
}

/// One guide-table failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuideError {
    /// A stable marker plus a detail.
    Invalid(&'static str, String),
}

impl GuideError {
    /// The stable marker.
    pub fn marker(&self) -> &'static str {
        match self {
            GuideError::Invalid(marker, _) => marker,
        }
    }
}

impl fmt::Display for GuideError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuideError::Invalid(marker, detail) => write!(formatter, "{marker}: {detail}"),
        }
    }
}

impl std::error::Error for GuideError {}

/// A bounded guide table, oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuideTable {
    guides: Vec<Guide>,
}

impl GuideTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from decoded rows after validation.
    pub fn from_parts(guides: Vec<Guide>) -> Result<Self, GuideError> {
        let table = GuideTable { guides };
        table.validate()?;
        Ok(table)
    }

    /// The retained guides, oldest first.
    pub fn guides(&self) -> &[Guide] {
        &self.guides
    }

    /// How many guides are retained.
    pub fn len(&self) -> usize {
        self.guides.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.guides.is_empty()
    }

    /// Append one line guide; a new row at [`GUIDE_CAP`] is refused by name.
    pub fn push_line(&mut self, guide: GuideLine) -> Result<(), GuideError> {
        self.push(Guide::Line(guide))
    }

    /// Append one point guide.
    pub fn push_point(&mut self, guide: GuidePoint) -> Result<(), GuideError> {
        self.push(Guide::Point(guide))
    }

    /// Append one row.
    pub fn push(&mut self, guide: Guide) -> Result<(), GuideError> {
        guide.validate()?;
        if self.guides.len() >= GUIDE_CAP {
            return Err(GuideError::Invalid(
                markers::GUIDE_CAP_EXCEEDED,
                format!("the document holds at most {GUIDE_CAP} guides"),
            ));
        }
        self.guides.push(guide);
        Ok(())
    }

    /// Remove one row by 0-based position (the command layer exposes 1-based).
    pub fn remove(&mut self, index: usize) -> Option<Guide> {
        (index < self.guides.len()).then(|| self.guides.remove(index))
    }

    /// Remove every guide (one table write at the command layer).
    pub fn clear(&mut self) {
        self.guides.clear();
    }

    /// Validate the cap and every row.
    pub fn validate(&self) -> Result<(), GuideError> {
        if self.guides.len() > GUIDE_CAP {
            return Err(GuideError::Invalid(
                markers::GUIDE_TABLE_TOO_LARGE,
                format!(
                    "tables/guides.json holds {} rows, over the {GUIDE_CAP}-row budget",
                    self.guides.len()
                ),
            ));
        }
        for guide in &self.guides {
            guide.validate()?;
        }
        Ok(())
    }

    /// Encode every row.
    pub fn to_json(&self) -> Vec<Value> {
        self.guides.iter().map(Guide::to_json).collect()
    }

    /// Decode a row list.
    pub fn from_json(rows: &[Value]) -> Result<Self, GuideError> {
        if rows.len() > GUIDE_CAP {
            return Err(GuideError::Invalid(
                markers::GUIDE_TABLE_TOO_LARGE,
                format!(
                    "tables/guides.json holds {} rows, over the {GUIDE_CAP}-row budget",
                    rows.len()
                ),
            ));
        }
        let mut guides = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let guide = Guide::from_json(row).map_err(|error| {
                GuideError::Invalid(error.marker(), format!("row {index}: {error}"))
            })?;
            guides.push(guide);
        }
        Ok(GuideTable { guides })
    }
}

fn encode_point(point: Point3) -> Value {
    Value::Object(Map::from_iter([
        ("x".to_string(), Value::from(point.x)),
        ("y".to_string(), Value::from(point.y)),
        ("z".to_string(), Value::from(point.z)),
    ]))
}

fn finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn length(vector: Point3) -> f64 {
    vector.x.hypot(vector.y).hypot(vector.z)
}

fn decode_point(value: Option<&Value>, name: &str) -> Result<Point3, GuideError> {
    let value = value
        .ok_or_else(|| GuideError::Invalid(markers::GUIDE_INVALID, format!("{name}: missing")))?;
    let object = value.as_object().ok_or_else(|| {
        GuideError::Invalid(
            markers::GUIDE_INVALID,
            format!("{name}: expected an object"),
        )
    })?;
    if object.len() != 3 {
        return Err(GuideError::Invalid(
            markers::GUIDE_INVALID,
            format!("{name}: expected x, y and z"),
        ));
    }
    let component = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| {
                GuideError::Invalid(
                    markers::GUIDE_INVALID,
                    format!("{name}.{key}: expected a finite number"),
                )
            })
    };
    Ok(Point3 {
        x: component("x")?,
        y: component("y")?,
        z: component("z")?,
    })
}
