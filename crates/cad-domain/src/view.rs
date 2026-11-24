//! Persisted named views (`tables/views.json`, `.ymodel` minor 18).
//!
//! A named view is **document data** in AutoCAD (the `VIEW` table), so this
//! build stores it beside the drawing rather than in session state. The row
//! carries exactly what `VIEW SAVE` captures and `VIEW RESTORE` applies:
//!
//! * the viewport camera (eye/target/up and the projection),
//! * whether the viewport observed 2D or 3D, plus the 2D camera a switch back
//!   restores (the same payload [`crate`]-side `ViewMode2d3d` carries),
//! * the plan work plane,
//! * the active drawing space (model or one paper layout).
//!
//! Camera and projection types live in `cad-app` for rendering. This module is
//! the persistence DTO only: `cad-app` converts to and from its own types, so
//! `cad-db` never has to know a renderer camera and a saved document never
//! carries one.
//!
//! # Bounds and names
//!
//! [`NAMED_VIEW_CAP`] = **32** rows. Saving a 33rd is refused by name rather
//! than evicting an existing row, so the persisted table never loses a view the
//! user did not delete (undo would otherwise not be able to restore it). Names
//! are stored trimmed and resolved **exactly first, then by a unique ASCII
//! case-insensitive match**; saving an existing name replaces it and moves it to
//! the newest position. A name is never duplicated by [`NamedViewTable::save`].
//!
//! # Strictness
//!
//! The JSON codec refuses an unknown field, a missing field, an unknown enum
//! tag and a non-finite camera/work-plane value: a row this writer cannot
//! reproduce is a malformed document, not a row to skip.

use std::fmt;

use serde_json::{Map, Value};

use crate::ymodel::id::PersistentIdOf;
use crate::ymodel::json;
use crate::{CadError, CadResult, Point3, SpaceId, WorkPlane};

/// How many named views one document retains.
pub const NAMED_VIEW_CAP: usize = 32;

/// Longest accepted view name, in characters.
///
/// AutoCAD's symbol-table names are bounded; the command line already refuses an
/// empty name, and the table refuses an unbounded one so a persisted document
/// cannot carry an arbitrarily large name.
pub const MAX_NAMED_VIEW_NAME_CHARS: usize = 255;

/// A camera projection, persisted without the renderer type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewProjection {
    /// World units per logical pixel (the plan path).
    Orthographic { scale: f64 },
    /// Full vertical field of view, in radians.
    Perspective { vertical_fov_radians: f64 },
}

/// A camera, persisted without the renderer type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewCamera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Point3,
    pub projection: ViewProjection,
}

/// One saved named view.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedView {
    /// Stored name, trimmed and non-empty.
    pub name: String,
    /// The camera the viewport showed at save time.
    pub camera: ViewCamera,
    /// Whether the viewport was in the 3D observation mode.
    pub is_3d: bool,
    /// The 2D camera a switch back to plan restores.
    pub saved_2d_camera: ViewCamera,
    /// The plan work plane at save time.
    pub work_plane: WorkPlane,
    /// The active drawing space at save time.
    pub space: SpaceId,
}

/// One named-view table row failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedViewError {
    /// A stable marker plus a detail, ready for a `CadError`/snapshot error.
    Invalid(&'static str, String),
}

impl fmt::Display for NamedViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NamedViewError::Invalid(marker, detail) => write!(formatter, "{marker}: {detail}"),
        }
    }
}

impl std::error::Error for NamedViewError {}

/// Stable refusal markers.
pub mod markers {
    /// The name is empty, too long or contains a control character.
    pub const VIEW_NAME_INVALID: &str = "view.name_invalid";
    /// A camera or work-plane component is not finite / not usable.
    pub const VIEW_INVALID_CAMERA: &str = "view.invalid_camera";
    /// The projection scale or FOV is outside the accepted range.
    pub const VIEW_INVALID_PROJECTION: &str = "view.invalid_projection";
    /// The serialized table holds more rows than [`super::NAMED_VIEW_CAP`].
    pub const VIEW_TABLE_TOO_LARGE: &str = "view.table_too_large";
    /// Saving a new name would exceed [`super::NAMED_VIEW_CAP`].
    pub const VIEW_CAP_EXCEEDED: &str = "view.cap_exceeded";
    /// Two rows share a name (case-insensitively).
    pub const VIEW_DUPLICATE_NAME: &str = "view.duplicate_name";
}

impl NamedView {
    /// Check the row before it is written or applied.
    ///
    /// Every refusal names the row and the field; nothing is clamped and no
    /// default camera is invented.
    pub fn validate(&self) -> CadResult<()> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAMED_VIEW_NAME_CHARS {
            return Err(CadError::InvalidInput(format!(
                "{}: `{}` is not a usable view name",
                markers::VIEW_NAME_INVALID,
                self.name
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(CadError::InvalidInput(format!(
                "{}: `{name}` contains a control character",
                markers::VIEW_NAME_INVALID
            )));
        }
        validate_camera(&self.camera, &self.name)?;
        validate_camera(&self.saved_2d_camera, &self.name)?;
        validate_work_plane(&self.work_plane, &self.name)?;
        Ok(())
    }

    /// Encode one row.
    pub fn to_json(&self) -> Value {
        let mut row = Map::new();
        row.insert("name".to_string(), Value::String(self.name.clone()));
        row.insert("camera".to_string(), camera_to_json(&self.camera));
        row.insert(
            "mode".to_string(),
            Value::String(if self.is_3d { "3d" } else { "2d" }.to_string()),
        );
        row.insert(
            "saved_2d_camera".to_string(),
            camera_to_json(&self.saved_2d_camera),
        );
        row.insert(
            "work_plane".to_string(),
            work_plane_to_json(self.work_plane),
        );
        row.insert("space".to_string(), space_to_json(&self.space));
        Value::Object(row)
    }

    /// Decode one row strictly.
    pub fn from_json(value: &Value) -> Result<Self, NamedViewError> {
        let Value::Object(map) = value else {
            return Err(invalid("row", "expected a JSON object"));
        };
        let mut map = map.clone();
        let name = take_string(&mut map, "name")?;
        let camera = decode_camera(take(&mut map, "camera")?, "camera")?;
        let mode = take_string(&mut map, "mode")?;
        let is_3d = match mode.as_str() {
            "2d" => false,
            "3d" => true,
            other => {
                return Err(invalid(
                    "mode",
                    format!("expected \"2d\" or \"3d\", found {other:?}"),
                ))
            }
        };
        let saved_2d_camera = decode_camera(take(&mut map, "saved_2d_camera")?, "saved_2d_camera")?;
        let work_plane = decode_work_plane(take(&mut map, "work_plane")?)?;
        let space = decode_space(take(&mut map, "space")?)?;
        if let Some(key) = map.keys().min() {
            return Err(invalid(
                "row",
                format!("unknown field {key:?} (this writer cannot preserve it)"),
            ));
        }
        let view = NamedView {
            name,
            camera,
            is_3d,
            saved_2d_camera,
            work_plane,
            space,
        };
        view.validate().map_err(|error| match error {
            CadError::InvalidInput(detail) | CadError::Unsupported(detail) => {
                NamedViewError::Invalid(markers::VIEW_INVALID_CAMERA, detail)
            }
            other => NamedViewError::Invalid(markers::VIEW_INVALID_CAMERA, other.to_string()),
        })?;
        Ok(view)
    }
}

/// A bounded named-view table, newest last.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NamedViewTable {
    views: Vec<NamedView>,
}

impl NamedViewTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many views are retained.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The retained views in save order, oldest first.
    pub fn views(&self) -> impl Iterator<Item = &NamedView> + '_ {
        self.views.iter()
    }

    /// Resolve a name: exact match first, then a unique case-insensitive one.
    pub fn resolve(&self, name: &str) -> Option<&NamedView> {
        let name = name.trim();
        if let Some(found) = self.views.iter().find(|view| view.name == name) {
            return Some(found);
        }
        let mut matches = self
            .views
            .iter()
            .filter(|view| view.name.eq_ignore_ascii_case(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// Save (or replace, case-insensitively) one view and return whether an
    /// existing row was replaced.
    ///
    /// A replacement moves to the newest position. Adding a **new** name at
    /// [`NAMED_VIEW_CAP`] is [`markers::VIEW_CAP_EXCEEDED`] rather than an
    /// eviction, so a `VIEW SAVE` undo never has to resurrect a row the command
    /// silently dropped.
    pub fn save(&mut self, view: NamedView) -> Result<bool, NamedViewError> {
        let name = view.name.trim().to_string();
        if let Some(index) = self
            .views
            .iter()
            .position(|candidate| candidate.name.eq_ignore_ascii_case(&name))
        {
            self.views.remove(index);
            self.views.push(NamedView { name, ..view });
            return Ok(true);
        }
        if self.views.len() >= NAMED_VIEW_CAP {
            return Err(NamedViewError::Invalid(
                markers::VIEW_CAP_EXCEEDED,
                format!(
                    "`{name}` would be view {}, but the table holds at most {NAMED_VIEW_CAP} rows",
                    self.views.len() + 1
                ),
            ));
        }
        self.views.push(NamedView { name, ..view });
        Ok(false)
    }

    /// Remove one row, returning it when it existed.
    pub fn remove(&mut self, name: &str) -> Option<NamedView> {
        let name = name.trim();
        let index = self
            .views
            .iter()
            .position(|view| view.name.eq_ignore_ascii_case(name))?;
        Some(self.views.remove(index))
    }

    /// Encode every row, oldest first.
    pub fn to_json(&self) -> Vec<Value> {
        self.views.iter().map(NamedView::to_json).collect()
    }

    /// Decode a row table strictly (duplicates, cap and every row validated).
    pub fn from_json(rows: &[Value]) -> Result<Self, NamedViewError> {
        if rows.len() > NAMED_VIEW_CAP {
            return Err(NamedViewError::Invalid(
                markers::VIEW_TABLE_TOO_LARGE,
                format!(
                    "tables/views.json holds {} rows, over the {NAMED_VIEW_CAP}-row budget",
                    rows.len()
                ),
            ));
        }
        let mut table = NamedViewTable::new();
        for (index, row) in rows.iter().enumerate() {
            let view = NamedView::from_json(row)
                .map_err(|error| with_context(format!("row {index}"), error))?;
            if table
                .views
                .iter()
                .any(|candidate| candidate.name.eq_ignore_ascii_case(&view.name))
            {
                return Err(NamedViewError::Invalid(
                    markers::VIEW_DUPLICATE_NAME,
                    format!(
                        "row {index}: the name `{}` is stored more than once",
                        view.name
                    ),
                ));
            }
            table.views.push(view);
        }
        Ok(table)
    }
}

fn with_context(context: String, error: NamedViewError) -> NamedViewError {
    match error {
        NamedViewError::Invalid(marker, detail) => {
            NamedViewError::Invalid(marker, format!("{context}: {detail}"))
        }
    }
}

fn invalid(field: &str, detail: impl fmt::Display) -> NamedViewError {
    NamedViewError::Invalid(markers::VIEW_INVALID_CAMERA, format!("{field}: {detail}"))
}

fn take(map: &mut Map<String, Value>, key: &str) -> Result<Value, NamedViewError> {
    map.remove(key)
        .ok_or_else(|| invalid(key, "required field is missing"))
}

fn take_string(map: &mut Map<String, Value>, key: &str) -> Result<String, NamedViewError> {
    match take(map, key)? {
        Value::String(value) => Ok(value),
        _ => Err(invalid(key, "expected a string")),
    }
}

fn take_number(map: &mut Map<String, Value>, key: &str) -> Result<f64, NamedViewError> {
    json::decode_f64(&take(map, key)?).map_err(|error| invalid(key, error))
}

fn take_point(map: &mut Map<String, Value>, key: &str) -> Result<Point3, NamedViewError> {
    json::decode_point3(&take(map, key)?).map_err(|error| invalid(key, error))
}

fn validate_camera(camera: &ViewCamera, name: &str) -> CadResult<()> {
    for (label, point) in [
        ("eye", camera.eye),
        ("target", camera.target),
        ("up", camera.up),
    ] {
        for (axis, value) in [("x", point.x), ("y", point.y), ("z", point.z)] {
            if !value.is_finite() {
                return Err(CadError::InvalidInput(format!(
                    "{}: view `{name}` camera {label} has the non-finite {axis} {value}",
                    markers::VIEW_INVALID_CAMERA
                )));
            }
        }
    }
    let up_length =
        (camera.up.x * camera.up.x + camera.up.y * camera.up.y + camera.up.z * camera.up.z).sqrt();
    if up_length <= 1e-12 {
        return Err(CadError::InvalidInput(format!(
            "{}: view `{name}` camera up hint is the zero vector",
            markers::VIEW_INVALID_CAMERA
        )));
    }
    match camera.projection {
        ViewProjection::Orthographic { scale } => {
            if !scale.is_finite() || scale <= 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "{}: view `{name}` orthographic scale must be finite and positive",
                    markers::VIEW_INVALID_PROJECTION
                )));
            }
        }
        ViewProjection::Perspective {
            vertical_fov_radians,
        } => {
            if !vertical_fov_radians.is_finite()
                || !(0.0..std::f64::consts::PI).contains(&vertical_fov_radians)
                || vertical_fov_radians <= 0.0
            {
                return Err(CadError::InvalidInput(format!(
                    "{}: view `{name}` perspective FOV must be finite and in (0, pi)",
                    markers::VIEW_INVALID_PROJECTION
                )));
            }
        }
    }
    Ok(())
}

fn validate_work_plane(plane: &WorkPlane, name: &str) -> CadResult<()> {
    for (label, point) in [("origin", plane.origin), ("u", plane.u), ("v", plane.v)] {
        for (axis, value) in [("x", point.x), ("y", point.y), ("z", point.z)] {
            if !value.is_finite() {
                return Err(CadError::InvalidInput(format!(
                    "{}: view `{name}` work plane {label} has the non-finite {axis} {value}",
                    markers::VIEW_INVALID_CAMERA
                )));
            }
        }
    }
    Ok(())
}

fn camera_to_json(camera: &ViewCamera) -> Value {
    let mut map = Map::new();
    map.insert("eye".to_string(), json::encode_point3(camera.eye));
    map.insert("target".to_string(), json::encode_point3(camera.target));
    map.insert("up".to_string(), json::encode_point3(camera.up));
    let mut projection = Map::new();
    match camera.projection {
        ViewProjection::Orthographic { scale } => {
            projection.insert(
                "kind".to_string(),
                Value::String("orthographic".to_string()),
            );
            projection.insert("scale".to_string(), json::encode_f64(scale));
        }
        ViewProjection::Perspective {
            vertical_fov_radians,
        } => {
            projection.insert("kind".to_string(), Value::String("perspective".to_string()));
            projection.insert(
                "vertical_fov".to_string(),
                json::encode_f64(vertical_fov_radians),
            );
        }
    }
    map.insert("projection".to_string(), Value::Object(projection));
    Value::Object(map)
}

fn decode_camera(value: Value, field: &str) -> Result<ViewCamera, NamedViewError> {
    let Value::Object(map) = value else {
        return Err(invalid(field, "expected a JSON object"));
    };
    let mut map = map;
    let eye = take_point(&mut map, "eye")?;
    let target = take_point(&mut map, "target")?;
    let up = take_point(&mut map, "up")?;
    let Value::Object(mut projection_map) = take(&mut map, "projection")? else {
        return Err(invalid("projection", "expected a JSON object"));
    };
    let kind = take_string(&mut projection_map, "kind")?;
    let projection = match kind.as_str() {
        "orthographic" => ViewProjection::Orthographic {
            scale: take_number(&mut projection_map, "scale")?,
        },
        "perspective" => ViewProjection::Perspective {
            vertical_fov_radians: take_number(&mut projection_map, "vertical_fov")?,
        },
        other => {
            return Err(invalid(
                "projection.kind",
                format!("unknown projection {other:?}"),
            ))
        }
    };
    if let Some(key) = projection_map.keys().min() {
        return Err(invalid(
            "projection",
            format!("unknown field {key:?} (this writer cannot preserve it)"),
        ));
    }
    if let Some(key) = map.keys().min() {
        return Err(invalid(
            field,
            format!("unknown field {key:?} (this writer cannot preserve it)"),
        ));
    }
    Ok(ViewCamera {
        eye,
        target,
        up,
        projection,
    })
}

fn work_plane_to_json(plane: WorkPlane) -> Value {
    let mut map = Map::new();
    map.insert("origin".to_string(), json::encode_point3(plane.origin));
    map.insert("u".to_string(), json::encode_point3(plane.u));
    map.insert("v".to_string(), json::encode_point3(plane.v));
    Value::Object(map)
}

fn decode_work_plane(value: Value) -> Result<WorkPlane, NamedViewError> {
    let Value::Object(mut map) = value else {
        return Err(invalid("work_plane", "expected a JSON object"));
    };
    let origin = take_point(&mut map, "origin")?;
    let u = take_point(&mut map, "u")?;
    let v = take_point(&mut map, "v")?;
    if let Some(key) = map.keys().min() {
        return Err(invalid(
            "work_plane",
            format!("unknown field {key:?} (this writer cannot preserve it)"),
        ));
    }
    Ok(WorkPlane { origin, u, v })
}

fn space_to_json(space: &SpaceId) -> Value {
    let mut map = Map::new();
    match space {
        SpaceId::Model => {
            map.insert("kind".to_string(), Value::String("model".to_string()));
        }
        SpaceId::Paper(layout) => {
            map.insert("kind".to_string(), Value::String("paper".to_string()));
            map.insert("layout".to_string(), Value::from(layout.seq() as u64));
        }
        SpaceId::Block(block) => {
            map.insert("kind".to_string(), Value::String("block".to_string()));
            map.insert("block".to_string(), Value::from(block.seq() as u64));
        }
    }
    Value::Object(map)
}

fn decode_space(value: Value) -> Result<SpaceId, NamedViewError> {
    use crate::{BlockId, LayoutId};
    let Value::Object(mut map) = value else {
        return Err(invalid("space", "expected a JSON object"));
    };
    let kind = take_string(&mut map, "kind")?;
    let space = match kind.as_str() {
        "model" => SpaceId::Model,
        "paper" => {
            let raw = take_u64(&mut map, "layout")?;
            SpaceId::Paper(LayoutId::from_seq(raw))
        }
        "block" => {
            let raw = take_u64(&mut map, "block")?;
            SpaceId::Block(BlockId::from_seq(raw))
        }
        other => return Err(invalid("space.kind", format!("unknown space {other:?}"))),
    };
    if let Some(key) = map.keys().min() {
        return Err(invalid(
            "space",
            format!("unknown field {key:?} (this writer cannot preserve it)"),
        ));
    }
    Ok(space)
}

fn take_u64(map: &mut Map<String, Value>, key: &str) -> Result<u128, NamedViewError> {
    let raw = take(map, key)?
        .as_u64()
        .map(u128::from)
        .ok_or_else(|| invalid(key, "expected an unsigned integer"))?;
    if raw > crate::ymodel::id::MAX_ID as u128 {
        return Err(invalid(
            key,
            format!("{raw} is above the persisted identifier ceiling"),
        ));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(scale: f64) -> ViewCamera {
        ViewCamera {
            eye: Point3 {
                x: 0.0,
                y: 0.0,
                z: 10.0,
            },
            target: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            projection: ViewProjection::Orthographic { scale },
        }
    }

    fn view(name: &str) -> NamedView {
        NamedView {
            name: name.to_string(),
            camera: camera(1.5),
            is_3d: false,
            saved_2d_camera: camera(1.5),
            work_plane: WorkPlane {
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                u: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                v: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
            space: SpaceId::Model,
        }
    }

    #[test]
    fn a_row_round_trips_through_json() {
        let mut row = view("Plan A");
        row.is_3d = true;
        row.space = SpaceId::Paper(crate::LayoutId(3));
        let json = row.to_json();
        assert_eq!(NamedView::from_json(&json).unwrap(), row);
    }

    #[test]
    fn the_table_preserves_order_and_resolves_names_case_insensitively() {
        let mut table = NamedViewTable::new();
        assert!(!table.save(view("First")).unwrap());
        assert!(!table.save(view("Second")).unwrap());
        assert!(table.save(view("second")).unwrap());
        let names: Vec<&str> = table.views().map(|view| view.name.as_str()).collect();
        assert_eq!(names, vec!["First", "second"]);
        assert_eq!(table.resolve("SECOND").unwrap().name, "second");
        assert!(table.remove("FIRST").is_some());
        assert!(table.remove("first").is_none());
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn the_table_refuses_a_new_name_at_the_cap_without_evicting() {
        let mut table = NamedViewTable::new();
        for index in 0..NAMED_VIEW_CAP {
            table.save(view(&format!("V{index}"))).unwrap();
        }
        assert_eq!(table.len(), NAMED_VIEW_CAP);
        assert!(matches!(
            table.save(view("NEW")),
            Err(NamedViewError::Invalid(markers::VIEW_CAP_EXCEEDED, _))
        ));
        assert_eq!(table.len(), NAMED_VIEW_CAP);
        assert!(table.resolve("V0").is_some(), "no existing row was evicted");
        assert!(table.resolve("NEW").is_none());
        // Replacing an existing name still works at the cap.
        assert!(table.save(view("V0")).unwrap());
    }

    #[test]
    fn decoding_refuses_duplicates_unknown_fields_and_an_oversized_table() {
        let rows = vec![view("A").to_json(), view("a").to_json()];
        assert_eq!(
            NamedViewTable::from_json(&rows).unwrap_err(),
            NamedViewError::Invalid(
                markers::VIEW_DUPLICATE_NAME,
                "row 1: the name `a` is stored more than once".into()
            )
        );

        let mut json = view("A").to_json();
        json["extra"] = Value::from(1);
        assert!(NamedViewTable::from_json(&[json]).is_err());

        let too_many: Vec<Value> = (0..=NAMED_VIEW_CAP)
            .map(|index| view(&format!("V{index}")).to_json())
            .collect();
        assert!(matches!(
            NamedViewTable::from_json(&too_many).unwrap_err(),
            NamedViewError::Invalid(markers::VIEW_TABLE_TOO_LARGE, _)
        ));
    }

    #[test]
    fn a_non_finite_or_degenerate_camera_is_refused() {
        let mut row = view("Bad");
        row.camera.eye.x = f64::NAN;
        assert!(row.validate().is_err());

        let mut row = view("Bad");
        row.camera.up = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(row.validate().is_err());

        let mut row = view("Bad");
        row.camera.projection = ViewProjection::Orthographic { scale: 0.0 };
        assert!(row.validate().is_err());
    }
}
