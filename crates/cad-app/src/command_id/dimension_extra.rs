//! Long-form notes for the AutoCAD 2026 **rotated linear dimension** command
//! [`crate::CommandId::DimRotated`] (`DIMROTATED`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here.
//!
//! ## Input
//!
//! AutoCAD prompts `Specify angle of dimension line <0>` and then asks for the
//! two extension-line origins and the dimension-line location. This build's
//! fixed-step capture cannot insert a numeric angle prompt into the point
//! sequence, so the angle is the **session parameter** channel
//! [`crate::draw_tool::ToolParameter::DimRotation`] (degrees, default `0`, any
//! finite value — a negative angle is a legal rotation in the other direction and
//! is not normalised). The bare `DIMROTATED` selector starts a **three-pick**
//! capture ([`crate::draw_tool::DrawToolKind::DimRotated`], arity
//! `Exact(3)`); a payload caller supplies
//! [`crate::CommandPayload::RotatedLinearDimension`] with `rotation` already in
//! **radians** (the host converts the degrees exactly once).
//!
//! ## Construction
//!
//! The definition is `DimensionKind::Linear { rotation }` with
//! `DimensionPoints::Linear { first, second, dimension_line: location }` — the
//! same subtype `DIMLINEAR` uses, and the same shared synthesis
//! ([`cad_domain::DimensionGeometry::synthesized`]) recomputes the measurement,
//! the formatted text and the display. The measurement is the projection of the
//! origin separation onto the dimension line's direction,
//! `|cos θ · Δx + sin θ · Δy|` (the `Dot` product the synthesis already uses), so
//! `0` measures `Δx`, `π/2` measures `Δy` and `30°` measures the 30° projection.
//! The location pick only places the dimension line; it never changes the angle.
//!
//! ## Refusals
//!
//! * `dimrotated.rotation` — the rotation is non-finite (NaN/±∞);
//! * `dimrotated.points` — the payload is not exactly three points;
//! * `dimrotated.non_finite` — a point is non-finite;
//! * `dimrotated.off_plane` — the points are not on one `+Z` plane;
//! * `dimrotated.origins` — the two extension-line origins coincide;
//! * `dimrotated.degenerate` — the chosen direction measures zero.
//!
//! Every refusal happens before any write: no transaction, no revision bump and
//! no history entry. Creation is **non-associative** (no `ACAD_DIMASSOC`).
//!
//! ## NOT IMPLEMENTED
//!
//! The interactive `Specify angle of dimension line` prompt, the object-selection
//! variant (pick a line/object to derive the origins), a `Text`/`Mtext` override,
//! associativity (`DIMASSOC=2`) and 3D/work-plane creation. Work-only.
//!
//! # `DIMSPACE` (AutoCAD 2026)
//!
//! [`crate::CommandId::DimSpace`] is the **selection-based** re-spacing command. Its
//! payload is [`crate::CommandPayload::None`]: the current selection is the whole
//! input, exactly like `MATCHPROP`/`JOIN`, and a supplied payload is refused by name
//! (`dimspace.payload`).
//!
//! ## Declared v1 boundary
//!
//! AutoCAD prompts "Select base dimension", "Select dimensions to space", then
//! "Enter value or [Auto] <Auto>". This build implements neither the object prompts
//! (they are replaced by the current selection) nor the **value prompt**: spacing is
//! **always automatic**, i.e. the base dimension's resolved style `DIMDLI`
//! ([`cad_domain::DimensionStyleValues::dim_line_increment`], AutoCAD's value `0`).
//! Associative reflow is **NOT IMPLEMENTED** either: no dimension is re-associated
//! with its measured geometry, and the measured points never move.
//!
//! ## Construction
//!
//! * the **first** selected linear dimension is the base; every remaining selected
//!   dimension is a target, in **pick order**;
//! * all selected objects must be directly addressable **model-space**
//!   `DIMENSION` entities and every one must be `DimensionKind::Linear` — a
//!   non-dimension, an aligned/angular/radial/ordinate dimension, a block member, a
//!   sub-element, another document's ref or another space is refused by name;
//! * the stacking normal is `±rot90(base.rotation)`, signed **away from the base's
//!   measured geometry** (`dot(base.location − midpoint(first, second), n̂) > 0`); a
//!   zero dot product is `dimspace.undecidable_normal`, never a guess;
//! * target `i` (`i = 1..=N`) keeps its own `first`/`second`/rotation/style snapshot/
//!   text override and moves only its dimension line to
//!   `base.location + n̂ · (i · spacing)`; the whole entity is re-synthesized through
//!   [`cad_domain::DimensionGeometry::synthesized`].
//!
//! A non-finite or non-positive base `DIMDLI` is `dimspace.bad_spacing` (it would
//! stack every target on one line). A target already on its row is not staged, so a
//! run that changes nothing opens **no** transaction and reports
//! `dimspace.unchanged`; otherwise every changed target is staged in **one
//! transaction / one undo step** and the success diagnostic `dimspace.applied`
//! names the count. Work-only.
//!
//! ## NOT IMPLEMENTED
//!
//! The interactive base/target prompts, the spacing **value** prompt (`0` = auto or
//! an explicit distance), angular-dimension spacing against its arc, associative
//! reflow, a live preview, and non-model spaces.
