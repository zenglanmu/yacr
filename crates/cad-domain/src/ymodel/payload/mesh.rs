//! Mesh and 3D-body payload classes (ADR-0037 D4).
//!
//! Covers `cad:Mesh` and `cad:Body3d`, plus the nested `Mesh`, `SubElementId`,
//! `BodyKind`, `BodyExact`, `BodyDisplay`, `Precision`, `Completeness` and the
//! whole neutral B-rep tree ([`crate::brep`]).
//!
//! Two format rules matter here:
//!
//! * **Nothing is re-derived.** A body's `display` is the stored tessellation
//!   cache; the payload reader rebuilds the value exactly as written, because
//!   the kernel seam is not reachable from `cad-domain` and a re-tessellation
//!   would change the file on the next save. Likewise `BrepData` is the exact
//!   geometry as stored: a placement is never baked in, a tolerance is never
//!   invented.
//! * **Missing geometry is a state, not an absence.** `BodyExact::Unavailable`
//!   carries its reason; a `BrepSurface::Unsupported`/`BrepCurve::Unsupported`
//!   carries its type key. Both round-trip as data rather than becoming `null`
//!   or an empty list.

use serde_json::{Map, Value};

use super::{fields, object, PayloadError};
use crate::brep::{BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface};
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    Body3d, BodyDisplay, BodyExact, BodyKind, Completeness, Mesh, Point3, Precision,
    SemanticGeometry, SubElementId,
};

// ---------------------------------------------------------------------------
// Mesh.
// ---------------------------------------------------------------------------

pub(super) fn encode_mesh(mesh: &Mesh) -> Value {
    object(
        "cad:Mesh",
        vec![
            ("vertices", super::basic::encode_vec_point3(&mesh.vertices)),
            (
                "triangles",
                Value::Array(
                    mesh.triangles
                        .iter()
                        .map(|triangle| {
                            Value::Array(triangle.iter().map(|index| Value::from(*index)).collect())
                        })
                        .collect(),
                ),
            ),
            ("normals", super::basic::encode_vec_point3(&mesh.normals)),
            (
                "face_sources",
                Value::Array(
                    mesh.face_sources
                        .iter()
                        .map(|source| source.as_ref().map_or(Value::Null, encode_sub_element))
                        .collect(),
                ),
            ),
            (
                "colors",
                Value::Array(
                    mesh.colors
                        .iter()
                        .map(|color| {
                            Value::Array(color.iter().map(|byte| Value::from(*byte)).collect())
                        })
                        .collect(),
                ),
            ),
        ],
    )
}

pub(super) fn decode_mesh(value: &Value) -> Result<Mesh, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Mesh")?;
    let vertices = reader.vec_point3("vertices")?;
    let triangles = reader.vec_u32x3("triangles")?;
    let normals = reader.vec_point3("normals")?;
    let raw_sources = reader.take("face_sources")?;
    let face_sources = match &raw_sources {
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => Ok(None),
                value => decode_sub_element(value).map(Some),
            })
            .collect::<Result<Vec<_>, PayloadError>>()?,
        _ => {
            return Err(PayloadError::new(
                "cad:Mesh.face_sources: expected an array of records",
            ));
        }
    };
    let colors = reader.u8x3_list("colors")?;
    reader.finish()?;
    Ok(Mesh {
        vertices,
        triangles,
        normals,
        face_sources,
        colors,
    })
}

fn encode_sub_element(source: &SubElementId) -> Value {
    let mut map = Map::new();
    map.insert(
        "source_key".to_string(),
        Value::String(source.source_key.clone()),
    );
    map.insert(
        "topology_revision".to_string(),
        Value::from(source.topology_revision.0),
    );
    Value::Object(map)
}

fn decode_sub_element(value: &Value) -> Result<SubElementId, PayloadError> {
    let mut reader = ObjectReader::new(value, "sub_element")?;
    let source_key = reader.string("source_key")?;
    let topology_revision = reader.u64("topology_revision")?;
    reader.finish()?;
    Ok(SubElementId {
        source_key,
        topology_revision: crate::Revision(topology_revision),
    })
}

// ---------------------------------------------------------------------------
// Body3d and its parts.
// ---------------------------------------------------------------------------

pub(super) fn encode_body(body: &Body3d) -> Value {
    object(
        "cad:Body3d",
        vec![
            ("kind", encode_body_kind(&body.kind)),
            ("exact", encode_body_exact(&body.exact)),
            ("display", encode_body_display(&body.display)),
        ],
    )
}

pub(super) fn decode_body(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Body3d")?;
    let kind = decode_body_kind(&reader.take("kind")?)?;
    let exact = decode_body_exact(&reader.take("exact")?)?;
    let display = decode_body_display(&reader.take("display")?)?;
    reader.finish()?;
    Ok(SemanticGeometry::Body3d(Box::new(Body3d::from_persisted(
        kind, exact, display,
    ))))
}

fn encode_body_kind(kind: &BodyKind) -> Value {
    Value::String(
        match kind {
            BodyKind::Wire => "wire",
            BodyKind::Sheet => "sheet",
            BodyKind::Solid => "solid",
            BodyKind::Unknown => "unknown",
        }
        .to_string(),
    )
}

fn decode_body_kind(value: &Value) -> Result<BodyKind, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "wire" => Ok(BodyKind::Wire),
            "sheet" => Ok(BodyKind::Sheet),
            "solid" => Ok(BodyKind::Solid),
            "unknown" => Ok(BodyKind::Unknown),
            other => Err(PayloadError::new(format!(
                "cad:Body3d.kind: unknown body kind {other:?}"
            ))),
        },
        _ => Err(PayloadError::new(
            "cad:Body3d.kind: expected a string token",
        )),
    }
}

fn encode_body_exact(exact: &BodyExact) -> Value {
    match exact {
        BodyExact::Exact(brep) => fields(vec![
            ("state", Value::String("exact".to_string())),
            ("brep", encode_brep(brep)),
        ]),
        BodyExact::Unavailable { reason } => fields(vec![
            ("state", Value::String("unavailable".to_string())),
            ("reason", Value::String(reason.clone())),
        ]),
    }
}

fn decode_body_exact(value: &Value) -> Result<BodyExact, PayloadError> {
    let mut reader = ObjectReader::new(value, "cad:Body3d.exact")?;
    let state = reader.string("state")?;
    let exact = match state.as_str() {
        "exact" => {
            let brep = decode_brep(&reader.take("brep")?)?;
            reader.finish()?;
            BodyExact::Exact(Box::new(brep))
        }
        "unavailable" => {
            let reason = reader.string("reason")?;
            reader.finish()?;
            BodyExact::Unavailable { reason }
        }
        other => {
            return Err(PayloadError::new(format!(
                "cad:Body3d.exact.state: unknown state {other:?}"
            )));
        }
    };
    Ok(exact)
}

fn encode_body_display(display: &BodyDisplay) -> Value {
    fields(vec![
        ("mesh", encode_mesh(&display.mesh)),
        (
            "edges",
            Value::Array(
                display
                    .edges
                    .iter()
                    .map(|edge| super::basic::encode_vec_point3(edge))
                    .collect(),
            ),
        ),
        ("tolerance", json::encode_opt_f64(display.tolerance)),
        ("precision", encode_precision(&display.precision)),
        ("completeness", encode_completeness(&display.completeness)),
    ])
}

fn decode_body_display(value: &Value) -> Result<BodyDisplay, PayloadError> {
    let mut reader = ObjectReader::new(value, "cad:Body3d.display")?;
    let mesh = decode_mesh(&reader.take("mesh")?)?;
    let raw_edges = reader.take("edges")?;
    let edges = match &raw_edges {
        Value::Array(items) => items
            .iter()
            .map(decode_points)
            .collect::<Result<Vec<_>, PayloadError>>()?,
        _ => {
            return Err(PayloadError::new(
                "cad:Body3d.display.edges: expected an array of point arrays",
            ));
        }
    };
    let tolerance = reader.opt_f64("tolerance")?;
    let precision = decode_precision(&reader.take("precision")?)?;
    let completeness = decode_completeness(&reader.take("completeness")?)?;
    reader.finish()?;
    Ok(BodyDisplay {
        mesh,
        edges,
        tolerance,
        precision,
        completeness,
    })
}

/// Decode a bare array of points (a body-display edge).
fn decode_points(value: &Value) -> Result<Vec<Point3>, PayloadError> {
    let Value::Array(items) = value else {
        return Err(PayloadError::new("edge: expected an array of points"));
    };
    items
        .iter()
        .map(|item| json::decode_point3(item).map_err(PayloadError::from))
        .collect()
}

fn encode_precision(precision: &Precision) -> Value {
    match precision {
        Precision::Analytic => fields(vec![("kind", Value::String("analytic".to_string()))]),
        Precision::Approximate { error_bound } => fields(vec![
            ("kind", Value::String("approximate".to_string())),
            ("error_bound", json::encode_opt_f64(*error_bound)),
        ]),
        Precision::Unknown => fields(vec![("kind", Value::String("unknown".to_string()))]),
    }
}

fn decode_precision(value: &Value) -> Result<Precision, PayloadError> {
    let mut reader = ObjectReader::new(value, "precision")?;
    let kind = reader.string("kind")?;
    let precision = match kind.as_str() {
        "analytic" => Precision::Analytic,
        "approximate" => {
            let error_bound = reader.opt_f64("error_bound")?;
            Precision::Approximate { error_bound }
        }
        "unknown" => Precision::Unknown,
        other => {
            return Err(PayloadError::new(format!(
                "precision.kind: unknown precision {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(precision)
}

fn encode_completeness(completeness: &Completeness) -> Value {
    let (kind, reasons) = match completeness {
        Completeness::Complete => ("complete", None),
        Completeness::Partial(reasons) => ("partial", Some(reasons)),
        Completeness::Missing(reasons) => ("missing", Some(reasons)),
        Completeness::Unverified => ("unverified", None),
    };
    let mut entries = vec![("kind", Value::String(kind.to_string()))];
    if let Some(reasons) = reasons {
        entries.push((
            "reasons",
            Value::Array(
                reasons
                    .iter()
                    .map(|reason| Value::String(reason.clone()))
                    .collect(),
            ),
        ));
    }
    fields(entries)
}

fn decode_completeness(value: &Value) -> Result<Completeness, PayloadError> {
    let mut reader = ObjectReader::new(value, "completeness")?;
    let kind = reader.string("kind")?;
    let completeness = match kind.as_str() {
        "complete" => Completeness::Complete,
        "partial" => Completeness::Partial(reader.vec_string("reasons")?),
        "missing" => Completeness::Missing(reader.vec_string("reasons")?),
        "unverified" => Completeness::Unverified,
        other => {
            return Err(PayloadError::new(format!(
                "completeness.kind: unknown completeness {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(completeness)
}

// ---------------------------------------------------------------------------
// The neutral B-rep.
// ---------------------------------------------------------------------------

fn encode_brep(brep: &BrepData) -> Value {
    let mut map = Map::new();
    map.insert(
        "shells".to_string(),
        Value::Array(brep.shells.iter().map(encode_brep_shell).collect()),
    );
    map.insert(
        "placement".to_string(),
        brep.placement
            .as_ref()
            .map_or(Value::Null, encode_brep_placement),
    );
    map.insert(
        "tolerance".to_string(),
        json::encode_opt_f64(brep.tolerance),
    );
    Value::Object(map)
}

fn decode_brep(value: &Value) -> Result<BrepData, PayloadError> {
    let mut reader = ObjectReader::new(value, "brep")?;
    let raw_shells = reader.take("shells")?;
    let shells = match &raw_shells {
        Value::Array(items) => items
            .iter()
            .map(decode_brep_shell)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("brep.shells: expected an array"));
        }
    };
    let placement = match reader.take("placement")? {
        Value::Null => None,
        value => Some(decode_brep_placement(&value)?),
    };
    let tolerance = reader.opt_f64("tolerance")?;
    reader.finish()?;
    Ok(BrepData {
        shells,
        placement,
        tolerance,
    })
}

fn encode_brep_placement(placement: &BrepPlacement) -> Value {
    let mut map = Map::new();
    map.insert(
        "matrix".to_string(),
        encode_f64_matrix(
            &placement
                .matrix
                .iter()
                .map(|row| &row[..])
                .collect::<Vec<_>>(),
        ),
    );
    map.insert(
        "translation".to_string(),
        json::encode_point3(placement.translation),
    );
    map.insert("scale".to_string(), json::encode_f64(placement.scale));
    Value::Object(map)
}

fn decode_brep_placement(value: &Value) -> Result<BrepPlacement, PayloadError> {
    let mut reader = ObjectReader::new(value, "placement")?;
    let rows = decode_f64_matrix(&reader.take("matrix")?, 3, 3, "placement.matrix")?;
    let translation = reader.point3("translation")?;
    let scale = reader.f64("scale")?;
    reader.finish()?;
    let mut matrix = [[0.0f64; 3]; 3];
    for (row_index, row) in rows.iter().enumerate() {
        for (column, cell) in row.iter().enumerate() {
            matrix[row_index][column] = *cell;
        }
    }
    Ok(BrepPlacement {
        matrix,
        translation,
        scale,
    })
}

fn encode_brep_shell(shell: &BrepShell) -> Value {
    fields(vec![
        ("id", Value::from(shell.id)),
        (
            "faces",
            Value::Array(shell.faces.iter().map(encode_brep_face).collect()),
        ),
    ])
}

fn decode_brep_shell(value: &Value) -> Result<BrepShell, PayloadError> {
    let mut reader = ObjectReader::new(value, "shell")?;
    let id = reader.u32("id")?;
    let raw_faces = reader.take("faces")?;
    let faces = match &raw_faces {
        Value::Array(items) => items
            .iter()
            .map(decode_brep_face)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("shell.faces: expected an array"));
        }
    };
    reader.finish()?;
    Ok(BrepShell { id, faces })
}

fn encode_brep_face(face: &BrepFace) -> Value {
    let mut map = Map::new();
    map.insert("id".to_string(), Value::from(face.id));
    map.insert("surface".to_string(), encode_brep_surface(&face.surface));
    map.insert("reversed".to_string(), Value::Bool(face.reversed));
    map.insert(
        "loops".to_string(),
        Value::Array(face.loops.iter().map(encode_brep_loop).collect()),
    );
    map.insert(
        "domain".to_string(),
        face.domain.map_or(Value::Null, |domain| {
            Value::Array(vec![
                json::encode_f64x2(domain[0]),
                json::encode_f64x2(domain[1]),
            ])
        }),
    );
    map.insert(
        "tolerance".to_string(),
        json::encode_opt_f64(face.tolerance),
    );
    Value::Object(map)
}

fn decode_brep_face(value: &Value) -> Result<BrepFace, PayloadError> {
    let mut reader = ObjectReader::new(value, "face")?;
    let id = reader.u32("id")?;
    let surface = decode_brep_surface(&reader.take("surface")?)?;
    let reversed = reader.bool("reversed")?;
    let raw_loops = reader.take("loops")?;
    let loops = match &raw_loops {
        Value::Array(items) => items
            .iter()
            .map(decode_brep_loop)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("face.loops: expected an array"));
        }
    };
    let domain = match reader.take("domain")? {
        Value::Null => None,
        value => {
            let Value::Array(bounds) = &value else {
                return Err(PayloadError::new(
                    "face.domain: expected null or two parameter pairs",
                ));
            };
            if bounds.len() != 2 {
                return Err(PayloadError::new(
                    "face.domain: expected null or two parameter pairs",
                ));
            }
            Some([
                json::decode_f64x2(&bounds[0]).map_err(PayloadError::from)?,
                json::decode_f64x2(&bounds[1]).map_err(PayloadError::from)?,
            ])
        }
    };
    let tolerance = reader.opt_f64("tolerance")?;
    reader.finish()?;
    Ok(BrepFace {
        id,
        surface,
        reversed,
        loops,
        domain,
        tolerance,
    })
}

fn encode_brep_loop(loop_: &BrepLoop) -> Value {
    fields(vec![(
        "edges",
        Value::Array(loop_.edges.iter().map(encode_brep_curve).collect()),
    )])
}

fn decode_brep_loop(value: &Value) -> Result<BrepLoop, PayloadError> {
    let mut reader = ObjectReader::new(value, "loop")?;
    let raw_edges = reader.take("edges")?;
    let edges = match &raw_edges {
        Value::Array(items) => items
            .iter()
            .map(decode_brep_curve)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("loop.edges: expected an array"));
        }
    };
    reader.finish()?;
    Ok(BrepLoop { edges })
}

fn encode_brep_surface(surface: &BrepSurface) -> Value {
    match surface {
        BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        } => fields(vec![
            ("kind", Value::String("plane".to_string())),
            ("origin", json::encode_point3(*origin)),
            ("normal", json::encode_point3(*normal)),
            ("u_dir", json::encode_point3(*u_dir)),
        ]),
        BrepSurface::Sphere {
            center,
            radius,
            u_dir,
            pole,
        } => fields(vec![
            ("kind", Value::String("sphere".to_string())),
            ("center", json::encode_point3(*center)),
            ("radius", json::encode_f64(*radius)),
            ("u_dir", json::encode_point3(*u_dir)),
            ("pole", json::encode_point3(*pole)),
        ]),
        BrepSurface::Torus {
            center,
            normal,
            major_radius,
            minor_radius,
            u_dir,
        } => fields(vec![
            ("kind", Value::String("torus".to_string())),
            ("center", json::encode_point3(*center)),
            ("normal", json::encode_point3(*normal)),
            ("major_radius", json::encode_f64(*major_radius)),
            ("minor_radius", json::encode_f64(*minor_radius)),
            ("u_dir", json::encode_point3(*u_dir)),
        ]),
        BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } => fields(vec![
            ("kind", Value::String("cylinder".to_string())),
            ("origin", json::encode_point3(*origin)),
            ("axis", json::encode_point3(*axis)),
            ("ref_dir", json::encode_point3(*ref_dir)),
            ("radius", json::encode_f64(*radius)),
        ]),
        BrepSurface::Cone {
            origin,
            axis,
            ref_dir,
            radius,
            sin_half_angle,
            cos_half_angle,
        } => fields(vec![
            ("kind", Value::String("cone".to_string())),
            ("origin", json::encode_point3(*origin)),
            ("axis", json::encode_point3(*axis)),
            ("ref_dir", json::encode_point3(*ref_dir)),
            ("radius", json::encode_f64(*radius)),
            ("sin_half_angle", json::encode_f64(*sin_half_angle)),
            ("cos_half_angle", json::encode_f64(*cos_half_angle)),
        ]),
        BrepSurface::Nurbs {
            u_knots,
            v_knots,
            points,
        } => fields(vec![
            ("kind", Value::String("nurbs".to_string())),
            ("u_knots", super::basic::encode_vec_f64(u_knots)),
            ("v_knots", super::basic::encode_vec_f64(v_knots)),
            (
                "points",
                Value::Array(
                    points
                        .iter()
                        .map(|row| {
                            Value::Array(
                                row.iter()
                                    .map(|point| {
                                        Value::Array(
                                            point
                                                .iter()
                                                .map(|value| json::encode_f64(*value))
                                                .collect(),
                                        )
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                ),
            ),
        ]),
        BrepSurface::Unsupported { type_key } => fields(vec![
            ("kind", Value::String("unsupported".to_string())),
            ("type_key", Value::String(type_key.clone())),
        ]),
    }
}

fn decode_brep_surface(value: &Value) -> Result<BrepSurface, PayloadError> {
    let mut reader = ObjectReader::new(value, "surface")?;
    let kind = reader.string("kind")?;
    let surface = match kind.as_str() {
        "plane" => BrepSurface::Plane {
            origin: reader.point3("origin")?,
            normal: reader.point3("normal")?,
            u_dir: reader.point3("u_dir")?,
        },
        "sphere" => BrepSurface::Sphere {
            center: reader.point3("center")?,
            radius: reader.f64("radius")?,
            u_dir: reader.point3("u_dir")?,
            pole: reader.point3("pole")?,
        },
        "torus" => BrepSurface::Torus {
            center: reader.point3("center")?,
            normal: reader.point3("normal")?,
            major_radius: reader.f64("major_radius")?,
            minor_radius: reader.f64("minor_radius")?,
            u_dir: reader.point3("u_dir")?,
        },
        "cylinder" => BrepSurface::Cylinder {
            origin: reader.point3("origin")?,
            axis: reader.point3("axis")?,
            ref_dir: reader.point3("ref_dir")?,
            radius: reader.f64("radius")?,
        },
        "cone" => BrepSurface::Cone {
            origin: reader.point3("origin")?,
            axis: reader.point3("axis")?,
            ref_dir: reader.point3("ref_dir")?,
            radius: reader.f64("radius")?,
            sin_half_angle: reader.f64("sin_half_angle")?,
            cos_half_angle: reader.f64("cos_half_angle")?,
        },
        "nurbs" => {
            let u_knots = reader.vec_f64("u_knots")?;
            let v_knots = reader.vec_f64("v_knots")?;
            let raw_points = reader.take("points")?;
            let Value::Array(rows) = &raw_points else {
                return Err(PayloadError::new("surface.points: expected an array"));
            };
            let mut points = Vec::with_capacity(rows.len());
            for row in rows {
                let Value::Array(cells) = row else {
                    return Err(PayloadError::new(
                        "surface.points: expected rows of four-number arrays",
                    ));
                };
                let mut cells_out = Vec::with_capacity(cells.len());
                for cell in cells {
                    let Value::Array(values) = cell else {
                        return Err(PayloadError::new(
                            "surface.points: expected a four-number array",
                        ));
                    };
                    if values.len() != 4 {
                        return Err(PayloadError::new(
                            "surface.points: expected a four-number array",
                        ));
                    }
                    cells_out.push([
                        json::decode_f64(&values[0]).map_err(PayloadError::from)?,
                        json::decode_f64(&values[1]).map_err(PayloadError::from)?,
                        json::decode_f64(&values[2]).map_err(PayloadError::from)?,
                        json::decode_f64(&values[3]).map_err(PayloadError::from)?,
                    ]);
                }
                points.push(cells_out);
            }
            BrepSurface::Nurbs {
                u_knots,
                v_knots,
                points,
            }
        }
        "unsupported" => BrepSurface::Unsupported {
            type_key: reader.string("type_key")?,
        },
        other => {
            return Err(PayloadError::new(format!(
                "surface.kind: unknown surface {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(surface)
}

fn encode_brep_curve(curve: &BrepCurve) -> Value {
    match curve {
        BrepCurve::Line { start, end } => fields(vec![
            ("kind", Value::String("line".to_string())),
            ("start", json::encode_point3(*start)),
            ("end", json::encode_point3(*end)),
        ]),
        BrepCurve::Circle {
            center,
            normal,
            u_dir,
            radius,
        } => fields(vec![
            ("kind", Value::String("circle".to_string())),
            ("center", json::encode_point3(*center)),
            ("normal", json::encode_point3(*normal)),
            ("u_dir", json::encode_point3(*u_dir)),
            ("radius", json::encode_f64(*radius)),
        ]),
        BrepCurve::Arc {
            center,
            normal,
            u_dir,
            radius,
            start,
            end,
        } => fields(vec![
            ("kind", Value::String("arc".to_string())),
            ("center", json::encode_point3(*center)),
            ("normal", json::encode_point3(*normal)),
            ("u_dir", json::encode_point3(*u_dir)),
            ("radius", json::encode_f64(*radius)),
            ("start", json::encode_point3(*start)),
            ("end", json::encode_point3(*end)),
        ]),
        BrepCurve::Unsupported { type_key } => fields(vec![
            ("kind", Value::String("unsupported".to_string())),
            ("type_key", Value::String(type_key.clone())),
        ]),
    }
}

fn decode_brep_curve(value: &Value) -> Result<BrepCurve, PayloadError> {
    let mut reader = ObjectReader::new(value, "curve")?;
    let kind = reader.string("kind")?;
    let curve = match kind.as_str() {
        "line" => BrepCurve::Line {
            start: reader.point3("start")?,
            end: reader.point3("end")?,
        },
        "circle" => BrepCurve::Circle {
            center: reader.point3("center")?,
            normal: reader.point3("normal")?,
            u_dir: reader.point3("u_dir")?,
            radius: reader.f64("radius")?,
        },
        "arc" => BrepCurve::Arc {
            center: reader.point3("center")?,
            normal: reader.point3("normal")?,
            u_dir: reader.point3("u_dir")?,
            radius: reader.f64("radius")?,
            start: reader.point3("start")?,
            end: reader.point3("end")?,
        },
        "unsupported" => BrepCurve::Unsupported {
            type_key: reader.string("type_key")?,
        },
        other => {
            return Err(PayloadError::new(format!(
                "curve.kind: unknown curve {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(curve)
}

fn encode_f64_matrix(rows: &[&[f64]]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| Value::Array(row.iter().map(|value| json::encode_f64(*value)).collect()))
            .collect(),
    )
}

fn decode_f64_matrix(
    value: &Value,
    rows: usize,
    columns: usize,
    field: &str,
) -> Result<Vec<Vec<f64>>, PayloadError> {
    let Value::Array(raw_rows) = value else {
        return Err(PayloadError::new(format!(
            "{field}: expected an array of rows"
        )));
    };
    if raw_rows.len() != rows {
        return Err(PayloadError::new(format!("{field}: expected {rows} rows")));
    }
    let mut out = Vec::with_capacity(rows);
    for raw_row in raw_rows {
        let Value::Array(raw_cells) = raw_row else {
            return Err(PayloadError::new(format!(
                "{field}: expected an array of rows"
            )));
        };
        if raw_cells.len() != columns {
            return Err(PayloadError::new(format!(
                "{field}: expected {columns} columns"
            )));
        }
        let mut cells = Vec::with_capacity(columns);
        for cell in raw_cells {
            cells.push(json::decode_f64(cell).map_err(PayloadError::from)?);
        }
        out.push(cells);
    }
    Ok(out)
}
