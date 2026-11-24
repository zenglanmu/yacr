//! The 2026-11-07 draw batch: `DONUT`, `SOLID`, `HELIX`, `MLINE` and
//! `MLSTYLE`.
//!
//! Every creating command validates its input **before** touching the drawing
//! and then goes through the shared create funnel
//! ([`Application::commit_new_entities`]): one transaction, one undo step on the
//! session's active layer. `MLSTYLE` writes no drawing data at all — it only
//! maintains the session-only bounded style store that `MLINE` reads.
//!
//! Full contracts: [`crate::command_id::draw2`].

use super::*;

/// Segments sampled per HELIX turn (bounded together by [`MAX_HELIX_VERTICES`]).
const HELIX_SEGMENTS_PER_TURN: usize = 64;
/// The explicit HELIX vertex cap (one more than the maximum segment count).
const MAX_HELIX_VERTICES: usize = 4096;
/// Smallest accepted SOLID / MLINE point separation, in world units.
const MIN_BATCH_SEPARATION: f64 = 1e-12;
/// The largest MLINE baseline this command accepts (2·(N-1) LINEs are written).
const MAX_MLINE_VERTICES: usize = 1024;
/// Smallest `1 + n₀·n₁` that still defines a finite MLINE miter corner.
const MLINE_JOIN_EPS: f64 = 1e-9;
/// Smallest accepted SOLID area (twice the shoelace sum), in world units².
const MIN_SOLID_AREA: f64 = 1e-12;

fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `DONUT`: one filled annular (inner > 0) or disk (inner == 0) face on the
    /// active layer, one transaction.
    ///
    /// The filled face is the **same triangle-mesh representation `SOLID` uses**
    /// (`AcDbSolid`), not a pair of concentric circles: the disk is a triangle fan
    /// and the ring is a triangle strip between the outer and inner rings.
    pub(crate) fn donut(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Draw2(payload) = &command.payload else {
            return Err(refusal(
                "donut.payload",
                "DONUT needs a Draw2::Donut { center, inner, outer } payload",
            ));
        };
        let Draw2Payload::Donut {
            center,
            inner,
            outer,
        } = payload.as_ref()
        else {
            return Err(refusal(
                "donut.payload",
                "DONUT needs a Draw2::Donut { center, inner, outer } payload",
            ));
        };
        if !is_finite_point(*center) {
            return Err(refusal(
                "donut.invalid_point",
                "the DONUT centre must be finite",
            ));
        }
        if !inner.is_finite() || !outer.is_finite() || *inner < 0.0 {
            return Err(refusal(
                "donut.invalid_radius",
                format!(
                    "DONUT radii must be finite and inner >= 0, got inner {inner}, outer {outer}"
                ),
            ));
        }
        if *outer <= *inner {
            return Err(refusal(
                "donut.radius_order",
                format!("DONUT needs outer > inner, got inner {inner}, outer {outer}"),
            ));
        }
        let zero_inner = *inner == 0.0;
        let mesh = donut_mesh(*center, *inner, *outer);
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            SemanticGeometry::Mesh(mesh),
            "AcDbSolid",
            "create donut",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "drawing.donut".into(),
            message: format!(
                "DONUT: stored one filled {} as triangle-mesh geometry (the SOLID path); \
                 outer radius {outer}, inner radius {inner}",
                if zero_inner { "disk" } else { "annulus" }
            ),
        });
        Ok(outcome)
    }

    /// `SOLID`: one filled triangle/quad as a two-triangle `Mesh`.
    pub(crate) fn solid(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Draw2(payload) = &command.payload else {
            return Err(refusal(
                "solid.payload",
                "SOLID needs a Draw2::Solid { points } payload",
            ));
        };
        let Draw2Payload::Solid { points } = &**payload else {
            return Err(refusal(
                "solid.payload",
                "SOLID needs a Draw2::Solid { points } payload",
            ));
        };
        let mesh = solid_mesh(points)?;
        self.commit_new_entity_typed(
            session,
            command,
            SemanticGeometry::Mesh(mesh),
            "AcDbSolid",
            "create solid",
        )
    }

    /// `HELIX`: one flat 2D spiral polyline in the XY plane.
    pub(crate) fn helix(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Draw2(payload) = &command.payload else {
            return Err(refusal(
                "helix.payload",
                "HELIX needs a Draw2::Helix { base, radius, turns, height } payload",
            ));
        };
        let Draw2Payload::Helix {
            base,
            radius,
            turns,
            height,
        } = payload.as_ref()
        else {
            return Err(refusal(
                "helix.payload",
                "HELIX needs a Draw2::Helix { base, radius, turns, height } payload",
            ));
        };
        if !is_finite_point(*base) {
            return Err(refusal(
                "helix.invalid_point",
                "the HELIX base point must be finite",
            ));
        }
        if !radius.is_finite() || *radius <= 0.0 {
            return Err(refusal(
                "helix.invalid_radius",
                format!("HELIX radius must be finite and > 0, got {radius}"),
            ));
        }
        if !turns.is_finite() || *turns <= 0.0 {
            return Err(refusal(
                "helix.invalid_turns",
                format!("HELIX turns must be finite and > 0, got {turns}"),
            ));
        }
        if !height.is_finite() || *height < 0.0 {
            return Err(refusal(
                "helix.invalid_height",
                format!("HELIX height must be finite and >= 0, got {height}"),
            ));
        }
        if *height > 0.0 {
            return Err(CadError::Unsupported(format!(
                "helix.3d_not_implemented: the 3D spring form (height {height}) is not \
                 implemented; only the flat 2D spiral (height 0) exists"
            )));
        }
        let points = helix_points(*base, *radius, *turns);
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Polyline {
                bulges: vec![0.0; points.len()],
                points,
                closed: false,
                width: 0.0,
            },
            "create helix",
        )
    }

    /// `MLINE`: the mitered element LINEs of a multiline over an N-point
    /// baseline.
    pub(crate) fn mline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Draw2(payload) = &command.payload else {
            return Err(refusal(
                "mline.payload",
                "MLINE needs a Draw2::Mline { spacing, offset, points } payload",
            ));
        };
        let Draw2Payload::Mline {
            spacing,
            offset,
            points,
        } = &**payload
        else {
            return Err(refusal(
                "mline.payload",
                "MLINE needs a Draw2::Mline { spacing, offset, points } payload",
            ));
        };
        if !(2..=MAX_MLINE_VERTICES).contains(&points.len()) {
            return Err(refusal(
                "mline.point_count",
                format!(
                    "MLINE needs 2..={MAX_MLINE_VERTICES} baseline points, got {}",
                    points.len()
                ),
            ));
        }
        if !points.iter().all(|point| is_finite_point(*point)) {
            return Err(refusal(
                "mline.invalid_point",
                "every MLINE baseline point must be finite",
            ));
        }
        if !offset.is_finite() {
            return Err(refusal(
                "mline.invalid_offset",
                format!("the MLINE offset must be finite, got {offset}"),
            ));
        }
        let spacing = match *spacing {
            Some(value) => value,
            None => session.ml_styles.current_spacing(),
        };
        if !spacing.is_finite() || spacing <= 0.0 {
            return Err(refusal(
                "mline.invalid_spacing",
                format!("MLINE spacing must be finite and > 0, got {spacing}"),
            ));
        }
        let geometries = mline_geometries(points, spacing, *offset)?;
        self.commit_new_entities(session, command, geometries, "create mline")
    }

    /// `MLSTYLE`: report or set one session-only multiline style.
    pub(crate) fn mlstyle(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Draw2(payload) = &command.payload else {
            return Err(refusal(
                "mlstyle.payload",
                "MLSTYLE needs a Draw2::MlStyle(List | Set { name, spacing }) payload",
            ));
        };
        let action = match &**payload {
            Draw2Payload::MlStyle(action) => action.clone(),
            _ => {
                return Err(refusal(
                    "mlstyle.payload",
                    "MLSTYLE needs a Draw2::MlStyle(List | Set { name, spacing }) payload",
                ));
            }
        };
        match action {
            MlStyleCommand::List => {
                let current = session.ml_styles.current_name().to_string();
                let styles: Vec<Diagnostic> = session
                    .ml_styles
                    .styles()
                    .iter()
                    .map(|style| Diagnostic {
                        object: None,
                        code: "mlstyle.entry".into(),
                        message: format!(
                            "{}{} spacing={}",
                            if style.name == current { "* " } else { "  " },
                            style.name,
                            style.spacing
                        ),
                    })
                    .collect();
                let count = styles.len();
                let mut diagnostics = styles;
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "mlstyle.list".into(),
                    message: format!(
                        "MLSTYLE LIST: {count} session-only style(s); the current one is {current}"
                    ),
                });
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics,
                    measurement: None,
                    point_report: None,
                })
            }
            MlStyleCommand::Set { name, spacing } => {
                session
                    .ml_styles
                    .set(&name, spacing)
                    .map_err(CadError::InvalidInput)?;
                let current = session.ml_styles.current();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "mlstyle.set".into(),
                        message: format!(
                            "MLSTYLE SET: current style is {} at spacing {} (session-only)",
                            current.name, current.spacing
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
        }
    }
}

/// Build the filled face of a SOLID from 3 or 4 ring-ordered points.
fn solid_mesh(points: &[Point3]) -> CadResult<Mesh> {
    if points.len() < 3 || points.len() > 4 {
        return Err(refusal(
            "solid.point_count",
            format!("SOLID needs 3 or 4 points, got {}", points.len()),
        ));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(refusal(
            "solid.invalid_point",
            "every SOLID point must be finite",
        ));
    }
    let first = points[0];
    let z_tolerance = 1e-9 * (1.0 + points.iter().map(|point| point.z.abs()).sum::<f64>());
    if points
        .iter()
        .any(|point| (point.z - first.z).abs() > z_tolerance)
    {
        return Err(refusal(
            "solid.not_planar",
            "SOLID points must lie in one +Z plane",
        ));
    }
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        let (a, b) = (points[index], points[next]);
        if (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z) <= MIN_BATCH_SEPARATION {
            return Err(refusal(
                "solid.duplicate_point",
                format!("SOLID points {index} and {next} coincide"),
            ));
        }
    }
    if points.len() == 4 && quad_crosses(points) {
        return Err(refusal(
            "solid.self_crossing",
            "the SOLID quadrilateral's edges cross; pass the corners in boundary order",
        ));
    }
    let mut twice_area = 0.0;
    for index in 0..points.len() {
        let a = (points[index].x - first.x, points[index].y - first.y);
        let b = (
            points[(index + 1) % points.len()].x - first.x,
            points[(index + 1) % points.len()].y - first.y,
        );
        twice_area += a.0 * b.1 - a.1 * b.0;
    }
    if !twice_area.is_finite() || twice_area.abs() <= MIN_SOLID_AREA {
        return Err(refusal(
            "solid.zero_area",
            "the SOLID face encloses no area",
        ));
    }
    let triangles = if points.len() == 3 {
        vec![[0, 1, 2]]
    } else {
        vec![[0, 1, 2], [0, 2, 3]]
    };
    let mesh = Mesh {
        vertices: points.to_vec(),
        triangles,
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        // A SOLID/TRACE face has no surface parameterisation: no UV0, no texture.
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    };
    let normals = cad_geometry::compute_vertex_normals(&mesh);
    Ok(Mesh { normals, ..mesh })
}

/// Segments per full circle in the filled DONUT mesh.
const DONUT_SEGMENTS: usize = 64;

/// The filled face of a DONUT: a triangle fan for a disk (`inner == 0`) or a
/// triangle strip between the outer and inner rings for an annulus.
///
/// Both rings wind counter-clockwise in XY, so every triangle's normal points
/// `+Z` (the same convention `solid_mesh` produces), and the vertex normals are
/// the shared computed ones.
fn donut_mesh(center: Point3, inner: f64, outer: f64) -> Mesh {
    let mut vertices = Vec::with_capacity(2 * DONUT_SEGMENTS + 1);
    let mut triangles = Vec::with_capacity(2 * DONUT_SEGMENTS);
    if inner == 0.0 {
        vertices.push(center);
        for index in 0..DONUT_SEGMENTS {
            let angle = std::f64::consts::TAU * index as f64 / DONUT_SEGMENTS as f64;
            vertices.push(Point3 {
                x: center.x + outer * angle.cos(),
                y: center.y + outer * angle.sin(),
                z: center.z,
            });
        }
        for index in 0..DONUT_SEGMENTS {
            let next = (index + 1) % DONUT_SEGMENTS;
            triangles.push([0, index as u32 + 1, next as u32 + 1]);
        }
    } else {
        for index in 0..DONUT_SEGMENTS {
            let angle = std::f64::consts::TAU * index as f64 / DONUT_SEGMENTS as f64;
            vertices.push(Point3 {
                x: center.x + outer * angle.cos(),
                y: center.y + outer * angle.sin(),
                z: center.z,
            });
        }
        for index in 0..DONUT_SEGMENTS {
            let angle = std::f64::consts::TAU * index as f64 / DONUT_SEGMENTS as f64;
            vertices.push(Point3 {
                x: center.x + inner * angle.cos(),
                y: center.y + inner * angle.sin(),
                z: center.z,
            });
        }
        let n = DONUT_SEGMENTS as u32;
        for index in 0..n {
            let next = (index + 1) % n;
            let outer_current = index;
            let outer_next = next;
            let inner_current = n + index;
            let inner_next = n + next;
            triangles.push([outer_current, outer_next, inner_next]);
            triangles.push([outer_current, inner_next, inner_current]);
        }
    }
    let mesh = Mesh {
        vertices,
        triangles,
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    };
    let normals = cad_geometry::compute_vertex_normals(&mesh);
    Mesh { normals, ..mesh }
}

/// Whether the two diagonals of a quadrilateral cross (a bow-tie).
fn quad_crosses(points: &[Point3]) -> bool {
    segments_cross(points[0], points[1], points[2], points[3])
        || segments_cross(points[1], points[2], points[3], points[0])
}

fn segments_cross(a: Point3, b: Point3, c: Point3, d: Point3) -> bool {
    let orientation =
        |p: Point3, q: Point3, r: Point3| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    (o1 * o2 < 0.0) && (o3 * o4 < 0.0)
}

/// Sample the flat spiral of a HELIX: radius grows linearly from 0 to `radius`.
fn helix_points(base: Point3, radius: f64, turns: f64) -> Vec<Point3> {
    let requested = (turns * HELIX_SEGMENTS_PER_TURN as f64).ceil();
    let segments = if requested.is_finite() {
        (requested as usize).clamp(2, MAX_HELIX_VERTICES - 1)
    } else {
        MAX_HELIX_VERTICES - 1
    };
    let mut points = Vec::with_capacity(segments + 1);
    for index in 0..=segments {
        let t = index as f64 / segments as f64;
        let angle = t * turns * std::f64::consts::TAU;
        let r = radius * t;
        points.push(Point3 {
            x: base.x + r * angle.cos(),
            y: base.y + r * angle.sin(),
            z: base.z,
        });
    }
    points
}

/// Build the element LINEs of a partial MLINE over an N-point baseline.
///
/// The stored form is **one LINE per element per segment** (this build has no
/// `AcDbMline` entity), so an `N`-point baseline is `2·(N-1)` LINEs. Interior
/// vertices are **mitered** — AutoCAD's default MLINE join — by offsetting each
/// vertex to the intersection of the two adjacent offset lines
/// (`n₀ + n₁) / (1 + n₀·n₁)`), so an element turns a corner without a notch and
/// without a doubled segment.
///
/// Refusals (nothing written):
///
/// * `mline.degenerate_baseline` — a segment has no length;
/// * `mline.join_degenerate` — an exact 180° reversal, whose miter point is at
///   infinity (there is no finite corner to join).
fn mline_geometries(
    points: &[Point3],
    spacing: f64,
    offset: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    let mut normals: Vec<(f64, f64)> = Vec::with_capacity(points.len().saturating_sub(1));
    for pair in points.windows(2) {
        let (dx, dy) = (pair[1].x - pair[0].x, pair[1].y - pair[0].y);
        let length = dx.hypot(dy);
        if !length.is_finite() || length <= MIN_BATCH_SEPARATION {
            return Err(refusal(
                "mline.degenerate_baseline",
                "an MLINE baseline segment has no length",
            ));
        }
        // The left normal of the segment (the element offset direction).
        normals.push((-dy / length, dx / length));
    }
    // One offset direction per vertex: the segment normal at an end, the miter
    // direction at an interior corner.
    let mut miters: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    miters.push(normals[0]);
    for index in 1..points.len() - 1 {
        let (n0, n1) = (normals[index - 1], normals[index]);
        let denominator = 1.0 + (n0.0 * n1.0 + n0.1 * n1.1);
        if denominator.abs() <= MLINE_JOIN_EPS {
            return Err(refusal(
                "mline.join_degenerate",
                format!(
                    "the baseline reverses direction by 180° at vertex {index}; its miter point \
                     is at infinity, so the two elements have no finite corner to join"
                ),
            ));
        }
        miters.push(((n0.0 + n1.0) / denominator, (n0.1 + n1.1) / denominator));
    }
    miters.push(normals[normals.len() - 1]);

    let half = spacing / 2.0;
    let mut geometries = Vec::with_capacity(2 * normals.len());
    for element in [half + offset, -half + offset] {
        let shifted: Vec<Point3> = points
            .iter()
            .zip(&miters)
            .map(|(point, (mx, my))| Point3 {
                x: point.x + mx * element,
                y: point.y + my * element,
                z: point.z,
            })
            .collect();
        for pair in shifted.windows(2) {
            geometries.push(SemanticGeometry::Line {
                start: pair[0],
                end: pair[1],
            });
        }
    }
    Ok(geometries)
}
