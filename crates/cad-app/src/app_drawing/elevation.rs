//! `ELEV`: the session elevation every newly created entity is placed at.
//!
//! AutoCAD's `ELEV` stores "the current elevation for new objects": a new object
//! is created at that z instead of zero. This build has one creation funnel, so
//! the rule is applied once, at the funnel's three create entry points
//! ([`crate::app_drawing::Application`]'s `commit_new_entity`,
//! `commit_new_entities` and `commit_new_entity_typed`), and nowhere else.
//!
//! # What the offset is
//!
//! A pure z translation of the created geometry by the session token
//! ([`crate::SessionState::drafting_extras`] → `DraftingExtras::elev`). It is the
//! same operation `MOVE` applies, expressed through the one transform authority
//! (`cad_db::transform_geometry`), so a created CIRCLE's centre, a created
//! TEXT's insertion point, a hatch's plane and an INSERT's placement all move
//! together — no kind is translated by a second, private rule.
//!
//! `elev == 0.0` (the default, and AutoCAD's initial value) is **identity**: the
//! geometry is returned untouched, so a session that never typed `ELEV` produces
//! byte-identical entities to the pre-`ELEV` build.
//!
//! The token describes **model-space** elevation because that is the space every
//! create command inserts into (`commit_new_entity`/`commit_new_entities`/
//! `commit_new_entity_typed` all pass `SpaceId::Model`); no create command in this
//! build creates in a paper layout, so there is no second space for the token to
//! mean something different in.
//!
//! `Shape` is the one kind `cad_db::transform_geometry` refuses (it has no
//! transform arm); its geometry is `position` + `size` + `rotation`, so a z
//! translation is exactly `position.z + elev` and is applied here directly rather
//! than refusing a command AutoCAD elevates.
//!
//! # Which commands are covered
//!
//! Every command whose handler inserts its entity/entities through the three
//! create entry points, i.e. the draw family (`LINE`, `XLINE`, `RAY`, `CIRCLE`
//! and its `2P`/`3P`/`D` forms, `ARC`, `PLINE`, `RECTANG`, `POLYGON` and
//! `POLYGON CIRCUMSCRIBED`, `ELLIPSE`, `SPLINE`, `POINT`, `DONUT`, `SOLID`,
//! `MLINE`, `HELIX`, `SKETCH`, `TRACE`, `WIPEOUT`, `BOUNDARY`, `HATCH`,
//! `-HATCH` and `HATCH` from an internal point/`GRADIENT`, `REVISIONCLOUD`,
//! `TEXT`, `MTEXT`, `MLEADER`, `SHAPE`, `TABLE`/`-TABLE`, `TOLERANCE`,
//! `IMAGEATTACH`, `INSERT`, the `DIVIDE` point marks and the dimension creations
//! `DIMLINEAR`/`DIMALIGNED`/`DIMROTATED`/`DIMANGULAR`/`DIMORDINATE`/
//! `DIMDIAMETER`/`DIMCENTER`/`DIMBASELINE`/`DIMCONTINUE`/`QDIM`).
//!
//! # Which commands are deliberately *not* covered
//!
//! Commands that **copy or move existing rows** keep the source's own z (the
//! source already carries its elevation and re-elevating a copy would move it
//! away from the object it was derived from): `OFFSET`, `COPY`, `MIRROR`,
//! `ARRAY`/`ARRAYCLASSIC`/`ARRAYPATH`, `PASTE`/`PASTEBLOCK`, `ADDSELECTED`
//! (which goes through the explicit-display copy entry point), `CHSPACE` and
//! every edit (`MOVE`/`ROTATE`/`SCALE`/`STRETCH`/`JOIN`/…). That is why the
//! elevation is not applied inside `insert_new_entities` itself: the copy path
//! reaches it too.
//!
//! A `Body3d` is exempt **here** for a different reason: the solid primitive
//! commands resolve their own base plane before the funnel. The explicit 3D
//! points form carries the author's own z, so nothing may translate it; the
//! AutoCAD option constructions (`BOX CUBE`, `CYLINDER DIAMETER`, …) are plan
//! inputs whose base z is the session `ELEV` token, applied once by
//! [`crate::app_drawing::solid_spec`] when the body is built. Translating the
//! finished `Body3d` would therefore double-apply `ELEV` to the plan forms, so
//! the exemption is the correct rule for both solid shapes.
//!
//! # Refusals
//!
//! The session setter only stores a finite token (`elev.invalid_value`), and a
//! translation that is not representable is refused **before** the transaction
//! opens: `elev.unsupported_kind` for the kinds the transform surface cannot map
//! (`Opaque`/`Compound`; creation never produces them) and
//! `elev.invalid_geometry` when the offset overflows a coordinate. Both write
//! nothing.

use super::*;

impl Application {
    /// Apply the session `ELEV` token to one geometry about to be created.
    ///
    /// `elevation` is [`crate::session_state::DraftingExtras::elev`]; `0.0` is
    /// identity.
    pub(crate) fn elevate_new_geometry(
        geometry: SemanticGeometry,
        elevation: f64,
    ) -> CadResult<SemanticGeometry> {
        // A `Body3d` is an explicitly placed 3D object: its own Z is the
        // author's intent (the primitive commands take full 3D points), so the
        // session `ELEV` token — AutoCAD's "current elevation for new objects" —
        // never translates it. A `Mesh` is the same: `FOLLOWME` builds it in
        // world coordinates from a profile and a path, and offsetting it would
        // tear it away from those rows. Exempted here, in the one funnel, so no
        // create path can sneak an offset in.
        if matches!(
            &geometry,
            SemanticGeometry::Body3d(_) | SemanticGeometry::Mesh(_)
        ) {
            return Ok(geometry);
        }
        if elevation == 0.0 {
            return Ok(geometry);
        }
        if !elevation.is_finite() {
            // The setter refuses this, so a caller reaching here staged the token
            // itself; refuse rather than create a non-finite coordinate.
            return Err(CadError::InvalidInput(format!(
                "elev.invalid_value: the session ELEV token {elevation} is not finite, so no \
                 object can be created at that elevation"
            )));
        }
        // `Shape` is the one stored kind the generic transform refuses; a z
        // translation is exactly representable in its own fields.
        if let SemanticGeometry::Shape {
            shape_name,
            code,
            position,
            size,
            rotation,
            font,
        } = &geometry
        {
            let position = Point3 {
                x: position.x,
                y: position.y,
                z: position.z + elevation,
            };
            if !is_finite_point(position) {
                return Err(CadError::InvalidInput(format!(
                    "elev.invalid_geometry: the ELEV offset {elevation} overflows a created SHAPE \
                     position"
                )));
            }
            return Ok(SemanticGeometry::Shape {
                shape_name: shape_name.clone(),
                code: *code,
                position,
                size: *size,
                rotation: *rotation,
                font: font.clone(),
            });
        }
        let translation = Transform3::translation(Point3 {
            x: 0.0,
            y: 0.0,
            z: elevation,
        });
        cad_db::transform_geometry(&geometry, &translation).map_err(|error| match error {
            CadError::Unsupported(reason) => CadError::InvalidInput(format!(
                "elev.unsupported_kind: {} cannot be created at elevation {elevation}: {reason}",
                geometry_kind(&geometry)
            )),
            other => CadError::InvalidInput(format!(
                "elev.invalid_geometry: the ELEV offset {elevation} cannot be applied to a created \
                 {}: {other}",
                geometry_kind(&geometry)
            )),
        })
    }

    /// Apply the session `ELEV` token to a batch about to be created in one
    /// transaction (all-or-nothing: a refusal happens before the transaction).
    pub(crate) fn elevate_new_geometries(
        geometries: Vec<SemanticGeometry>,
        elevation: f64,
    ) -> CadResult<Vec<SemanticGeometry>> {
        if elevation == 0.0 {
            return Ok(geometries);
        }
        geometries
            .into_iter()
            .map(|geometry| Self::elevate_new_geometry(geometry, elevation))
            .collect()
    }
}
