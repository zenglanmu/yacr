//! Grip model: which draggable handles an entity exposes, hit-testing one
//! handle, and the geometry a **single-handle drag** produces.
//!
//! This is the pure model only. There is deliberately **no** transaction, no
//! history entry, no renderer marker and no host arming here: the `GRIPSTRETCH`
//! **command** lives in `crate::app_drawing::grip` (it re-reads the addressed
//! entity, calls [`apply_grip`] and owns the single transaction / undo entry),
//! the marker is `crate::render_scene::grip_marker_overlay` (fed by the host),
//! and pointer routing / arming lives in `cad-host` (`input/grip_drag.rs`).
//! Everything in this module is a pure function over one entity's geometry (or,
//! for [`grip_hit`]/[`grip_hit_planar`], over a list of entities), so it can be
//! unit-tested without a drawing, a session or a GPU.
//!
//! ## A grip is not a snap point
//!
//! Object snap points (OSNAP, `cad-measure`'s
//! `SnapKind::{Endpoint, Midpoint, Center, …}`) answer "where may a **new**
//! point land?"; they are derived per geometry and only exist while a point is
//! being captured. A grip answers a different question: "which **existing**
//! handle of this entity may be dragged, and what geometry does dragging it
//! produce?". The two surfaces are unrelated and this module neither reuses nor
//! extends `SnapKind`: `SnapKind::Midpoint` is a snap point and
//! [`GripKind::Midpoint`] is a handle, and they are two separate values on two
//! separate surfaces. Nothing here reads, wraps or re-derives an OSNAP point.
//!
//! ## Supported handles (the whole implemented surface)
//!
//! | geometry | handles, in list order |
//! |---|---|
//! | `SemanticGeometry::Line` | [`GripKind::Endpoint`] `0` = `start`, `1` = `end`, then [`GripKind::Midpoint`] `2` |
//! | `SemanticGeometry::Circle` | [`GripKind::Center`] `0`, then [`GripKind::Quadrant`] `0..=3` |
//! | `SemanticGeometry::Arc` | [`GripKind::Center`] `0`, then [`GripKind::Endpoint`] `0` = start point, `1` = end point |
//! | `SemanticGeometry::Ellipse` | [`GripKind::Center`] `0`, then [`GripKind::Quadrant`] `0..=3` — **full ellipses only** (see below) |
//! | `SemanticGeometry::Polyline` | [`GripKind::Vertex`] `index` = stored vertex index |
//! | `SemanticGeometry::Spline` | [`GripKind::ControlPoint`] `index` = stored control-point index |
//! | `SemanticGeometry::Point` | [`GripKind::Point`] `0` |
//! | `SemanticGeometry::Text` | [`GripKind::Insertion`] `0` = the stored `position` (TEXT and MTEXT alike) |
//! | `SemanticGeometry::Insert` | [`GripKind::Insertion`] `0` = the placement's **insertion point** (the transformed block-space origin) |
//! | `SemanticGeometry::Dimension` | [`GripKind::DefinitionPoint`] `index` = the position in the per-subtype order below (the DXF definition points, then the text middle point) |
//!
//! The `ARC` order is `Center 0, Endpoint 0, Endpoint 1`: the centre comes
//! first because it was the only handle before the endpoints were added, and the
//! list order is the hit-test candidate order (see the tie-break rule), so
//! keeping it first preserves every tie the centre used to win.
//!
//! `POLYLINE` is the one semantic variant for **every** polyline class the
//! importer produces (LWPOLYLINE, 2D `AcDb2dPolyline`, legacy `AcDbPolyline` and
//! 3D `AcDb3dPolyline`): the class is not carried into [`SemanticGeometry`], so
//! this model cannot single out the 3D form and vertex grips therefore apply to
//! all of them. Distinguishing (and refusing) the 3D class is NOT IMPLEMENTED
//! and would need a domain change, not a guess here. A vertex drag only moves
//! x/y and keeps the vertex's own `z`, exactly like a `LINE` endpoint drag, so
//! it never edits a 3D polyline's elevation.
//!
//! `SPLINE` is a NURBS definition, and its control points are **not on the
//! curve**: a B-spline (rational or not) passes through its first and last control
//! point only when the end knots have full multiplicity, and never through the
//! interior ones. That is not a bug and not an approximation: the
//! stored `control_points` are the definition, the drawn curve is the
//! de-Boor/NURBS evaluation of `(degree, knots, control_points, weights)`. A
//! control-point grip therefore sits at the stored control point (a marker off
//! the drawn curve is correct), and a drag that redefines one of them **reshapes**
//! the curve — after such a drag the curve no longer passes through the points it
//! used to pass through. No handle names a point **on** the drawn curve: this
//! model exposes the definition points only, not the curve's endpoints (see the
//! drag rules). We do not claim vendor equivalence for the preview/reshape feel;
//! AutoCAD also drags control points without re-fitting, but that is a
//! resemblance, not a tested claim.
//!
//! Only control-point indices `0..=255` are addressable because [`GripId::index`]
//! is a `u8`: a spline with more than 256 stored control points exposes the first
//! 256 (exactly like a [`GripKind::Vertex`] polyline) and the rest are simply
//! unreachable in this build. Nothing is wrapped, truncated or renumbered.
//!
//! A spline whose stored definition is **internally inconsistent** —
//! `control_points.len() != weights.len()`, or
//! `knots.len() != control_points.len() + degree + 1` — exposes **no** grips at
//! all, and every [`GripKind::ControlPoint`] id against it is
//! [`REASON_STALE_HANDLE`]. We do not guess a repaired definition (padding a knot
//! vector or a weight list would silently change the curve), and we do not
//! validate the shape either: `cad-db`'s validator is the authority on what may be
//! stored. This model only declines to invent a handle for a definition it cannot
//! interpret.
//!
//! `TEXT` and `MTEXT` are the **same** semantic variant
//! (`SemanticGeometry::Text`); only `DbEntity.object.type_key` distinguishes the
//! DXF class (`AcDbText` vs `AcDbMText`), so this model cannot — and does not —
//! give them different handles. Both expose exactly one [`GripKind::Insertion`]
//! grip at the **stored insertion point**: the stored `position`, verbatim. The
//! handle is **not** offset by `h_align`/`v_align` (a centre/right or middle/top
//! attachment does not move it), is **not** rotated about that point by
//! `rotation`, and does not depend on `height`/`wrap_width`, so an alignment
//! change or a rotation never moves it and a drag moves nothing but `position`.
//! `MULTILEADER` (`SemanticGeometry::MultiLeader`) is a **different** variant and
//! stays on the refusal list: its annotation text is not exposed through a text
//! insertion-point handle in this build, so it has no grip at all.
//!
//! `INSERT` (a block reference) exposes exactly **one** [`GripKind::Insertion`]
//! grip, `index` 0, at the placement's **insertion point**. The stored
//! `SemanticGeometry::Insert { block, transform }` carries the reference and a
//! 4x4 affine `Transform3` whose point mapping has no homogeneous divide
//! (`Transform3::apply_point`), and the importer folds the block's base point into
//! that transform, so the block-space origin lands exactly on the `INSERT`'s
//! insertion point. The handle is therefore `transform * (0, 0, 0)` — the
//! translation column of the matrix, which is the one existing convention for "the
//! insertion point of an insert" in this repo (`cad-import-acadrust`'s
//! `insert_array_transform` / `block_placement`); no second convention is invented
//! here. The handle is the *origin's image*, not "the translation as a point":
//! those are the same `f64` bits, and the contract test pins that a change to the
//! rotation/scale part with an unchanged translation does not move the handle.
//!
//! A block reference is grip-able only as a **top-level model-space entity**. A
//! block *member* — and any attribute (`ATTRIB`) or point inside a block — is
//! reached through an instance path (`SelectionRef::instance`), which a
//! [`GripId`] cannot even express; `GRIPSTRETCH` already refuses a non-model-space
//! target with `property.not_addressable`. This model adds **no second**
//! addressability rule: it describes the geometry it is handed. Grips for block
//! members, for attributes and for points inside a block are **NOT IMPLEMENTED**.
//!
//! ### `DIMENSION` definition points
//!
//! A dimension exposes one [`GripKind::DefinitionPoint`] grip per stored DXF
//! definition point, in the documented per-subtype order, plus **one final**
//! handle for the stored text middle point (group 11). `index` is the position in
//! that order, so the handle set is fully described by the stored definition:
//!
//! | subtype | `index` → slot |
//! |---|---|
//! | `Linear { rotation }` and `Aligned` | `0` `first` (13), `1` `second` (14), `2` `dimension_line` (10), `3` `text_middle` (11) |
//! | `Radius` / `Diameter` (`DimensionPoints::Radial`) | `0` `center` (10), `1` `point_on_arc` (15), `2` `text_middle` (11) |
//! | `Angular2Ln` | `0` `line1_start` (13), `1` `line1_end` (14), `2` `line2_start` (15), `3` `line2_end` (10), `4` `arc_point` (16), `5` `text_middle` (11) |
//! | `Angular3Pt` | `0` `vertex` (15), `1` `first` (13), `2` `second` (14), `3` `arc_point` (10), `4` `text_middle` (11) |
//! | `Ordinate { x_ordinate }` | `0` `feature` (13), `1` `leader_end` (14), `2` `base` (10), `3` `text_middle` (11) |
//!
//! `text_middle` is always the last index, i.e. the subtype's definition-point
//! count, so it can never be confused with a point slot. Every handle position is
//! the **stored** point verbatim (bit for bit, all three coordinates): nothing is
//! derived, offset or projected, so the marker sits exactly on the stored DXF slot
//! even when the drawn display puts the text elsewhere (a stored `text_middle` of
//! `(0, 0, 0)` means "derive the text position", and the handle still marks the
//! stored zero slot — it does not pretend to be the drawn text anchor).
//!
//! **Not a grip**: `leader_length` (group 40, a scalar), `text_rotation` (group
//! 53, a scalar), every resolved style value (`DimensionStyleValues`: arrow size,
//! text height/gap, extension-line offsets, `DIMLFAC`/`DIMDEC`/`DIMPOST`/
//! `DIMZIN`/`DIMDSEP`, …), the subtype scalars (`Linear { rotation }`,
//! `Ordinate { x_ordinate }`) and the extrusion `normal`. They are carried through
//! every drag verbatim; a length/rotation/style grip is **NOT IMPLEMENTED**, and no
//! [`GripKind`] variant is invented for one.
//!
//! A `DefinitionPoint` variant (rather than reusing [`GripKind::Endpoint`] or
//! [`GripKind::Insertion`]) is deliberate: a dimension's handles *are* its DXF
//! definition points plus the text middle point, and sharing another kind would
//! leave [`REASON_STALE_HANDLE`] unable to tell a dimension's handle from a
//! `LINE`'s endpoint or a text's insertion point.
//!
//! [`grips_for_entity`] returns an **empty** vector for every other geometry, and
//! [`grip_hit`] therefore never hits one. No marker is invented, no index is
//! guessed and no geometry is silently translated instead: an entity with no
//! grips is simply not grip-editable in this build.
//!
//! ### Drag rules
//!
//! * **`LINE` endpoint** (`index` 0/1): the dragged endpoint takes the target and
//!   the other endpoint is returned **bit-identically**.
//! * **`LINE` midpoint** (`index` 2): the whole line **translates**. With
//!   `delta = to - midpoint(start, end)` both endpoints move by that same vector,
//!   so the line stays parallel to itself and its length is unchanged; nothing
//!   is rotated, scaled or snapped. The *reported* handle position is the exact
//!   f64 midpoint `(start + end) * 0.5` of the two stored endpoints (no rounding
//!   to a grid).
//! * **`CIRCLE` centre** (`index` 0): the centre becomes the target; `radius` and
//!   `normal` are returned bit-identically.
//! * **`CIRCLE` quadrant** (`index` `0..=3`, documented order: `0` = `+ax`,
//!   `1` = `+ay`, `2` = `-ax`, `3` = `-ay` of the circle's **own** plane frame):
//!   the **radius** becomes the distance from the centre to the target
//!   **projected into the circle's plane**, while the centre and the normal stay
//!   put (both bit-identically). It is therefore never a 3D distance: a target
//!   `h` off the plane gives the in-plane distance, not `sqrt(in_plane^2 + h^2)`,
//!   so a drag can never tilt a circle into an ellipse. A drag that would give a
//!   zero or non-finite radius is refused by name
//!   ([`REASON_DEGENERATE_RADIUS`]); it is **never clamped** to a minimum.
//! * **`ARC` centre** (`index` 0): the arc moves **rigidly** — `center` takes the
//!   target and `radius` / `normal` / `start` / `sweep` are bit-identical, so the
//!   radius and the angular extent are untouched.
//! * **`ARC` endpoint** (`index` 0 = start point, `1` = end point): the arc stays
//!   on the **same circle** — `center`, `radius` and `normal` are bit-identical —
//!   and only the dragged endpoint's angle changes. That angle becomes the
//!   in-plane direction of the target from the centre (the out-of-plane offset,
//!   including the target's `z`, is ignored: the arc's plane is fixed by its own
//!   normal). The **opposite endpoint keeps its angle**, so the end angle is the
//!   stored `start + sweep` when the start is dragged, and the stored `start`
//!   when the end is dragged. The sweep is then **recomputed in the stored
//!   direction** (positive for a counter-clockwise arc, negative for a
//!   clockwise/reflected one) as the in-plane extent between the two angles, and
//!   must stay in the **open** interval `(0, 2π)` in that direction.
//!
//!   A target that collapses the arc — a recomputed sweep within
//!   [`ARC_SWEEP_EPSILON`] (`1e-9` rad, the same slack the tessellator and the
//!   representation provider use to call a sweep "full") of `0` or `2π` — is
//!   refused by name ([`REASON_DEGENERATE_ARC`]) and **never clamped** into the
//!   open interval. So is a target with no in-plane direction at all (the centre
//!   itself, or a point on the normal through it). **This endpoint rule is our
//!   choice, not an AutoCAD claim**: it is what keeps a single-handle drag from
//!   tilting the arc, and no vendor behaviour is asserted by it.
//! * **`ELLIPSE` centre** (`index` 0): the ellipse moves **rigidly** — `center`
//!   takes the target and `normal` / `major_axis` / `ratio` / `start` / `sweep`
//!   are returned bit-identically.
//! * **`ELLIPSE` quadrant** (`index` `0..=3`, documented order in the ellipse's
//!   own stored frame: `0` = `+major`, `1` = `+minor`, `2` = `-major`,
//!   `3` = `-minor`, i.e. the same `+, +, -, -` ordering as the circle
//!   quadrants): the drag sets the **corresponding radius** to the distance from
//!   the centre to the target measured **in the ellipse's own plane** (never a
//!   3D distance and never an x/y distance). `center` and `normal` are
//!   bit-identical. Which stored field carries that radius depends on the
//!   handle: the ellipse stores its major semi-axis **as the `major_axis`
//!   vector** (`|major_axis|` is the major radius) and its minor radius as
//!   `|major_axis| * ratio`, so
//!   * a **major** quadrant (`0`/`2`) sets `|major_axis|` to the measured
//!     distance and keeps the stored **minor radius** (`major_len * |ratio|`)
//!     unchanged, which recomputes `ratio = minor_len / new_major_len`;
//!   * a **minor** quadrant (`1`/`3`) sets the minor radius to the measured
//!     distance, i.e. `ratio = distance / |major_axis|`, and leaves
//!     `major_axis` bit-identical.
//!
//!   **Axis direction never rotates** (our choice, **not an AutoCAD claim**): a
//!   major drag keeps the stored major direction and only changes its length,
//!   so `major_axis` is the stored vector scaled by
//!   `distance / |major_axis|`. This is the rule that cannot produce a
//!   degenerate ellipse (the direction is never re-derived from a target that
//!   might sit on the centre, and the two axes stay orthogonal by construction),
//!   and it is the same "one handle, one radius" shape as the circle quadrant
//!   rule. A target off the major axis therefore still sets the major **radius**
//!   from its full in-plane distance; it does not turn the ellipse.
//!
//!   The domain model has **no** `minor <= major` ordering: `cad-db` validation
//!   only requires a positive finite `ratio`, and the tessellator and
//!   representation provider draw `|major_axis| * |ratio|` as the minor
//!   semi-axis, so `ratio > 1` is a representable, correctly drawn ellipse whose
//!   stored axis roles are simply swapped. A drag that pushes one radius past
//!   the other is therefore **not** refused (there is no invariant to break) and
//!   is **not** silently re-normalized (swapping the axes would rotate the
//!   stored frame and change the parameterisation behind the user's back). Only
//!   the create path (`draw_tool::ellipse_parameters`) happens to keep
//!   `ratio <= 1`; that is a storage assumption of the axis-endpoint method, not
//!   a grip invariant.
//!
//!   A zero or non-finite in-plane distance is refused by name
//!   ([`REASON_DEGENERATE_RADIUS`]) and **never clamped**; so is a stored frame
//!   from which no in-plane radius can be derived (non-finite/zero major axis or
//!   a normal parallel to it). A drag that leaves the dragged radius at its
//!   stored length is [`REASON_ZERO_DISPLACEMENT`], exactly like the circle rule
//!   (a target off the plane at the same in-plane distance has no geometric
//!   effect).
//! * **`ELLIPSE` partial (elliptical arc)**: `SemanticGeometry::Ellipse` carries
//!   a `start`/`sweep` parameter range, and a range that is **not** a full turn
//!   is an elliptical arc. Such an entity exposes the **centre grip only**;
//!   quadrant drags are refused by name ([`REASON_PARTIAL_ELLIPSE`], **our
//!   choice, not an AutoCAD claim**). The four axis points are stored-parameter
//!   quadrant points, and on a partial ellipse some of them lie outside the
//!   drawn range — reshaping the arc from a handle that is not on the drawn curve
//!   would be a silent reinterpretation of the stored parameter range, so we do
//!   not offer it. The **centre** drag stays available because it changes no
//!   radius and no parameter, so the arc's `start`/`sweep` remain meaningful
//!   bit-for-bit. "Full turn" uses the same slack as the tessellator and the
//!   representation provider (`|sweep| >= 2π - 1e-9`), so the handle set and the
//!   drawn curve always agree about which ellipses are complete.
//! * **`POLYLINE` vertex** (`index` = stored vertex index): the dragged vertex
//!   takes `x`/`y` from the target and keeps its own stored `z`, so the drag
//!   never changes an elevation (3D vertex editing is NOT IMPLEMENTED). Every
//!   other vertex is returned **bit-identically**, and `closed` plus the whole
//!   `bulges` vector are copied verbatim. A bulge is therefore **never
//!   recomputed** for the moved vertex: the segment entering and the segment
//!   leaving it keep the bulge they were stored with (our reading of how the
//!   handle behaves — **not vendor-verified**). An `index` outside the stored
//!   vertices is [`REASON_STALE_HANDLE`].
//! * **`SPLINE` control point** (`index` = stored control-point index): the
//!   dragged entry of `control_points` takes the target **verbatim** (all three
//!   coordinates, no projection: a control point is a free 3D definition point,
//!   not a point on a plane) and `degree`, `knots` and `weights` are returned
//!   **bit-identically**. No knot and no weight is recomputed, renormalised or
//!   re-fitted: the result is a **different NURBS** that happens to share the
//!   parameterisation — that is our choice (AutoCAD also drags control points
//!   without re-fitting, but no vendor equivalence is claimed), and it is the
//!   reason a control-point drag changes the shape of the whole curve rather than
//!   moving a point along it. Every other control point is bit-identical. An
//!   `index` outside the stored control points — and any id against an
//!   internally inconsistent definition — is [`REASON_STALE_HANDLE`].
//! * **`POINT`**: the position becomes the target.
//! * **`TEXT`/`MTEXT` insertion point** (`index` 0): the stored `position`
//!   becomes the target **verbatim** — all three coordinates, no projection and
//!   no alignment/rotation offset — and `text`, `style`, `height`, `rotation`,
//!   `font`, `h_align`, `v_align` and `wrap_width` are returned
//!   **bit-identically**, including `wrap_width: Some(w)` (a plain `f64` that is
//!   copied, never recomputed). Because the handle is the **stored** insertion
//!   point, a `Center`/`Right` or `Middle`/`Top` attachment and a non-zero
//!   `rotation` do not move it, and a drag does not change them. A rotation,
//!   height, wrap-width or attachment (alignment) grip is **NOT IMPLEMENTED**:
//!   the insertion point is the only text handle in this build.
//! * **`INSERT` insertion point** (`index` 0): the block reference moves
//!   **rigidly** — the drag keeps the whole 3x3 linear part of the transform
//!   (rotation, non-uniform scale, mirror, shear: all nine coefficients) and the
//!   `block` reference **bit-identically**, and changes only the translation
//!   column so that the insertion point becomes the target. Concretely the new
//!   matrix is the stored one with `m[0][3] = to.x`, `m[1][3] = to.y`,
//!   `m[2][3] = to.z`; rows 0..2 keep every other coefficient and row 3 is
//!   untouched. That is the *origin's image* rule: a placement maps the
//!   block-space origin to its translation column, so assigning the target there
//!   makes the handle land on the target **exactly** (no solve, no inverse, no
//!   residual error). A drag therefore never rotates, scales, mirrors or shears a
//!   block reference — it only moves it — and every block-space point keeps its
//!   offset from the insertion point. Nothing is snapped and nothing is
//!   projected: the target is stored in all three coordinates.
//!
//!   Because no matrix is inverted, a **singular** (but finite) linear part is
//!   not refused: translating a flattened instance is still exact. A stored
//!   transform with a **non-finite** coefficient is refused by name
//!   ([`REASON_DEGENERATE_TRANSFORM`]) instead of being written back — `cad-db`
//!   refuses such an insert transform at commit, so staging it would only move the
//!   failure later.
//! * **`DIMENSION` definition point / text middle point**
//!   ([`GripKind::DefinitionPoint`], `index` = the position in the per-subtype
//!   order above): the rule clones the stored [`DimensionDefinition`], replaces
//!   **exactly that one point** and **re-synthesizes** the entity with
//!   [`DimensionGeometry::synthesized`], passing the entity's own `style_name`,
//!   `style` and `text_override` — so `kind`, `text_rotation`, `normal`,
//!   `style_name`, `style` and the override string are preserved, and every point
//!   the drag did not address (plus `leader_length`) is bit-identical. The
//!   measurement and the whole display are **recomputed** from the mutated
//!   definition by `cad-domain`'s single builder; `cad-app` never patches a
//!   dimension child by hand, which is what keeps group 42 and the drawn number in
//!   step. Dragging the text middle point therefore moves the stored group-11
//!   anchor, recomputes the display around it and leaves the measurement alone.
//!
//!   Refusals, all before anything is written: a subtype/point-set mismatch is
//!   [`REASON_DIMENSION_INCONSISTENT`] (a bug signal — the public constructors
//!   cannot produce one), and a mutation the synthesis cannot express is
//!   [`REASON_DIMENSION_DEGENERATE`] (a `Radial` `point_on_arc` dragged onto its
//!   centre, two angular lines driven parallel, an `Angular3Pt` `arc_point` on the
//!   vertex, …). A non-finite measurement or an empty display is refused with the
//!   same code: a `NaN` group 42 is never stored and an empty display is never
//!   emitted. Nothing is clamped. Declared limitation: `cad-domain` keeps
//!   `DisplaySource` private, so a drag cannot distinguish the synthesized branch
//!   from the imported-picture branch and always re-synthesizes — a picture-branch
//!   dimension becomes a synthesized one whose measurement matches its mutated
//!   definition (see `grips/dimension.rs`).
//!
//! ## Explicit refusal list (no marker, no hit)
//!
//! `MULTILEADER` (its annotation text is not addressed through a text
//! insertion-point handle here), `HATCH`, `IMAGE`/`MASK`, and
//! opaque/compound geometry (`Opaque`, `Compound`, `Mesh`, `Shape`). `TEXT`/
//! `MTEXT` is **no longer** on this list: it exposes one [`GripKind::Insertion`]
//! grip at the stored insertion point (see above). **`INSERT` is no longer on this
//! list either**: a block reference exposes one [`GripKind::Insertion`] grip at its
//! insertion point and drags rigidly (see above) — but grips for block **members**
//! (instance paths), for block **attributes** and for points **inside** a block are
//! **NOT IMPLEMENTED**. `ELLIPSE` is **no longer** on
//! this list: a full ellipse exposes centre + quadrant handles (see the drag
//! rules), and a partial (elliptical-arc) ellipse exposes the centre grip only,
//! with quadrant ids refused by [`REASON_PARTIAL_ELLIPSE`]. `SPLINE` is **no
//! longer** on this list either: it exposes one [`GripKind::ControlPoint`] per
//! stored control point (see above). **`DIMENSION` is no longer on this list
//! either**: it exposes one [`GripKind::DefinitionPoint`] grip per stored DXF
//! definition point plus one for the text middle point (see above), and a drag
//! re-synthesizes the display from the mutated definition. The dimension stays
//! **non-associative** by design — moving the geometry it measures does not move
//! it — which is a property of the entity, not of its handle set.
//!
//! A **width**, **rotation**, **height**, **attachment** (alignment) and
//! **tangent** grip are **NOT IMPLEMENTED**.
//! They are not modelled as [`GripKind`] variants either: a variant that always
//! yields an empty vector or an error would be a placeholder pretending to be a
//! capability. A later lane that implements one of them adds its variant **and**
//! its drag rule together (as this lane did for [`GripKind::Vertex`] and, for the
//! stored text insertion point and the block-reference insertion point,
//! [`GripKind::Insertion`]).
//!
//! ## Coordinates: 3D-exact, never projected (except the three in-plane rules)
//!
//! A handle's [`Grip::position`] is a stored or stored-derived point — the same
//! `f64` bits as the geometry (an endpoint, a point position, a circle/arc/ellipse
//! centre, a polyline vertex, a spline control point, a text insertion point, a
//! block reference's insertion point, i.e. the transformed origin — the
//! translation column of its placement — a dimension's DXF definition point or its
//! text middle point) or
//! the documented exact derivation of it (a line midpoint; an arc endpoint, i.e.
//! the stored
//! `center + radius * (cos angle * ax + sin angle * ay)` in the arc's own plane
//! frame, the same expression the tessellator uses; an ellipse quadrant, i.e.
//! `center ± major_axis` and `center ± ` the stored minor semi-axis vector in the
//! ellipse's own frame, again the tessellator's own expression). Nothing is
//! rounded and nothing is snapped.
//!
//! A drag assigns the target point to the dragged handle in **all three**
//! coordinates; nothing is projected onto a work plane or onto Z = 0. The caller
//! resolves the pointer into the world point it means — the host (`cad-host`'s
//! `input/grip_drag.rs`) resolves it in the plan view (x/y from
//! `screen_to_world`, plus the dragged handle's own `z`, so a plan-view drag
//! never changes an elevation) — and this module stores whatever finite target it
//! is given. A non-finite target is refused (see below) instead of stored.
//!
//! The **three** projections this module performs are the circle-quadrant radius,
//! the arc-endpoint angle and the ellipse-quadrant radius: all are measured in
//! the entity's **own** plane, precisely so that a non-planar drag cannot deform a
//! circle, tilt an arc out of its plane, or turn an ellipse. Those projections
//! apply to the *drag target of a quadrant or an arc-endpoint handle* only; a
//! centre drag, an arc-rigid drag, a line drag, a polyline vertex drag, a spline
//! control-point drag, a point drag and a text insertion-point drag are stored
//! 3D-exact (a polyline vertex keeps its own `z`), and no handle position is ever
//! projected back onto a plane.
//!
//! ## The plane frame is shared, not invented
//!
//! Quadrant handle positions of a **circle**, the circle in-plane radius, and the
//! arc endpoint positions and angles use `cad_geometry::arbitrary_axis` — the
//! AutoCAD arbitrary-axis-algorithm frame the tessellator, representation
//! provider, intersect/transform code and the importer already measure circle/arc
//! angles in. It returns `(ax, ay, unit normal)`, with `ax`/`ay` orthonormal and
//! inside the circle plane, so `dot(to - center, ax)` and `dot(to - center, ay)`
//! are the exact in-plane coordinates of a displacement, and
//! `atan2(dot(…, ay), dot(…, ax))` is the exact in-plane angle of a direction.
//!
//! An **ellipse** has no free choice of frame: its quadrant handles are its own
//! stored axes, so they use the ellipse frame the tessellator and the
//! representation provider draw with — `(unit(major_axis),
//! normalize(cross(unit(normal), unit(major_axis))))` — which is the expression
//! `cad_geometry`'s (crate-private) `ellipse_minor_dir` evaluates. Feeding
//! `arbitrary_axis` here would put the "quadrant" handles off the drawn curve
//! whenever the stored major axis is not aligned with the arbitrary frame, so the
//! ellipse reuses the stored-axis convention instead, and it does **not** invent a
//! second one: there is no new angle convention, no re-normalised axis and no
//! rotated frame in this module.
//!
//! ## Hit-testing rule
//!
//! [`grip_hit`] returns the **closest** handle whose Euclidean world distance to
//! `world` is `<= tolerance`. Candidates are visited in a fixed order — the
//! entities in the order the caller supplied them, and within one entity its
//! grips in list order (see the table: for a `LINE` `start`, `end`, midpoint; for
//! a `CIRCLE` the centre and then quadrants 0..=3; for an `ARC` the centre and
//! then endpoint 0 = start, endpoint 1 = end; for an `ELLIPSE` the centre and then
//! quadrants 0..=3 when it is a full ellipse; for a `POLYLINE` the vertices in
//! stored order; for a `SPLINE` the control points in stored order; for a
//! `TEXT`/`MTEXT` the single insertion point; for a `DIMENSION` its definition
//! points in the per-subtype order above, ending with the text middle point) — and a
//! candidate replaces the current winner only when it is
//! **strictly** closer.
//!
//! [`grip_hit_planar`] is the **plan-view** variant the host uses: the same
//! ordering, tie-break and finiteness refusals, but the distance is measured in
//! x/y only. A pointer resolves through the viewport's `screen_to_world`, which
//! returns a point on the work plane (`z = work_plane.origin.z`); measuring 3D
//! distance against that point would miss every grip whose own `z` differs, so
//! a line at another elevation would be un-grippable in the very view that shows
//! it. Ignoring `z` is the plan-view rule, **not** 3D grip editing: moving a
//! handle in `z` is NOT IMPLEMENTED and the host keeps the dragged handle's own
//! `z` in the target it commits.
//!
//! **Tie-break rule:** on an exact distance tie the winner is the **first**
//! candidate in that iteration order, i.e. the earliest entity of the input and,
//! within one entity, the earlier entry of the list — which is the lower `index`
//! inside one kind, and the earlier kind in the table's order (`Endpoint` before
//! `Midpoint` for a `LINE`, `Center` before every `Quadrant` for a `CIRCLE` or a
//! full `ELLIPSE`, `Center` before `Endpoint` `0` before `Endpoint` `1` for an
//! `ARC`). For a
//! `POLYLINE` the list is the stored vertex order, so the lower vertex index wins
//! a tie; for a `SPLINE` it is the stored control-point order, so the lower
//! control-point index wins a tie and a query exactly at a control point is that
//! control point's hit (it is strictly closer than every other one). This is
//! deterministic and is pinned by contract tests. Note that the
//! midpoint is usually *nearer* than the endpoints, so a query at the midpoint is
//! a midpoint hit, not a tie: a tie between `start` and the midpoint needs a
//! query at a quarter of the line's length.
//!
//! A non-finite `world`, a non-finite or negative `tolerance`, and a stored
//! position at a non-finite coordinate never hit (nothing is fabricated from a
//! `NaN` comparison).
//!
//! ## Refusal reasons (English, stable, machine-matched)
//!
//! [`apply_grip`] refuses with an existing `CadError` variant whose message
//! **starts with** one of these codes:
//!
//! * [`REASON_UNSUPPORTED_KIND`] (`CadError::Unsupported`) — the geometry kind
//!   is on the refusal list above and exposes no grip model at all. The message
//!   names the DXF class.
//! * [`REASON_STALE_HANDLE`] (`CadError::InvalidInput`) — the [`GripId`] does
//!   not name a handle of **this** geometry: a `LINE` with a
//!   [`GripKind::Center`]/[`GripKind::Quadrant`]/[`GripKind::Point`]/[`GripKind::Vertex`]/[`GripKind::ControlPoint`]/[`GripKind::Insertion`]/[`GripKind::DefinitionPoint`]
//!   grip, an `AcDbCircle`/`AcDbArc` with a [`GripKind::Vertex`] grip, a
//!   `POLYLINE` with a non-vertex grip, a `SPLINE` with a non-control-point grip,
//!   a `TEXT`/`MTEXT` with a non-[`GripKind::Insertion`] grip, a block reference
//!   with a non-[`GripKind::Insertion`] grip, a `DIMENSION` with a
//!   non-[`GripKind::DefinitionPoint`] grip,
//!   or an `index` outside the handle range
//!   (`LINE` endpoint > 1, `LINE` midpoint ≠ 2, `CIRCLE`/`ELLIPSE` centre > 0,
//!   `CIRCLE`/`ELLIPSE` quadrant > 3, `ARC` centre > 0, `ARC` endpoint > 1,
//!   `POLYLINE` vertex ≥ the stored vertex count,
//!   `SPLINE` control point ≥ the stored control-point count, `POINT` > 0,
//!   `TEXT`/`MTEXT` insertion point > 0, block-reference insertion point > 0,
//!   `DIMENSION` definition point ≥ this subtype's handle count). It is
//!   also the refusal for **every** control-point id on an internally
//!   inconsistent spline definition, because such a spline exposes no handle at
//!   all. The caller is
//!   never trusted to have kept its id in sync with the geometry.
//! * [`REASON_ZERO_DISPLACEMENT`] (`CadError::InvalidInput`) — the drag would not
//!   change the geometry, so there is nothing to write. That is the target
//!   equalling the dragged handle's current position (endpoint, arc endpoint,
//!   point, midpoint, centre, quadrant, polyline vertex, spline control point,
//!   text insertion point, block-reference insertion point),
//!   and additionally:
//!   a quadrant target that already sits at the **stored** radius (its
//!   out-of-plane offset is ignored by the radius rule), an arc endpoint drag
//!   whose recomputed start/sweep are identical to the stored ones (an
//!   off-plane target along the same ray, which the angle rule ignores), an
//!   ellipse-quadrant drag whose measured distance equals the stored radius of
//!   that handle (the axis direction is fixed, so such a target changes nothing),
//!   and a
//!   vertex target whose x/y equal the stored vertex (the drag keeps the stored
//!   `z`, so a z-only difference changes nothing). It is an error, not a silent
//!   `Ok`: the command must skip the transaction (and therefore the undo entry).
//!   Equality is IEEE `==` (so `-0.0` equals `0.0`, and no tolerance is applied —
//!   a sub-tolerance drag is a real, tiny edit).
//! * [`REASON_DEGENERATE_RADIUS`] (`CadError::InvalidInput`) — a circle- or
//!   ellipse-quadrant drag whose in-plane distance from the centre is zero or
//!   non-finite (the target is the centre, or the target lies on the plane normal
//!   through it), or an ellipse-quadrant drag on a stored frame no in-plane radius
//!   can be derived from (non-finite/zero major axis, or a normal parallel to it).
//!   A radius cannot be `0` or `NaN`, and this model never clamps: it refuses by
//!   name.
//! * [`REASON_DEGENERATE_ARC`] (`CadError::InvalidInput`) — an arc-endpoint drag
//!   that has no in-plane direction (target at the centre or on the normal
//!   through it), that produces a non-finite sweep, or whose sweep lands within
//!   [`ARC_SWEEP_EPSILON`] of `0` or `2π`. The arc must keep an extent strictly
//!   inside `(0, 2π)`; the sweep is **refused by name, never clamped**.
//! * [`REASON_PARTIAL_ELLIPSE`] (`CadError::InvalidInput`) — an ellipse-quadrant
//!   id on a **partial** ellipse (one whose stored `|sweep|` is not a full turn).
//!   Such an entity exposes the centre grip only; the refusal names the reason
//!   instead of pretending the handle is stale, and it is checked in the handle
//!   identity position (before the target's finiteness), because no quadrant
//!   handle of that geometry exists to be validated.
//! * [`REASON_DEGENERATE_TRANSFORM`] (`CadError::InvalidInput`) — a block-reference
//!   insertion-point drag on a stored `Transform3` that carries a **non-finite**
//!   coefficient. The rigid rule keeps that coefficient (it only rewrites the
//!   translation column), and `cad-db` refuses a non-finite insert transform at
//!   commit, so writing it back would move the failure later and hide the cause. A
//!   **singular** (but finite) linear part is deliberately **not** refused: the
//!   rule needs no matrix inverse or decomposition, so a flattened instance still
//!   translates exactly and its linear part stays bit-identical.
//! * [`REASON_DIMENSION_INCONSISTENT`] (`CadError::InvalidInput`) — a
//!   `DIMENSION` whose [`DimensionDefinition::is_consistent`] fails: its subtype
//!   and point slots disagree, or a stored point is non-finite. Such a dimension
//!   exposes no handle at all (see `grips/dimension.rs`); both public constructors
//!   refuse such a definition, so reaching this refusal through a stored entity is
//!   a **bug signal**, and it is also what a mutated definition that stopped being
//!   consistent would report (replacing one point in place cannot do that).
//! * [`REASON_DIMENSION_DEGENERATE`] (`CadError::InvalidInput`) — a `DIMENSION`
//!   definition-point drag whose mutated definition no longer has a synthesizable
//!   display (a `Radial` `point_on_arc` moved onto the centre, an `Angular3Pt`
//!   `arc_point` moved onto the vertex, two angular lines driven parallel, an
//!   angular sector collapsed to `0` or `2π`, …), or whose synthesis would produce
//!   a non-finite measurement or an empty display. Refused by name, **never
//!   clamped**: no `NaN` group 42 and no empty display is ever stored.
//! * [`REASON_NON_FINITE_TARGET`] (`CadError::InvalidInput`) — the target has a
//!   `NaN` or infinite coordinate. Storing it would corrupt the entity, and the
//!   database refuses it at commit anyway; refusing here names the cause. The
//!   same code covers a midpoint translation that overflows `f64` (the inputs are
//!   finite but the translated endpoints are not), because the *result* would be
//!   an unstorable coordinate.
//!
//! The checks run in a fixed order: geometry kind, then handle identity (which
//! includes "this kind has no handle on this geometry at all", such as a
//! quadrant id on a partial ellipse), then the stored-geometry sanity the drag
//! rule needs (an internally inconsistent spline definition, a block reference's
//! non-finite placement), then target finiteness, then the drag's own
//! geometric refusals. "Already at the
//! handle" is checked before a drag's geometric rule, so a drag onto the handle
//! itself reports [`REASON_ZERO_DISPLACEMENT`] rather than a degenerate radius or
//! arc. An unsupported kind is therefore reported as
//! [`REASON_UNSUPPORTED_KIND`] even when the id would also be stale (there is no
//! handle of that kind to be stale *against*), while a supported kind with a
//! mismatched id is always [`REASON_STALE_HANDLE`].
//!
//! ## Command-level reason (the one code this module does not raise itself)
//!
//! [`REASON_TARGET_MISSING`] belongs to the same `grip.*` vocabulary but is
//! raised by the command (`crate::app_drawing::grip`), never by this module: a
//! [`GripId`] names an entity, and this module only ever sees one entity's
//! geometry, so it cannot report that the entity is gone. It is declared here so
//! every `grip.*` code has one authority.

use cad_db::DbEntity;
use cad_domain::{EntityId, Point3};

mod dimension;
mod drag;
mod handles;
mod reasons;
pub use drag::apply_grip;
pub use handles::grips_for_entity;
pub(crate) use reasons::finite_target;

/// Stable reason code: the geometry kind exposes no grip model.
pub const REASON_UNSUPPORTED_KIND: &str = "grip.unsupported_kind";
/// Stable reason code: the [`GripId`] does not name a handle of this geometry.
pub const REASON_STALE_HANDLE: &str = "grip.stale_handle";
/// Stable reason code: the drag would not change the geometry.
pub const REASON_ZERO_DISPLACEMENT: &str = "grip.zero_displacement";
/// Stable reason code: the drag target is not a finite world point.
pub const REASON_NON_FINITE_TARGET: &str = "grip.non_finite_target";
/// Stable reason code: a quadrant drag yields a zero or non-finite radius.
pub const REASON_DEGENERATE_RADIUS: &str = "grip.degenerate_radius";
/// Stable reason code: an arc-endpoint drag would collapse the arc's sweep.
pub const REASON_DEGENERATE_ARC: &str = "grip.degenerate_arc";
/// Stable reason code: a quadrant id on a **partial** ellipse (an elliptical arc).
pub const REASON_PARTIAL_ELLIPSE: &str = "grip.partial_ellipse";
/// Stable reason code: a block reference's stored placement is not finite.
pub const REASON_DEGENERATE_TRANSFORM: &str = "grip.degenerate_transform";
/// Stable reason code: a dimension definition is self-contradictory (its subtype
/// and point slots disagree, or a stored point is non-finite).
///
/// Both public `DimensionGeometry` constructors refuse such a definition, so this
/// is a **bug signal** for a stored entity: it also covers a mutated definition
/// that stopped being consistent, which no single-point replacement can cause.
pub const REASON_DIMENSION_INCONSISTENT: &str = "grip.dimension_inconsistent";
/// Stable reason code: a dimension definition-point drag leaves no synthesizable
/// display (or a non-finite measurement / an empty display).
///
/// The mutated definition is never clamped into something storable: a `NaN`
/// group 42 or an empty display would be silent corruption.
pub const REASON_DIMENSION_DEGENERATE: &str = "grip.dimension_degenerate";
/// Stable reason code: the addressed entity does not exist in the drawing.
///
/// Raised by the `GRIPSTRETCH` command (`crate::app_drawing::grip`), which is the
/// only place that can know an entity id stopped resolving; see the module doc.
pub const REASON_TARGET_MISSING: &str = "grip.target_missing";

/// The angular slack that makes a recomputed arc sweep degenerate.
///
/// `1e-9` rad is the same slack the tessellator (`cad-geometry/src/tessellate.rs`)
/// and the representation provider use to call an existing sweep "full"; it is
/// reused here so "the arc vanished / became the whole circle" has one threshold
/// in the codebase. A sweep inside it is refused, never clamped.
pub const ARC_SWEEP_EPSILON: f64 = 1e-9;

/// A draggable handle of one entity.
///
/// The variants are exactly the handles the drag rules above implement; see the
/// module doc for the deliberate absence of width/rotation/height/attachment/
/// tangent kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GripKind {
    /// An endpoint: a `LINE`'s start/end, or an `ARC`'s start (`index` 0) and end
    /// (`index` 1) point.
    Endpoint,
    /// The midpoint of a straight line (`LINE` only); dragging translates the
    /// whole line by `to - midpoint`.
    Midpoint,
    /// The centre of a circle, an arc or an ellipse; dragging moves the entity
    /// rigidly.
    Center,
    /// A quadrant point of a circle, or an axis point of a **full** ellipse
    /// (`index` `0..=3`); dragging sets the corresponding radius to the in-plane
    /// distance from the centre to the target. A partial ellipse (an elliptical
    /// arc) exposes no quadrant handle.
    Quadrant,
    /// A stored vertex of a polyline (`POLYLINE`/`LWPOLYLINE` only, `index` = the
    /// vertex index in stored order); dragging moves that one vertex in x/y and
    /// keeps its own `z`, leaving the other vertices and every bulge untouched.
    Vertex,
    /// A stored control point of a NURBS spline (`SPLINE` only, `index` = the
    /// control-point index in stored order). A control point belongs to the curve
    /// **definition** and is generally **not on** the drawn curve; dragging it
    /// redefines that one entry and keeps `degree`/`knots`/`weights` bit-identical,
    /// so the drag reshapes the curve as a different NURBS — it never re-fits it,
    /// and the curve stops passing through the points it used to pass through.
    ControlPoint,
    /// The position of a point entity (`POINT` only).
    Point,
    /// The **insertion point** of an entity that is placed by an anchor
    /// (`index` 0): a `TEXT`/`MTEXT`'s stored `position`, or a block reference's
    /// transformed block-space origin. For a text, dragging sets `position` to the
    /// target and leaves `text`, `style`, `height`, `rotation`, `font`, `h_align`,
    /// `v_align` and `wrap_width` bit-identical, so alignment and rotation never
    /// move the handle. For a block reference, dragging moves the instance
    /// **rigidly**: only the placement's translation column changes, so rotation,
    /// scale, mirror and `block` stay bit-identical — a drag never rotates or
    /// scales a block. It is deliberately **not**
    /// [`GripKind::Point`]: that kind is the `POINT` entity's own handle, and
    /// sharing it would leave [`REASON_STALE_HANDLE`] unable to tell a text or a
    /// block reference from a point entity.
    Insertion,
    /// A stored DXF **definition point** of a dimension, or the dimension's stored
    /// text middle point (group 11). `index` is the position in the documented
    /// per-subtype order (see the handle table): for a `Linear`/`Aligned`
    /// dimension `0` = `first`, `1` = `second`, `2` = `dimension_line`, `3` =
    /// `text_middle`; a `Radial` one has `0` = `center`, `1` = `point_on_arc`,
    /// `2` = `text_middle`; and so on for `Angular2Ln`, `Angular3Pt` and
    /// `Ordinate`. Dragging replaces that one stored point and **re-synthesizes**
    /// the display with the entity's own style and text override, so `kind`,
    /// `text_rotation`, `normal`, `style_name` and `style` are preserved and every
    /// other point is bit-identical. `leader_length` and `text_rotation` are
    /// scalars and are **not** handles. It is deliberately **not**
    /// [`GripKind::Endpoint`] or [`GripKind::Insertion`]: sharing another kind
    /// would leave [`REASON_STALE_HANDLE`] unable to tell a dimension's handle from
    /// a line endpoint or a text insertion point.
    DefinitionPoint,
}

/// The four circle/ellipse quadrant handles, in documented `index` order.
///
/// Each entry picks one axis of the entity's own plane frame (`0` = the first
/// axis, `1` = the second axis) and a sign, i.e. `0` = `+axis0`, `1` = `+axis1`,
/// `2` = `-axis0`, `3` = `-axis1`. For a circle the frame is
/// `cad_geometry::arbitrary_axis`, so a `+Z` circle reads `+X`, `+Y`, `-X`, `-Y`;
/// for an ellipse the frame is its stored `(unit major, unit minor)` axes, so the
/// same order reads `+major`, `+minor`, `-major`, `-minor`.
const QUADRANT_DIRECTIONS: [(u8, f64); 4] = [(0, 1.0), (1, 1.0), (0, -1.0), (1, -1.0)];

/// The identity of one handle: which entity, which kind of handle, which one.
///
/// `index` is the handle's ordinal **within its kind**, in a documented order:
/// for a `LINE`, `0` is `start`, `1` is `end` and `2` is the midpoint; for a
/// `CIRCLE`/`ELLIPSE`, `0` is the centre and the quadrants are `0..=3`; for an
/// `ARC`, `0` is the centre, `Endpoint` `0` is the start point and `Endpoint` `1`
/// the end point; for a `POLYLINE`, a `Vertex` `index` is the **stored** vertex
/// index (`u8`, so only vertices `0..=255` can be addressed); for a `SPLINE`, a
/// `ControlPoint` `index` is the **stored** control-point index (`u8`, so only
/// control points `0..=255` can be addressed); for a `TEXT`/`MTEXT`, `0` is the
/// stored insertion point; for a block reference (`INSERT`), `0` is the placement's
/// insertion point (the transformed block-space origin); for a `DIMENSION`, a
/// `DefinitionPoint` `index` is the position in that subtype's documented order
/// (the DXF definition points, then the text middle point); for a `POINT`, `0`
/// is the position. The id is a caller-held reference into geometry that may have
/// changed since it was read, so [`apply_grip`] re-validates it against the
/// geometry it is handed instead of trusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GripId {
    pub entity: EntityId,
    pub kind: GripKind,
    pub index: u8,
}

/// A handle plus the world position it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grip {
    pub id: GripId,
    /// The stored handle position (a line midpoint is the exact f64 midpoint of
    /// the two stored endpoints; a block reference's insertion point is the
    /// translation column of its placement).
    pub position: Point3,
}

/// Hit-test a world position against the handles of `entities`.
///
/// Returns the closest handle within `tolerance` world units, or `None`. A
/// candidate must satisfy `distance <= tolerance` (so a handle exactly at the
/// tolerance is hit, and `tolerance == 0.0` only hits an exactly coincident
/// handle). Entities that expose no grips are skipped. Ties are broken by the
/// documented iteration order: the **earliest entity of the input** wins, and
/// within one entity the **earlier grip of the list** wins (a candidate must be
/// strictly closer to replace the current winner).
///
/// A non-finite `world`, a non-finite or negative `tolerance`, and a stored
/// handle position at a non-finite coordinate are all non-hits.
#[must_use]
pub fn grip_hit<'a>(
    entities: impl IntoIterator<Item = &'a DbEntity>,
    world: Point3,
    tolerance: f64,
) -> Option<Grip> {
    if !is_finite(world) || !tolerance.is_finite() || tolerance < 0.0 {
        return None;
    }
    let mut best: Option<(f64, Grip)> = None;
    for entity in entities {
        for grip in grips_for_entity(entity) {
            let distance = distance(world, grip.position);
            // Written as an explicit NaN guard so a `NaN` distance (a
            // non-finite stored coordinate) cannot pass the filter.
            if distance.is_nan() || distance > tolerance {
                continue;
            }
            match best {
                // Strictly closer only: the earlier candidate keeps an exact
                // tie, which is the documented tie-break.
                Some((best_distance, _)) if distance >= best_distance => {}
                _ => best = Some((distance, grip)),
            }
        }
    }
    best.map(|(_, grip)| grip)
}

/// Hit-test a **plan-view** world position against the handles of `entities`.
///
/// Identical to [`grip_hit`] — same candidate iteration order, same
/// "strictly closer wins" replacement rule (so an exact tie still goes to the
/// earliest entity of the input and, within one entity, to the earlier grip of
/// the list), same `distance <= tolerance` predicate and same finiteness
/// refusals — with **one** difference: the distance is measured in the x/y plane
/// only, so the `z` coordinates of `world` and of the handle are ignored.
///
/// This is the documented **plan-view** hit rule for a pointer: the host
/// resolves a pointer through the viewport's `screen_to_world`, which always
/// returns a point on the work plane (`z = work_plane.origin.z`). A 3D distance
/// against that point would miss every grip at another elevation (a `LINE` drawn
/// at `z = 5` would be un-grippable in the plan view that displays it), so the
/// plan-view rule deliberately measures what the user is aiming at on screen.
///
/// It is **not** 3D grip editing: dragging a handle in `z` is NOT IMPLEMENTED.
/// The host keeps the dragged handle's own `z` in the target it commits, so a
/// plan-view drag never changes an entity's elevation.
///
/// A non-finite `world`, a non-finite or negative `tolerance`, and a stored
/// handle position at a non-finite coordinate (any of `x`, `y`, `z`) are
/// non-hits. The explicit stored-position check is required here because an
/// ignored `NaN` `z` would otherwise not poison the planar distance.
#[must_use]
pub fn grip_hit_planar<'a>(
    entities: impl IntoIterator<Item = &'a DbEntity>,
    world: Point3,
    tolerance: f64,
) -> Option<Grip> {
    if !is_finite(world) || !tolerance.is_finite() || tolerance < 0.0 {
        return None;
    }
    let mut best: Option<(f64, Grip)> = None;
    for entity in entities {
        for grip in grips_for_entity(entity) {
            if !is_finite(grip.position) {
                continue;
            }
            let distance = planar_distance(world, grip.position);
            if distance.is_nan() || distance > tolerance {
                continue;
            }
            match best {
                // Strictly closer only: an exact tie keeps the earlier candidate,
                // exactly like `grip_hit`.
                Some((best_distance, _)) if distance >= best_distance => {}
                _ => best = Some((distance, grip)),
            }
        }
    }
    best.map(|(_, grip)| grip)
}

fn is_finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

/// Whether every coefficient of a stored placement is finite.
///
/// The block-reference drag rule rewrites the translation column only, so a
/// non-finite coefficient of the stored transform would be carried straight into
/// the staged entity; `cad-db`'s validator refuses a non-finite insert transform,
/// so the drag refuses it by name ([`REASON_DEGENERATE_TRANSFORM`]) instead.
fn transform_is_finite(transform: &cad_domain::Transform3) -> bool {
    transform
        .matrix
        .iter()
        .flatten()
        .all(|cell| cell.is_finite())
}

fn distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// x/y distance, ignoring `z` (the plan-view rule of [`grip_hit_planar`]).
fn planar_distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
