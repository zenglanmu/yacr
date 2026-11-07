//! Long-form contract for [`crate::CommandId::MassProp`] (`MASSPROP`, AutoCAD
//! 2026 inquiry, 2026-11-07).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. Nothing here is
//! implemented in this module: the report lives in
//! `crate::app_measure_object` (`Application::mass_prop`) and the selection
//! checks reuse the existing measurement funnel.
//!
//! ## Input: the current selection (declared divergence)
//!
//! AutoCAD prompts `Select objects`. This build has one fixed capture machine
//! and already reads the **current selection** for the other selection-based
//! inquiry commands (`AREA OBJECT`, `LIST`, `MATCHPROP`), so `MASSPROP` does the
//! same: payload is [`crate::CommandPayload::None`], the session selection is the
//! whole input. An **empty** selection is the named
//! `massprop.selection` refusal. A non-empty payload is
//! `massprop.invalid_payload`; it is never silently ignored.
//!
//! This is a read-only command: it opens no transaction, publishes no
//! `ChangeSet`, records no `cad-history` entry and bumps no revision. The only
//! state it touches is the caller-visible diagnostic list. Viewer and Work both
//! allow it (it is out of `CommandId::requires_work_mode`).
//!
//! ## What v1 measures
//!
//! Only **closed, planar** objects, all on **one common plane**:
//!
//! * `Circle` — exact `π r²`, exact perimeter `2π r`, centroid = the stored
//!   centre, bounding box from the circle's own plane basis.
//! * full `Ellipse` — `|sweep| >= 2π − 1e-9` only. Exact `π a b` with
//!   `a = |major_axis|` and `b = a · ratio` (the stored ratio is honoured
//!   verbatim); centroid = the stored centre. The exact perimeter is an
//!   elliptic integral, which this build does not implement, so the reported
//!   value is **Ramanujan's second approximation** and is labelled
//!   `massprop.approx_ellipse_perimeter` (never presented as exact). An
//!   elliptical arc is `massprop.ellipse_arc`, not a whole ellipse.
//! * closed `Polyline`, any bulges, one plane — area through
//!   [`cad_geometry::measure_bulged_ring_area`] (the same exact bulge-aware,
//!   plane-aware routine `AREA OBJECT` uses), perimeter as the exact sum of
//!   straight chords and circular-arc lengths, centroid from the analytic
//!   straight-edge + circular-segment first moments. The ring's own plane comes
//!   from the area routine's `BulgedRingArea::plane`.
//!
//! Bounding boxes are **exact** for the supported shapes: a circle/ellipse uses
//! its plane basis, and a bulged polyline includes every arc's axis extremes
//! inside the segment sweep, not just the stored vertices.
//!
//! ## Refusals (nothing is reported; every code is stable)
//!
//! * `massprop.selection` — no selected object.
//! * `massprop.invalid_payload` — a payload other than `None`.
//! * `massprop.not_addressable` — a ref with an instance path or sub-element:
//!   block members and sub-elements have no addressable identity/geometry.
//! * `massprop.target_missing` — the ref names an entity that no longer exists
//!   (or belongs to another document).
//! * `massprop.open_polyline` — an open LWPOLYLINE encloses no area.
//! * `massprop.ellipse_arc` — a partial ellipse is not a closed region.
//! * `massprop.unsupported_kind` — every other kind, named in the message:
//!   `ARC`, `LINE`, `SPLINE`, `TEXT`, `INSERT`, `HATCH`, `DIMENSION`, `MESH`,
//!   `OPAQUE`, `COMPOUND`, `MULTILEADER`, `IMAGE`, `MASK`, `SHAPE`, `POINT`.
//!   There is no 3D-solid/region model in `cad-db`, so no solid is measured.
//! * `massprop.non_planar` — the polyline's own vertices do not lie in one
//!   plane within the computation tolerance.
//! * `massprop.not_coplanar` — two selected regions have different plane
//!   normals (up to sign) or a different plane offset. All regions must be
//!   coplanar, exactly like AutoCAD's region mass properties. The comparison
//!   uses the measurement computation tolerance.
//! * `massprop.degenerate` — a non-finite coordinate/normal/radius, a
//!   non-positive radius/axis, a zero area, or an overflow to a non-finite
//!   value. No value is clamped, rounded or invented.
//!
//! ## Report shape
//!
//! One `massprop.object` diagnostic per selected entity, carrying the entity id,
//! kind, exact **area**, **perimeter** (with an explicit `approximate` marker for
//! the ellipse), exact **bounding box** `min`/`max`, and **centroid**. One
//! `massprop.summary` diagnostic carries the object count, the summed area, the
//! summed perimeter (marked approximate when any member is), the union bounding
//! box and the area-weighted centroid of the per-object results. The summary is
//! the **area-weighted combination of the reported regions**, not a CSG union
//! (overlapping regions are counted once per selected object), which is the same
//! convention the per-object report exposes.
//!
//! A `massprop.not_implemented` diagnostic names the un-modelled AutoCAD half on
//! every run (moments of inertia, principal axes, volume/density, composition and
//! `.mpr` output), so the boundary is visible at run time as well as in these
//! docs.
//!
//! `CommandOutcome::measurement` / `point_report` stay `None`: `MASSPROP` reports
//! several properties and no single `MeasurementRecord` value can carry them
//! honestly.
//!
//! ## NOT IMPLEMENTED (named, never fabricated)
//!
//! Moments and products of inertia, radii of gyration, principal moments and
//! principal axes, mass/volume/3D solids, density, `Add`/`Subtract` region
//! composition, and writing the analysis to a `.mpr` file. There is no payload
//! field or command path for any of them.
