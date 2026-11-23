//! ACIS/SAT/SAB → neutral B-rep lift for 3DSOLID/BODY/REGION/SURFACE.
//!
//! This is the only place an acadrust `SatDocument` is interpreted. It walks
//! the body → shell → face → loop → coedge → edge → vertex record graph and
//! emits [`cad_kernel_adapter::BrepData`], which carries no acadrust type. The
//! kernel seam then tessellates that neutral data without ever seeing SAT/SAB.
//!
//! Nothing is guessed: a surface or curve this lift does not understand becomes
//! [`BrepSurface::Unsupported`] / [`BrepCurve::Unsupported`] and is reported by
//! the tessellator as a missing face. An unparseable payload stays raw bytes so
//! the default kernel reports it honestly as unsupported.

use acadrust::entities::acis::{
    SabReader, SatCoedge, SatConeSurface, SatDocument, SatEdge, SatEllipseCurve, SatFace, SatLoop,
    SatPlaneSurface, SatPoint, SatSphereSurface, SatTorusSurface, SatVertex, Sense,
};
use acadrust::entities::AcisData;
use acadrust::EntityType;
use cad_domain::Point3;
use cad_kernel_adapter::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface, SolidExchange,
    TessellationTolerance,
};

/// Lift a parsed SAT/SAB document into the neutral exchange type.
///
/// Faces are grouped by their owning shell pointer so a shell that produced no
/// facets can be reported as dropped. Surfaces and curves outside the
/// documented subset are marked unsupported rather than approximated.
///
/// # Tolerances
///
/// The SAT **record set** this reader walks (`body`/`lump`/`shell`/`face`/`loop`/
/// `coedge`/`edge`/`vertex`/`curve`) carries no per-body, per-face, per-edge or
/// per-vertex geometry tolerance, so every one of them stays `None` —
/// "unspecified", never a fabricated number. The three numbers in the SAT header
/// are document-level modeler settings, not this body's tolerance (acadrust's own
/// writer emits a spatial resolution of `10`, which as a 10 mm intersection
/// tolerance would loosen every later kernel call), so they are not used either.
pub fn sat_to_brep(document: &SatDocument) -> BrepData {
    let mut shells: Vec<BrepShell> = Vec::new();
    let mut order: Vec<i32> = Vec::new();
    for record in &document.records {
        if record.entity_type != "face" {
            continue;
        }
        let Some(face) = SatFace::from_record(record) else {
            continue;
        };
        let shell_ptr = face.shell().0;
        let surface = face_surface(document, &face);
        let reversed = face.sense() == Sense::Reversed;
        let loops = face_loops(document, &face);
        let brep_face = BrepFace {
            id: record.index as u32,
            surface,
            reversed,
            loops,
            // No per-face tolerance in the SAT record set: unspecified.
            tolerance: None,
            domain: None,
        };
        if let Some(pos) = order.iter().position(|&p| p == shell_ptr) {
            shells[pos].faces.push(brep_face);
        } else {
            order.push(shell_ptr);
            let id = if shell_ptr >= 0 {
                shell_ptr as u32
            } else {
                shells.len() as u32
            };
            shells.push(BrepShell {
                id,
                faces: vec![brep_face],
            });
        }
    }

    let placement = if document
        .records
        .iter()
        .any(|r| r.entity_type == "transform")
    {
        let (matrix, translation, scale) = document.placement();
        Some(BrepPlacement {
            matrix,
            translation: Point3 {
                x: translation[0],
                y: translation[1],
                z: translation[2],
            },
            scale,
        })
    } else {
        None
    };

    BrepData {
        shells,
        placement,
        // See the function docs: the record set states no body tolerance.
        tolerance: None,
    }
}

/// Build the kernel-facing exchange for a solid/surface entity.
///
/// Returns `None` for entity types that carry no ACIS body. A parsed payload
/// becomes [`SolidExchange::Brep`]; a payload acadrust cannot parse stays the
/// raw `Sat`/`Sab` bytes so the kernel refuses it honestly.
pub fn solid_exchange_from_entity(entity: &EntityType) -> Option<SolidExchange> {
    let acis = match entity {
        EntityType::Solid3D(s) => &s.acis_data,
        EntityType::Region(r) => &r.acis_data,
        EntityType::Body(b) => &b.acis_data,
        EntityType::Surface(s) => &s.acis_data,
        _ => return None,
    };
    Some(acis_exchange(acis))
}

/// Convert one entity's ACIS data without needing the entity wrapper.
pub fn acis_exchange(acis: &AcisData) -> SolidExchange {
    match acis.parse() {
        Some(document) => SolidExchange::Brep(sat_to_brep(&document)),
        None if acis.is_binary => SolidExchange::Sab(acis.sab_data.clone()),
        None => SolidExchange::Sat(acis.sat_data.as_bytes().to_vec()),
    }
}

/// The raw SAT/SAB bytes and a format tag (`1` = SAT text, `2` = SAB binary),
/// for retaining provenance in an opaque payload.
pub fn acis_raw_payload(acis: &AcisData) -> (u32, Vec<u8>) {
    if acis.is_binary {
        (2, acis.sab_data.clone())
    } else {
        (1, acis.sat_data.as_bytes().to_vec())
    }
}

/// Decode binary SAB bytes directly, for callers that hold raw bytes.
pub fn sab_to_brep(bytes: &[u8]) -> Option<BrepData> {
    SabReader::read(bytes)
        .ok()
        .map(|document| sat_to_brep(&document))
}

// The shape-relative tessellation policy lives in `cad-kernel-adapter` (shared
// with command-time body creation); this path stays the importer's entry point.

/// The import-time tessellation tolerance (the shared shape-relative rule).
pub fn acis_tolerance(brep: &BrepData) -> (TessellationTolerance, f64) {
    cad_kernel_adapter::shape_relative_tolerance(brep)
}

fn face_surface(document: &SatDocument, face: &SatFace<'_>) -> BrepSurface {
    let Some(record) = document.resolve(face.surface()) else {
        return BrepSurface::Unsupported {
            type_key: "missing-surface".into(),
        };
    };
    match record.entity_type.as_str() {
        "plane-surface" => match SatPlaneSurface::from_record(record) {
            Some(plane) => BrepSurface::Plane {
                origin: tuple(plane.root_point()),
                normal: tuple(plane.normal()),
                u_dir: tuple(plane.u_direction()),
            },
            None => unsupported(record),
        },
        "cone-surface" => match SatConeSurface::from_record(record) {
            // A cone with sin(half-angle) == 0 is a cylinder; its side face is
            // bounded by two full circles, which this build can tessellate.
            Some(cone) if cone.sin_half_angle().abs() < 1e-9 => BrepSurface::Cylinder {
                origin: tuple(cone.center()),
                axis: tuple(cone.axis()),
                ref_dir: tuple(cone.major_axis()),
                radius: cone.radius(),
            },
            // A genuine cone (or truncated cone): the lateral face is carried by
            // the half-angle and the reference cross-section radius.
            Some(cone) => BrepSurface::Cone {
                origin: tuple(cone.center()),
                axis: tuple(cone.axis()),
                ref_dir: tuple(cone.major_axis()),
                radius: cone.radius(),
                sin_half_angle: cone.sin_half_angle(),
                cos_half_angle: cone.cos_half_angle(),
            },
            None => unsupported(record),
        },
        "sphere-surface" => match SatSphereSurface::from_record(record) {
            Some(sphere) => BrepSurface::Sphere {
                center: tuple(sphere.center()),
                radius: sphere.radius(),
                u_dir: tuple(sphere.u_direction()),
                pole: tuple(sphere.pole()),
            },
            None => unsupported(record),
        },
        "torus-surface" => match SatTorusSurface::from_record(record) {
            Some(torus) => BrepSurface::Torus {
                center: tuple(torus.center()),
                normal: tuple(torus.normal()),
                major_radius: torus.major_radius(),
                minor_radius: torus.minor_radius(),
                u_dir: tuple(torus.u_direction()),
            },
            None => unsupported(record),
        },
        other => BrepSurface::Unsupported {
            type_key: other.to_string(),
        },
    }
}

fn unsupported(record: &acadrust::entities::acis::SatRecord) -> BrepSurface {
    BrepSurface::Unsupported {
        type_key: record.entity_type.clone(),
    }
}

fn face_loops(document: &SatDocument, face: &SatFace<'_>) -> Vec<BrepLoop> {
    let mut out = Vec::new();
    let max = document.records.len() + 1;
    let mut ptr = face.first_loop();
    let mut guard = 0;
    while let Some(record) = document.resolve(ptr) {
        guard += 1;
        if guard > max {
            break;
        }
        let Some(loop_) = SatLoop::from_record(record) else {
            break;
        };
        if let Some(brep_loop) = build_loop(document, &loop_) {
            out.push(brep_loop);
        }
        let next = loop_.next_loop();
        if next.is_null() || next.0 == ptr.0 {
            break;
        }
        ptr = next;
    }
    out
}

fn build_loop(document: &SatDocument, loop_: &SatLoop<'_>) -> Option<BrepLoop> {
    let first = loop_.first_coedge();
    if first.is_null() {
        return None;
    }
    let max = document.records.len() + 1;
    let mut edges = Vec::new();
    let mut ptr = first;
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > max {
            break;
        }
        let record = document.resolve(ptr)?;
        let coedge = SatCoedge::from_record(record)?;
        edges.push(coedge_curve(document, &coedge));
        let next = coedge.next();
        if next.is_null() || next.0 == first.0 {
            break;
        }
        ptr = next;
    }
    if edges.is_empty() {
        None
    } else {
        Some(BrepLoop { edges })
    }
}

fn coedge_curve(document: &SatDocument, coedge: &SatCoedge<'_>) -> BrepCurve {
    let Some(edge_record) = document.resolve(coedge.edge()) else {
        return BrepCurve::Unsupported {
            type_key: "missing-edge".into(),
        };
    };
    let Some(edge) = SatEdge::from_record(edge_record) else {
        return BrepCurve::Unsupported {
            type_key: edge_record.entity_type.clone(),
        };
    };
    let forward = coedge.sense() == Sense::Forward;
    let (start_ptr, end_ptr) = if forward {
        (edge.start_vertex(), edge.end_vertex())
    } else {
        (edge.end_vertex(), edge.start_vertex())
    };
    let start = vertex_point(document, start_ptr);
    let end = vertex_point(document, end_ptr);

    let Some(curve_record) = document.resolve(edge.curve()) else {
        // No underlying curve: a degenerate/singular edge (for example a cone
        // apex). It cannot be evaluated as part of a closed subset.
        return BrepCurve::Unsupported {
            type_key: "edge-without-curve".into(),
        };
    };
    match curve_record.entity_type.as_str() {
        "straight-curve" => match (start, end) {
            (Some(start), Some(end)) => BrepCurve::Line { start, end },
            _ => BrepCurve::Unsupported {
                type_key: "missing-vertex".into(),
            },
        },
        "ellipse-curve" => {
            let Some(ellipse) = SatEllipseCurve::from_record(curve_record) else {
                return BrepCurve::Unsupported {
                    type_key: "ellipse-curve".into(),
                };
            };
            // Only a circle is mapped: a non-unit ratio is a genuine ellipse, a
            // curve this build cannot evaluate.
            if (ellipse.ratio() - 1.0).abs() > 1e-9 {
                return BrepCurve::Unsupported {
                    type_key: "ellipse-arc".into(),
                };
            }
            let major = tuple(ellipse.major_axis());
            let radius = (major.x * major.x + major.y * major.y + major.z * major.z).sqrt();
            if !radius.is_finite() || radius <= 0.0 {
                return BrepCurve::Unsupported {
                    type_key: "zero-radius-circle".into(),
                };
            }
            let center = tuple(ellipse.center());
            let normal = tuple(ellipse.normal());
            let u_dir = normalize(major);
            // A full circle is an edge whose start and end vertex coincide.
            let closed = !edge.start_vertex().is_null() && edge.start_vertex() == edge.end_vertex();
            if closed {
                return BrepCurve::Circle {
                    center,
                    normal,
                    u_dir,
                    radius,
                };
            }
            arc_curve(&edge, center, normal, u_dir, radius, start, end)
        }
        other => BrepCurve::Unsupported {
            type_key: other.to_string(),
        },
    }
}

/// The neutral arc an **open** `ellipse-curve` edge describes, when it is one.
///
/// The edge's vertices (already in loop order at the call site) must lie on the
/// circle — in radius **and** in plane — or the record is not a circular arc and
/// stays [`BrepCurve::Unsupported`] (`partial-circle`) rather than being
/// re-projected.
///
/// # Orientation
///
/// The neutral arc's traversal is counter-clockwise about `normal`, and the two
/// ways round a chord are different arcs, so the direction is read from the
/// record rather than guessed:
///
/// * ACIS parameterises the ellipse counter-clockwise about its own normal, from
///   the major axis, and an edge records both ends' parameters
///   (`start_param`/`end_param`);
/// * its span is therefore `(end_param - start_param) mod 2π` — a **remainder**,
///   not `|end - start|`, because ACIS parameters may wrap past a full turn
///   (`5.5 → 0.5` is `1.28` rad, not `5.0`);
/// * of the two candidate sweeps from the loop-oriented `start` to its `end`
///   (about `normal` and about `-normal`), the one whose sweep matches that span
///   is the arc.
///
/// A **half arc** sweeps `π` either way, so the span cannot decide it: there the
/// ACIS counter-clockwise convention does, which is the `about_normal` branch —
/// i.e. a 180° arc is taken to run counter-clockwise about the ellipse normal
/// unless the recorded span says otherwise. An arc whose span matches *neither*
/// candidate is an inconsistent record and stays unsupported.
///
/// # No span
///
/// acadrust's v400 up-converter inserts `Float(0.0)` placeholders for edge
/// parameters, so every **v400** arc arrives with a zero span. It is reported as
/// `arc-without-span` (not as a mysterious "partial circle"), and v400 SAT arcs
/// are not importable through this path.
fn arc_curve(
    edge: &SatEdge<'_>,
    center: Point3,
    normal: Point3,
    u_dir: Point3,
    radius: f64,
    start: Option<Point3>,
    end: Option<Point3>,
) -> BrepCurve {
    let unsupported = || BrepCurve::Unsupported {
        type_key: "partial-circle".into(),
    };
    let (Some(start), Some(end)) = (start, end) else {
        return unsupported();
    };
    let n = normalize(normal);
    let u = normalize(u_dir);
    let v = normalize(cross(n, u));
    if !(is_axis(n) && is_axis(u) && is_axis(v)) || dot(n, u).abs() > 1e-6 {
        return unsupported();
    }
    let allowed = 1e-9 * radius.max(1.0);
    for p in [start, end] {
        let d = sub(p, center);
        if (length(d) - radius).abs() > allowed || dot(d, n).abs() > allowed {
            return unsupported();
        }
    }
    let angle = |p: Point3| {
        let d = sub(p, center);
        dot(d, v).atan2(dot(d, u))
    };
    let (from, to) = (angle(start), angle(end));
    if !from.is_finite() || !to.is_finite() {
        return unsupported();
    }
    let delta = to - from;
    let about_normal = if delta > 0.0 {
        delta
    } else {
        delta + std::f64::consts::TAU
    };
    let about_reversed = std::f64::consts::TAU - about_normal;
    let delta_param = edge.end_param() - edge.start_param();
    if !delta_param.is_finite() {
        return BrepCurve::Unsupported {
            type_key: "arc-without-span".into(),
        };
    }
    // A remainder, not an absolute difference: ACIS edge parameters may wrap past
    // a full turn.
    let span = delta_param.rem_euclid(std::f64::consts::TAU);
    if span <= 1e-9 {
        // Zero span: the record states no span at all (every v400 arc, whose edge
        // parameters the up-converter pads with `0.0`). Named, because calling it
        // a "partial circle" points at the wrong thing.
        return BrepCurve::Unsupported {
            type_key: "arc-without-span".into(),
        };
    }
    let slack = 1e-6 * (1.0 + span);
    if (about_normal - span).abs() <= slack {
        BrepCurve::Arc {
            center,
            normal: n,
            u_dir: u,
            radius,
            start,
            end,
        }
    } else if (about_reversed - span).abs() <= slack {
        BrepCurve::Arc {
            center,
            normal: Point3 {
                x: -n.x,
                y: -n.y,
                z: -n.z,
            },
            u_dir: u,
            radius,
            start,
            end,
        }
    } else {
        unsupported()
    }
}

fn vertex_point(
    document: &SatDocument,
    ptr: acadrust::entities::acis::SatPointer,
) -> Option<Point3> {
    let record = document.resolve(ptr)?;
    let vertex = SatVertex::from_record(record)?;
    let point_record = document.resolve(vertex.point())?;
    let point = SatPoint::from_record(point_record)?;
    let (x, y, z) = point.position();
    Some(Point3 { x, y, z })
}

fn tuple(value: (f64, f64, f64)) -> Point3 {
    Point3 {
        x: value.0,
        y: value.1,
        z: value.2,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

/// Whether a normalised direction is usable as a frame axis.
fn is_axis(v: Point3) -> bool {
    length(v) > 0.5
}

fn normalize(v: Point3) -> Point3 {
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if len > 1e-12 {
        Point3 {
            x: v.x / len,
            y: v.y / len,
            z: v.z / len,
        }
    } else {
        v
    }
}
