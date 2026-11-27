//! A persisted section plane (`tables/sections.json`, `.ymodel` minor 32).
//!
//! SketchUp sections cut the model with an infinite plane. This build models
//! **one** document-level plane: an origin, a unit normal and an enabled flag.
//! A section affects the model-space scene only; paper space is never clipped.
//!
//! The row is written only when a plane exists (enabled or paused), so an
//! ordinary document keeps its lower tier. Geometry outside the positive
//! half-space is removed by the scene builder; the cut face is not capped (the
//! cut boundary edges are drawn instead), which is an explicit boundary, not a
//! silent approximation.

use std::fmt;

use serde_json::{Map, Value};

use crate::{CadError, CadResult, Point3};

/// Stable refusal markers.
pub mod markers {
    /// The row or one of its values is malformed.
    pub const SECTION_INVALID: &str = "section.invalid";
    /// The write would not change the stored plane.
    pub const SECTION_NO_CHANGE: &str = "section.no_change";
    /// `SECTIONPLANE ON|TOGGLE` needs a stored plane first.
    pub const SECTION_PLANE_MISSING: &str = "section.plane_missing";
}

/// One document section plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionPlane {
    /// A point on the plane.
    pub origin: Point3,
    /// The unit normal; geometry with `dot(normal, p - origin) > 0` is cut away.
    pub normal: Point3,
    /// Whether the plane clips the model right now.
    pub active: bool,
}

impl SectionPlane {
    /// Build a plane, normalizing `normal` to unit length.
    ///
    /// A zero, non-finite or non-finite-origin input is a named refusal: the
    /// renderer must never receive a plane it cannot evaluate.
    pub fn new(origin: Point3, normal: Point3) -> CadResult<Self> {
        if !finite(origin) {
            return Err(invalid("origin must be finite"));
        }
        if !finite(normal) {
            return Err(invalid("normal must be finite"));
        }
        let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
        if !length.is_finite() || length < 1.0e-9 {
            return Err(invalid("normal must be non-zero"));
        }
        Ok(SectionPlane {
            origin,
            normal: Point3 {
                x: normal.x / length,
                y: normal.y / length,
                z: normal.z / length,
            },
            active: true,
        })
    }

    /// The plane with a different enabled flag.
    pub fn with_active(self, active: bool) -> Self {
        SectionPlane { active, ..self }
    }

    /// Validate the stored row (a decoded normal is normalized in place by
    /// [`Self::from_json`], so only finite values and a unit-ish normal remain).
    pub fn validate(&self) -> CadResult<()> {
        if !finite(self.origin) {
            return Err(invalid("origin must be finite"));
        }
        if !finite(self.normal) {
            return Err(invalid("normal must be finite"));
        }
        let length = (self.normal.x * self.normal.x
            + self.normal.y * self.normal.y
            + self.normal.z * self.normal.z)
            .sqrt();
        if !length.is_finite() || (length - 1.0).abs() > 1.0e-6 {
            return Err(invalid("normal must be a unit vector"));
        }
        Ok(())
    }

    /// Encode the one table row.
    pub fn to_json(&self) -> Value {
        let mut row = Map::new();
        row.insert("origin".into(), encode_point(self.origin));
        row.insert("normal".into(), encode_point(self.normal));
        row.insert("active".into(), Value::Bool(self.active));
        Value::Object(row)
    }

    /// Decode one row strictly; the normal is normalized before validation.
    pub fn from_json(value: &Value) -> CadResult<Self> {
        let Value::Object(row) = value else {
            return Err(invalid("row: expected a JSON object"));
        };
        let origin = decode_point(row.get("origin"), "origin")?;
        let normal = decode_point(row.get("normal"), "normal")?;
        let active = row
            .get("active")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("active: expected a boolean"))?;
        if let Some(key) = row
            .keys()
            .find(|key| !matches!(key.as_str(), "origin" | "normal" | "active"))
        {
            return Err(invalid(format!("unknown field {key:?}")));
        }
        let mut plane = SectionPlane::new(origin, normal)?;
        plane.active = active;
        plane.validate()?;
        Ok(plane)
    }
}

impl fmt::Display for SectionPlane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at ({:.3}, {:.3}, {:.3}), normal ({:.3}, {:.3}, {:.3})",
            if self.active { "ON" } else { "OFF" },
            self.origin.x,
            self.origin.y,
            self.origin.z,
            self.normal.x,
            self.normal.y,
            self.normal.z
        )
    }
}

fn finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn invalid(detail: impl Into<String>) -> CadError {
    CadError::InvalidInput(format!("{}: {}", markers::SECTION_INVALID, detail.into()))
}

fn encode_point(point: Point3) -> Value {
    Value::Array(vec![
        Value::from(point.x),
        Value::from(point.y),
        Value::from(point.z),
    ])
}

fn decode_point(value: Option<&Value>, key: &str) -> CadResult<Point3> {
    let Some(Value::Array(channels)) = value else {
        return Err(invalid(format!("{key}: expected three numbers")));
    };
    if channels.len() != 3 {
        return Err(invalid(format!("{key}: expected three numbers")));
    }
    let mut values = [0.0_f64; 3];
    for (index, channel) in channels.iter().enumerate() {
        values[index] = channel
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| invalid(format!("{key}[{index}]: expected a finite number")))?;
    }
    Ok(Point3 {
        x: values[0],
        y: values[1],
        z: values[2],
    })
}
