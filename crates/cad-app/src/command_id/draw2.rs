//! Long-form contracts for the 2026-11-07 2D draw/modify batch.
//!
//! One struct per command, kept in this doc module so `command_id.rs` stays
//! inside the code-size limit. The implementations live in
//! `app_drawing::draw2_batch` (creation) and `app_drawing::draw2_edit`
//! (selection edits); the shared validation/transaction discipline is the same
//! as every other create/edit command: every refusal happens before a
//! transaction opens, and one successful command is exactly one transaction /
//! one undo step.

/// `DONUT` — two concentric circles.
///
/// # Input
///
/// [`crate::Draw2Payload::Donut`] carries the centre and both radii. The
/// command-line form is `DONUT <cx> <cy> <inner> <outer>`; this build has no
/// centre pick prompt, so the centre is typed rather than clicked. The declared
/// AutoCAD form `DONUT <inner> <outer>` is therefore **not** available on the
/// command line; the payload/API path is the complete one.
///
/// # Write
///
/// `outer` becomes the first circle and `inner` the second, both
/// `SemanticGeometry::Circle` on the session's active layer in model space,
/// stored in one transaction / one undo step through the shared create funnel.
///
/// `inner == 0` is accepted (AutoCAD's filled donut form), but a zero-radius
/// circle is not representable: the command stores the **outer circle only**
/// and reports the `donut.inner_zero` diagnostic. That is a declared partial
/// result, never a fabricated second entity.
///
/// # Refusals (before any write)
///
/// * `donut.payload` — wrong payload shape;
/// * `donut.invalid_point` — non-finite centre;
/// * `donut.invalid_radius` — non-finite radius, or `inner < 0`;
/// * `donut.radius_order` — `outer <= inner`.
pub struct DonutCommand;

/// `SOLID` — one filled triangle or quadrilateral.
///
/// # Input
///
/// [`crate::Draw2Payload::Solid`] carries 3 or 4 boundary points in ring order.
/// The command-line form is `SOLID <x,y> <x,y> <x,y> [<x,y>]`.
///
/// # Representation
///
/// The existing filled-geometry representation is
/// `SemanticGeometry::Mesh(Mesh)` (the same one the importer's `SOLID`/`TRACE`
/// and `3DFACE` paths use): a triangle is one triangle, a quad is two triangles
/// `[0,1,2]` and `[0,2,3]`, with computed vertex normals and no colours. The
/// stored type key is `AcDbSolid`; the DXF exporter writes a
/// `PolyfaceMesh` for any `Mesh`, so a round-trip is **not** a SOLID record
/// (declared loss, see the batch doc). The outline is never drawn as an
/// unfilled line.
///
/// # Refusals (before any write)
///
/// * `solid.payload` — wrong payload shape;
/// * `solid.point_count` — fewer than 3 or more than 4 points;
/// * `solid.invalid_point` — a non-finite point;
/// * `solid.not_planar` — points off one `+Z` plane;
/// * `solid.duplicate_point` — two consecutive points coincide;
/// * `solid.self_crossing` — the quad's edges cross;
/// * `solid.zero_area` — the face encloses no area.
pub struct SolidCommand;

/// `HELIX` — a flat 2D spiral polyline.
///
/// # Input
///
/// [`crate::Draw2Payload::Helix`] carries the base point, the final radius, the
/// turn count and the height. The command-line form is
/// `HELIX <base x> <base y> <radius> <turns> <height>`.
///
/// # Write
///
/// The command stores one open `SemanticGeometry::Polyline` in the XY plane at
/// the base point's Z: the radius grows linearly from 0 to `radius` over
/// `turns` revolutions, sampled at up to 64 vertices per turn (bounded to 4096
/// vertices). One transaction / one undo step on the active layer.
///
/// # Height
///
/// `height == 0` is the flat spiral this build implements. Any positive height
/// is the **3D spring**, which is NOT IMPLEMENTED and is refused by name
/// (`helix.3d_not_implemented`) rather than silently flattened. A negative or
/// non-finite height is `helix.invalid_height`.
///
/// # Refusals (before any write)
///
/// * `helix.payload` — wrong payload shape;
/// * `helix.invalid_point` — non-finite base;
/// * `helix.invalid_radius` — non-positive/non-finite radius;
/// * `helix.invalid_turns` — non-positive/non-finite turns;
/// * `helix.invalid_height` — negative/non-finite height;
/// * `helix.3d_not_implemented` — `height > 0`.
pub struct HelixCommand;

/// `MLINE` — a partial multiline: two parallel LINEs around a baseline.
///
/// # Input
///
/// [`crate::Draw2Payload::Mline`] carries the baseline endpoints, an optional
/// spacing and a lateral offset. The command-line form is
/// `MLINE <spacing> <offset> <x1,y1> <x2,y2>`. When `spacing` is `None`, the
/// current `MLSTYLE` style's spacing is used (the style store feeds the
/// command); an explicit spacing overrides it for this invocation only.
///
/// # Write
///
/// Exactly **two** `SemanticGeometry::Line` entities are created, parallel to
/// the baseline at `±spacing/2 + offset` measured along the baseline's XY
/// normal. One transaction / one undo step on the active layer. The two-line
/// count is this v1's fixed element count: MLINE styles carry a name and a
/// spacing but no element table.
///
/// # NOT IMPLEMENTED
///
/// Joins, caps, miter/bevel corners, style element definitions, and a
/// multi-segment baseline. The command is a declared partial MLINE.
///
/// # Refusals (before any write)
///
/// * `mline.payload` — wrong payload shape;
/// * `mline.point_count` — not exactly two baseline points;
/// * `mline.invalid_point` — a non-finite point;
/// * `mline.degenerate_baseline` — the two points coincide;
/// * `mline.invalid_spacing` — non-positive/non-finite spacing;
/// * `mline.invalid_offset` — non-finite offset.
pub struct MlineCommand;

/// `MLSTYLE` — the bounded, session-only multiline style store.
///
/// # Input
///
/// `MLSTYLE LIST` reports every stored style. `MLSTYLE SET <name> <spacing>`
/// creates or updates one style and makes it current. The name is one token
/// (no spaces), trimmed, non-empty, at most 64 characters and single-line.
///
/// # State
///
/// The store lives on [`crate::SessionState`] (`ml_styles`), is capped at
/// [`crate::MULTILINE_STYLE_CAP`] (16) entries and is **never** persisted to
/// the drawing or DWG. The default style is `STANDARD` at spacing `1.0`.
/// Because it writes no drawing data, the command is legal in Viewer mode.
///
/// # Refusals (before any state change)
///
/// * `mlstyle.payload` — wrong payload shape;
/// * `mlstyle.invalid_name` — empty/blank/too long/multi-line name;
/// * `mlstyle.invalid_spacing` — non-positive/non-finite spacing;
/// * `mlstyle.capacity` — creating a seventeenth style.
///
/// NOT IMPLEMENTED: DWG `MLSTYLE` records, element tables, caps/joins,
/// linetypes and colours.
pub struct MlStyleContract;

/// `TREX` — trim or extend the current selection against itself.
///
/// # Input
///
/// `TREX TRIM|EXTEND` over the **current selection**. Every selected,
/// directly addressable model-space object is a target; every *other* selected
/// object is a boundary. There is no boundary prompt and no pick prompt: the
/// kept side of a TRIM target is the piece containing the target's parametric
/// midpoint (`u = 0.5`), and EXTEND grows the target's `u = 1` end. This
/// declared divergence makes the command deterministic without a click.
///
/// # Reuse
///
/// The command reuses the existing `TRIM`/`EXTEND` planner
/// (`plan_trim` / `plan_extend`) and its boundary collector — there is no
/// second trim implementation. A target whose plan refuses (no intersection,
/// degenerate pick, unsupported continuation) is skipped with a
/// `trex.skipped` diagnostic; an unsupported *boundary* geometry refuses the
/// whole command before any write.
///
/// One transaction / one undo step for every changed target.
///
/// # Refusals (before any write)
///
/// * `trex.payload` — wrong payload shape;
/// * `trex.selection` — fewer than two selected objects;
/// * `trex.not_addressable` — a ref from another document, a block-instance
///   member, a sub-element, a missing entity or a non-model-space row;
/// * `trex.unsupported_boundary` — a selected object that cannot bound a cut.
pub struct TrexCommand;

/// `EXTRIM` — trim crossing objects at one boundary.
///
/// # Input
///
/// [`crate::Draw2Payload::Extrim`] carries the boundary ref and the crossing
/// targets. The command-line form with `CommandPayload::None` uses the current
/// selection: the **first** selected object is the boundary and the rest are
/// targets (declared choice; there is no second pick prompt).
///
/// # Reuse and write
///
/// Each target is trimmed with the existing `plan_trim` planner, keeping the
/// piece that contains the target's parametric midpoint. The boundary may be a
/// `Line`, `Circle`, `Arc` or `Polyline` (the collector's supported set); any
/// target kind outside LINE/ARC/CIRCLE/SPLINE is refused by name. All targets
/// change in one transaction / one undo step.
///
/// # Refusals (before any write)
///
/// * `extrim.payload` — wrong payload shape;
/// * `extrim.selection` — no targets;
/// * `extrim.not_addressable` — a foreign ref, block member, sub-element,
///   missing entity or non-model-space row;
/// * `extrim.unsupported_boundary` — the boundary geometry cannot bound a cut;
/// * `extrim.unsupported_target` — a target kind the trim planner does not
///   accept.
pub struct ExtrimCommand;

/// `MPEDIT` — apply one `PEDIT` option to every selected polyline.
///
/// # Input
///
/// `MPEDIT CLOSE|OPEN|REVERSE` over the current selection
/// ([`crate::Draw2Payload::Mpedit`]). The option subset is exactly `PEDIT`'s.
///
/// # Write
///
/// Every selected, directly addressable model-space `LWPOLYLINE` is changed in
/// **one** transaction / one undo step. Non-polyline selected objects are
/// per-entity `mpedit.skipped` diagnostics and do not fail the command; a
/// polyline already in the requested state is reported as
/// `mpedit.unchanged` and not staged. With no eligible polyline the command is
/// an `mpedit.nothing_changed` no-op (no transaction, no undo entry).
///
/// All work is done before the transaction opens, so a refusal changes nothing.
///
/// # Refusals (before any write)
///
/// * `mpedit.payload` — wrong payload shape;
/// * `mpedit.selection` — an empty selection.
pub struct MpeditCommand;

/// `DRAWORDER` — rewrite model-space draw order.
///
/// # Input
///
/// `DRAWORDER FRONT|BACK|ABOVE|BELOW` over the current selection
/// ([`crate::Draw2Payload::DrawOrder`]). The selection must be direct
/// model-space refs.
///
/// # Semantics
///
/// * `FRONT` moves every selected object above every non-selected model-space
///   object, preserving the selection's relative order.
/// * `BACK` moves every selected object below every non-selected model-space
///   object, preserving the selection's relative order.
/// * `ABOVE` / `BELOW` use the **first selected** object as the reference and
///   move the rest (the moving set) immediately above/below it, preserving the
///   moving set's relative order. Objects already in the way are shifted by the
///   needed amount in the same transaction, so no two unrelated rows silently
///   swap places.
///
/// When every selected and shifted row already has its target order the command
/// is a `draworder.unchanged` no-op. Otherwise all changed rows are staged in one
/// transaction / one undo step. Paper-space and block draw order is not
/// reordered.
///
/// # Refusals (before any write)
///
/// * `draworder.payload` — wrong payload shape;
/// * `draworder.selection` — an empty selection;
/// * `draworder.reference_required` — `ABOVE`/`BELOW` with fewer than two
///   selected objects (there is no separate reference prompt);
/// * `draworder.not_addressable` — a foreign ref, block member, sub-element,
///   missing entity or non-model-space row;
/// * `draworder.order_exhausted` — a target order would overflow `i64`.
pub struct DrawOrderCommand;
