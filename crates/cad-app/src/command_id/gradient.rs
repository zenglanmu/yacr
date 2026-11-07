//! Long-form notes for [`crate::CommandId::Gradient`] (`GRADIENT`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. The
//! implementation reuses the HATCH creation path in `app_drawing/hatch.rs` — it
//! only forces the gradient fill, never a second boundary/loop implementation.
//!
//! ## Input
//!
//! Payload is [`crate::CommandPayload::None`]; the **current selection** is the
//! closed boundary, exactly like HATCH's pre-selection input
//! (ADR 0018 decision D, P6b). A caller that supplies a payload is refused
//! (`gradient.payload`) rather than having the selection silently ignored. The
//! command captures no points and takes no in-command picks.
//!
//! ## Boundary subset
//!
//! The same subset HATCH implements, through the same classifier, stitcher and
//! plane resolver: a `Circle`, a full `Ellipse`, a closed `Polyline`, and
//! `LINE`/`ARC` edges stitched into closed loops (arcs stay exact bulge
//! segments, never sampled). An empty selection is `gradient.selection`; the
//! shared HATCH refusals (open polyline, unsupported kind, non-finite input,
//! branch/open/degenerate stitch, more than one normal, not coplanar, and every
//! `HatchGeometry::new` creation-path rejection) are reported with their shared
//! codes and the message relabelled `GRADIENT …`.
//!
//! Unlike HATCH, a GRADIENT boundary must lie on the **`+Z` plane**: a boundary
//! whose resolved plane normal is not `+Z` (within `1e-9`) is refused as
//! `gradient.not_plus_z` before any write. This is the declared v1 limit of the
//! gradient bake, not a property of the hatch geometry.
//!
//! ## Fill
//!
//! The fill is always `HatchFill::Gradient`, resolved from the **existing**
//! session gradient channel (`hatchGradientKind`, `hatchGradientAngle` in
//! degrees, `hatchGradientColorA`/`B` as ACI indices) through the same
//! [`crate::app_drawing::hatch_parameters`] validation HATCH uses. The default
//! session values (Linear, 0°, ACI 1 → 5) are therefore the two-colour linear
//! default. An out-of-set kind, a non-finite angle or an ACI index outside the
//! palette (including the `0` ByBlock placeholder) is refused by name and writes
//! nothing. The stale HATCH pattern type/spacing/index are ignored: the command
//! forces the gradient kind, so a stale spacing can never refuse a gradient.
//!
//! ## Commit
//!
//! One `AcDbHatch` entity (`type_key`/`class_name` = `AcDbHatch`) in one
//! transaction / one undo step, on the active layer, through the shared create
//! funnel. The selected boundary entities are retained (no associativity). The
//! summary diagnostic is `drawing.hatch` and names the gradient kind, angle and
//! stop colours actually written.
//!
//! ## NOT IMPLEMENTED
//!
//! A gradient preview, the interactive `-GRADIENT` option prompts, `.pat`/pattern
//! handling, associativity, boundary-from-an-internal-point, a gradient editor
//! dialog, single-colour/`tint` gradients and non-`+Z` boundary planes. Work-only
//! (Viewer mode refuses it at the command layer).
