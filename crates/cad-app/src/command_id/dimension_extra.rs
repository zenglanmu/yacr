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
