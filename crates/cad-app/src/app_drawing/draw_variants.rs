//! The 2026-11-07 draw variants: WIPEOUT and `POLYGON CIRCUMSCRIBED`.
//!
//! Split out of `drawing_create.rs` (which is at the file-size ceiling) so both
//! commands keep their full contracts next to each other. The methods are inherent
//! methods on [`Application`] in this same crate, so the dispatch in
//! `drawing_create::drawing_command` and the shared `commit_new_entity` funnel are
//! unchanged.

use super::*;

impl Application {
    /// Insert a closed polygon **mask** (`WIPEOUT`) into model space.
    ///
    /// Payload is `Points(boundary)`: the **closed implicit ring** of the mask —
    /// at least three finite, `+Z`-coplanar vertices in click order, with the
    /// first vertex **not** repeated at the end. The stored geometry is
    /// `SemanticGeometry::Mask { boundary, inverted: false }`; the entity's
    /// stored type key is `AcDbWipeout` (the domain's own `class_name` mapping,
    /// the same one the importer and the renderer use). One entity, one
    /// transaction and one undo step on the active layer through the shared
    /// [`Self::commit_new_entity`] funnel.
    ///
    /// Full contract, including the named NOT-IMPLEMENTED surface (`Frames`
    /// ON/OFF/`Display-not-plot`, the `Polyline`/`Erase Polyline` options,
    /// self-intersection rejection and picking): [`crate::command_id::draw_variants::WipeoutCommand`].
    /// Every refusal (`wipeout.payload`, `wipeout.too_few_points`,
    /// `wipeout.invalid_point`, `wipeout.not_planar`,
    /// `wipeout.duplicate_vertex`, `wipeout.zero_area`) happens before the
    /// transaction, so a refused command changes nothing.
    pub(crate) fn create_wipeout(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "wipeout.payload: WIPEOUT needs a Points([boundary…]) payload".into(),
            ));
        };
        let geometry = wipeout_geometry(points.clone())?;
        self.commit_new_entity(session, command, geometry, "create wipeout")
    }

    /// Insert a closed regular POLYGON **circumscribed about a circle**, as one
    /// entity.
    ///
    /// Payload is `Points([center, tangent_point])`: the centre and a point on the
    /// **inscribed (tangent) circle** — the circle the polygon's edges touch —
    /// exactly as clicked (the construction input, like [`Self::create_polygon`]).
    /// The side count is the session sysvar
    /// ([`SessionState::polygon_sides`], `3..=1024`), read here rather than carried
    /// on the payload, so the session stays the single authority.
    ///
    /// The geometry is built by
    /// [`crate::draw_tool::circumscribed_polygon_vertices`] — the single
    /// construction/validation point shared with the live preview. It derives the
    /// tangent radius `r` from the second pick, the vertex radius
    /// `R = r / cos(π/n)`, and places the vertices at `θ ± π/n` (the picked
    /// direction is the **midpoint of the first edge**), stepping counter-clockwise
    /// about `+Z`. Every edge's midpoint is exactly `r` from the centre, the ring is
    /// closed with one zero bulge per vertex, and the refusals are
    /// `polygoncircumscribed.invalid_point` / `.sides` / `.not_planar` /
    /// `.zero_radius`. Committed through the shared [`Self::commit_new_entity`]
    /// funnel: one entity, one transaction and one undo step, active-layer check
    /// intact.
    ///
    /// Only this circumscribed construction and the existing inscribed
    /// [`Self::create_polygon`] are authored; AutoCAD's `Edge` branch and a numeric
    /// radius that rotates the ring by the snap rotation angle remain **NOT
    /// IMPLEMENTED**.
    pub(crate) fn create_polygon_circumscribed(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreatePolygonCircumscribed needs a Points([center, tangent_point]) payload".into(),
            ));
        };
        let center = *points.first().ok_or_else(|| {
            CadError::InvalidInput("CreatePolygonCircumscribed needs a center point".into())
        })?;
        let tangent_point = *points.get(1).ok_or_else(|| {
            CadError::InvalidInput("CreatePolygonCircumscribed needs a tangent-circle point".into())
        })?;
        let vertices = crate::draw_tool::circumscribed_polygon_vertices(
            center,
            tangent_point,
            session.polygon_sides(),
        )?;
        let geometry = SemanticGeometry::Polyline {
            bulges: vec![0.0; vertices.len()],
            points: vertices,
            closed: true,
        };
        self.commit_new_entity(session, command, geometry, "create circumscribed polygon")
    }
}

/// The smallest XY distance (world units) between two consecutive WIPEOUT
/// boundary vertices.
///
/// A zero-length edge has no direction and makes the implicit ring degenerate;
/// like the other world epsilons this is a tiny relative-free constant, so an
/// exactly-duplicate click pair is refused while any real edge is accepted.
const MIN_WIPEOUT_VERTEX_SEPARATION: f64 = 1e-12;

/// The smallest enclosed area (world units²) for a WIPEOUT boundary to be a real
/// region rather than a collapsed (collinear) run of points.
///
/// The comparison is on the **shoelace sum** (twice the signed area), computed in
/// a frame translated to the first vertex so it is translation-invariant at large
/// world coordinates.
const MIN_WIPEOUT_AREA: f64 = 1e-12;

/// Build the closed polygon **mask** of a WIPEOUT from its boundary vertices.
///
/// The ring is implicit: the returned `boundary` keeps exactly the vertices given
/// (the first vertex is stored once, never repeated at the end). The stored mask
/// is always `inverted: false` — masking the inside — because the create path has
/// no inverted/frame model (see
/// [`crate::command_id::draw_variants::WipeoutCommand`]).
///
/// Explicit refusals that create nothing (stable machine prefixes, never a
/// silent fix-up or projection): `wipeout.too_few_points`, `wipeout.invalid_point`,
/// `wipeout.not_planar` (the flat `+Z` work-plane subset), `wipeout.duplicate_vertex`
/// (a zero-length edge, the closing pair included) and `wipeout.zero_area` (a
/// collapsed ring). Self-intersection is deliberately **not** checked: a
/// self-crossing ring is stored as given and filled with the renderer's even-odd
/// rule, except that a ring whose net shoelace area is zero (for example a
/// symmetric bow-tie) is refused as `wipeout.zero_area` — the area test is the
/// only degeneracy test this v1 runs.
fn wipeout_geometry(points: Vec<Point3>) -> CadResult<SemanticGeometry> {
    let count = points.len();
    if count < 3 {
        return Err(CadError::InvalidInput(format!(
            "wipeout.too_few_points: WIPEOUT needs at least three boundary vertices, got {count}"
        )));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(CadError::InvalidInput(
            "wipeout.invalid_point: every WIPEOUT boundary vertex must be finite".into(),
        ));
    }
    let first = points[0];
    let z_tolerance = 1e-9 * (1.0 + points.iter().map(|point| point.z.abs()).sum::<f64>());
    if points
        .iter()
        .any(|point| (point.z - first.z).abs() > z_tolerance)
    {
        return Err(CadError::InvalidInput(
            "wipeout.not_planar: WIPEOUT boundary vertices must lie in one +Z plane".into(),
        ));
    }
    for index in 0..count {
        let next = (index + 1) % count;
        let (a, b) = (points[index], points[next]);
        let separation = (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z);
        if separation <= MIN_WIPEOUT_VERTEX_SEPARATION {
            return Err(CadError::InvalidInput(format!(
                "wipeout.duplicate_vertex: boundary vertices {index} and {next} coincide"
            )));
        }
    }
    // Twice the signed area, relative to the first vertex (translation-invariant).
    let mut twice_area = 0.0;
    for index in 1..(count - 1) {
        let a = (points[index].x - first.x, points[index].y - first.y);
        let b = (points[index + 1].x - first.x, points[index + 1].y - first.y);
        twice_area += a.0 * b.1 - a.1 * b.0;
    }
    if !twice_area.is_finite() || twice_area.abs() <= MIN_WIPEOUT_AREA {
        return Err(CadError::InvalidInput(
            "wipeout.zero_area: the WIPEOUT boundary encloses no area".into(),
        ));
    }
    Ok(SemanticGeometry::Mask {
        boundary: points,
        inverted: false,
    })
}
