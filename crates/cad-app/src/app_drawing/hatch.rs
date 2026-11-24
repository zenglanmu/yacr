//! HATCH creation from the **pre-selection** (ADR 0018 decision D, P6b).
//!
//! The boundary is the selection snapshot: `DrawToolKind::Hatch` captures no
//! points (the zero-point exact complement, like ARRAYRECT) and takes no
//! in-command entity picks, so the refs arrive in `CommandPayload::Hatch`. Each
//! accepted closed entity becomes **one loop** in the hatch plane's OCS 2D frame,
//! and the selected `LINE`/`ARC` edges are chained into one loop per closed chain:
//!
//! * `Circle` → an **exact** arc loop: four quadrant vertices with a
//!   `tan(pi/8)` bulge on every segment, i.e. four quarter-circle arcs. The
//!   ADR's cheap win stores a circle as two bulge-1 segments; this build uses
//!   four because the creation-path validator (`HatchGeometry::new`) requires at
//!   least three vertices and measures a chord ring, so a two-vertex loop is a
//!   degenerate chord set there. Four quarter arcs are still an exact circle and
//!   still export as an exact bulged polyline boundary.
//! * full `Ellipse` → a **sampled** chord loop (`ELLIPSE_SEGMENTS` chords per
//!   turn). A true ellipse is not a circular arc, so a bulge loop would be a
//!   lie; this loss is declared here and in the command docs.
//! * closed `Polyline` → its own points and bulges, mapped into the hatch plane.
//! * `Line`/`Arc` edges (any mix with the kinds above) → **stitched** into one or
//!   more closed loops by [`super::hatch_stitch`] and emitted as a closed
//!   **bulged** polyline: an `Arc` becomes one exact bulge segment
//!   (`bulge = tan(sweep / 4)`, matching the circle loop's quarter-arc
//!   `tan(pi/8)`) and a `Line` a bulge-0 segment — the arcs are never sampled.
//!   Endpoints chain within the absolute [`STITCH_TOLERANCE`] of 1e-6 world
//!   units; an edge may be consumed in either direction (mixed drawing
//!   orientations stitch, the traversal direction fixes the bulge sign); a node
//!   where more than two edges meet is refused (`hatch.boundary_branch`) instead
//!   of pairing guessed, and edges left outside every loop are refused
//!   (`hatch.boundary_open`) naming how many. A zero-length LINE or zero-sweep
//!   ARC is refused before stitching (`hatch.boundary_degenerate`).
//!
//! Every selected entity must lie in one plane with a single consistent normal
//! (circles/ellipses declare their normal; a polyline's is implied by its
//! points); anything else is refused **by name** before any write. The geometry
//! goes through `HatchGeometry::new`, so the ADR A.3 creation-path rejections
//! (self-intersecting, tangent/shared vertex, duplicate, partially overlapping,
//! unclosed, degenerate, non-finite) fail the whole command with no transaction.
//! **Declared limitation**: that validator works on the loop's **chords**, so for
//! a bulged loop (our circle loop included) a tangency that falls between the
//! sampled chords of two loops is not detected. An exact-circle contact whose
//! inscribed squares touch *is* refused (pinned by the tests); the residual gap
//! is a chord-sampling blind spot, not a silent acceptance of a crossing.
//!
//! The selected boundary entities are **retained**: the hatch is a derived
//! annotation over them (no associativity, no `DELOBJ` gating).
//!
//! The typed parameters are read from the session (the POLYGON precedent): the
//! fill kind, spacing, angle (degrees on the channel, radians in the domain), the
//! `HPDOUBLE` flag and the style. A user-defined pattern is synthesized from
//! angle/spacing/double by the same one-row definition the import path generates
//! for a `76=0`/`78=0` hatch, so creation and import share one recipe; the
//! generated line geometry remains **our implementation** (ADR U-1), not a
//! documented AutoCAD formula.
//!
//! Pattern type code `2` selects one of the **predefined** patterns by its
//! **1-based** index (`hatchPattern`). The implemented library is the three
//! hand-transcribed `.pat` patterns (`ANSI31`, `ANSI32`, `ANSI37`), stored as
//! AutoCAD `.pat` definition text and parsed by
//! `super::hatch_pattern::PatternLibrary` — the same loader, name resolution
//! and index resolution a user pattern file goes through. Spacing is the pattern
//! scale (`HPSCALE`), angle the hatch-level rotation and the double flag the
//! perpendicular second family; the definition rows come from the library, so
//! these are **our transcribed values, not vendor-verified**. An index outside
//! the library is refused **by name** — never clamped onto a neighbouring
//! pattern.
//!
//! Because `hatchPattern` is a closed **numeric** index it cannot carry a name, so
//! the typed command line has a one-shot **pattern-name** channel:
//! `HATCH PATTERN <name>` (a built-in library name) and `HATCH PATTERNFILE
//! <path.pat> [<name>]` (a user file the **host** read), both carried on
//! [`CommandPayload::HatchPattern`](crate::CommandPayload::HatchPattern) and
//! resolved with the very same functions `HATCHEDIT` uses. The session fill
//! **type** is not consulted on that path (the command line named the pattern);
//! the scale, hatch angle, `HPDOUBLE` flag and style stay the session channel, and
//! the recorded DXF type follows the source (`1` built-in, `2` user file). A
//! typed `HATCH SOLID` / `HATCH GRADIENT` is refused by name
//! (`hatch.fill_not_pattern`) in the session: this form selects patterns, the fill
//! kind is the channel and a gradient has its own `GRADIENT` command.
//!
//! Pattern type code `3` creates a two-colour **gradient** fill from its own
//! channel (`hatchGradientKind` Linear/Spherical, `hatchGradientAngle` in
//! **degrees**, `hatchGradientColorA`/`B` as ACI indices resolved through the
//! canonical palette). The entity records `Predefined` (`76=1`) because a
//! gradient is written as a solid fill plus the 450-470 block (an unverified
//! vendor convention). The angle is converted to radians exactly once here, while
//! the import path stores group 460 verbatim — the two conventions are pinned by
//! the create/export/reimport test. `reserved` and `shift` are fixed `0`, and the
//! pattern spacing/double/style are not consulted (a stale spacing never refuses
//! a gradient).
//!
//! The implementation is split for the 1000-line gate: this file is the command
//! entry point, [`super::hatch_boundary`] classifies the boundary / stitches it /
//! resolves the plane / writes the loops, [`super::hatch_stitch`] is the pure edge
//! chainer and [`super::hatch_parameters`] owns the typed channel plus the summary
//! diagnostic.

use super::*;

use super::hatch_boundary::{boundary_loops, boundary_shapes, resolve_plane, stitch_boundary};
use super::hatch_edit_pattern_file::{resolve_file_pattern, PatternChoice};
use super::hatch_edit_pattern_name::resolve_builtin_pattern;
use super::hatch_parameters::{
    gradient_parameters, hatch_creation_refusal, hatch_diagnostic, hatch_parameters,
    hatch_pattern_refusal, hatch_style, pattern_parameters, HatchFillMode,
};
use crate::command_id::hatch::HatchPatternRequest;

/// Relabel a shared HATCH refusal for the GRADIENT command: the boundary rules
/// are the same code, so only the command name in the message changes. HATCH
/// itself passes its errors through unchanged.
impl HatchFillMode {
    fn relabel(self, error: CadError) -> CadError {
        if self != HatchFillMode::Gradient {
            return error;
        }
        match error {
            CadError::InvalidInput(message) => {
                CadError::InvalidInput(message.replace("HATCH", "GRADIENT"))
            }
            other => other,
        }
    }
}

impl Application {
    /// HATCH the pre-selected boundary entities into one entity (ADR 0018 D).
    ///
    /// One transaction / one undo step through the shared create funnel, on the
    /// active layer, with `type_key`/`class_name` = `AcDbHatch`. The hatch is
    /// written into the **common space of its boundaries**: model space through
    /// the shared funnel, a paper layout through that layout's own draw-order
    /// sequence. The selected boundary entities are **retained**: the hatch is a
    /// derived annotation over them, exactly like an AutoCAD "select objects"
    /// hatch (no associativity).
    ///
    /// Refusals (nothing written, no transaction):
    ///
    /// * an empty selection;
    /// * a selection entry referenced through a block instance or a sub-element
    ///   (the boundary must be a directly addressable entity);
    /// * any kind other than `Circle`, a full `Ellipse`, a closed `Polyline` or
    ///   `Line`/`Arc` edges (open polyline/spline/text/block/…);
    /// * an ellipse that is not a full turn (an ellipse arc is not a closed loop);
    /// * a `Line`/`Arc` edge that is degenerate (zero length, zero sweep — refused
    ///   by name before stitching), a stitched node where more than two edges meet
    ///   (`hatch.boundary_branch`) and stitched edges left in an open chain
    ///   (`hatch.boundary_open`);
    /// * a selection that is not coplanar with a single consistent normal;
    /// * every `HatchGeometry::new` creation-path rejection (self-intersecting,
    ///   tangent/shared vertex, duplicate, partially overlapping, unclosed,
    ///   degenerate, non-finite);
    /// * a missing active layer, an out-of-range typed parameter — including a
    ///   `hatchPattern` index that does not resolve in the implemented predefined
    ///   table (refused by name, never clamped), a gradient kind outside
    ///   `{Linear, Spherical}`, a gradient angle that is not finite and a
    ///   gradient ACI index that is non-integral, outside `0..=255` or the ACI
    ///   `0` ByBlock placeholder — and Viewer mode (the command layer's mode
    ///   check).
    ///
    /// A [`CommandPayload::HatchPattern`](crate::CommandPayload::HatchPattern)
    /// payload adds the typed pattern-name refusals, all resolved before the
    /// document is touched: `hatch.needs_pattern_name`, `hatch.unknown_pattern`
    /// (naming the searched sources), `hatch.pattern_file_invalid` /
    /// `hatch.pattern_file_ambiguous` (the `.pat` parser's own codes) and the
    /// session-side `hatch.pattern_file_*` read refusals.
    pub(crate) fn create_hatch(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            // The tool/`-HATCH` shape: the fill comes from the session channel.
            CommandPayload::Hatch { refs } => self.create_hatch_from_selection(
                session,
                command,
                refs,
                HatchFillMode::Session,
                None,
            ),
            // The typed command line: `PATTERN <name>` / `PATTERNFILE <path>
            // [<name>]` selects the pattern by name, with the resolution shared
            // with `HATCHEDIT`. Resolved **before** the document is touched, so
            // an unknown name / bad file writes nothing and opens no transaction.
            CommandPayload::HatchPattern { refs, request } => {
                let choice = resolve_typed_pattern(request)?;
                self.create_hatch_from_selection(
                    session,
                    command,
                    refs,
                    HatchFillMode::Session,
                    Some(choice),
                )
            }
            _ => Err(CadError::InvalidInput(
                "Hatch needs a Hatch { refs } or HatchPattern { refs, request } payload".into(),
            )),
        }
    }

    /// GRADIENT: fill the **currently selected** closed boundary entities with the
    /// session gradient parameters.
    ///
    /// This is the HATCH creation path with the fill forced to
    /// [`HatchFillMode::Gradient`]: the boundary classifier, stitcher, plane
    /// resolver and the `HatchGeometry::new` creation validator are the *same*
    /// functions, so GRADIENT has no second boundary implementation and the same
    /// HATCH refusal surface (relabelled `GRADIENT …`). The fill comes from the
    /// existing gradient channel (`hatchGradientKind`, `hatchGradientAngle` in
    /// degrees, `hatchGradientColorA`/`B`); the default session values are the
    /// two-colour linear ramp. The stale `hatchPatternType`/index/spacing are not
    /// consulted. Full contract: [`crate::command_id::gradient`].
    ///
    /// Additional v1 refusal: a boundary whose resolved plane normal is not `+Z`
    /// is `gradient.not_plus_z` (the gradient bake is defined in that plane).
    /// The command takes no payload (`gradient.payload`) and an empty selection is
    /// `gradient.selection`; both write nothing.
    pub(crate) fn create_gradient(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "gradient.payload: GRADIENT takes no payload; the current selection is the closed \
                 boundary input"
                    .into(),
            ));
        }
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "gradient.selection: GRADIENT needs at least one selected closed boundary entity"
                    .into(),
            ));
        }
        self.create_hatch_from_selection(session, command, refs, HatchFillMode::Gradient, None)
    }

    /// The one boundary-to-hatch path shared by HATCH, `HATCH PATTERN[FILE]` and
    /// GRADIENT.
    ///
    /// `mode` picks the fill authority (session channel or forced gradient) and
    /// `pattern` is the **command-line** pattern selection: `None` keeps the
    /// session fill channel (the tool path), `Some(choice)` writes the named
    /// `.pat` definition without consulting the session fill *type*.
    fn create_hatch_from_selection(
        &mut self,
        session: &SessionState,
        command: &Command,
        refs: &[SelectionRef],
        mode: HatchFillMode,
        pattern: Option<PatternChoice>,
    ) -> CadResult<CommandOutcome> {
        let gradient = mode == HatchFillMode::Gradient;
        // An empty boundary is refused by name before any parameter or document
        // work: a payload caller gets the same `hatch.selection` code the typed
        // line answers with (the `HATCH` spelling; `gradient.selection` for the
        // GRADIENT command is pre-checked by its own entry point).
        if refs.is_empty() {
            return Err(mode.relabel(CadError::InvalidInput(
                "hatch.selection: HATCH needs at least one selected closed boundary entity; \
                 nothing was written"
                    .into(),
            )));
        }
        let parameters = if gradient {
            gradient_parameters(session)?
        } else if pattern.is_some() {
            // The typed pattern selection needs the pattern scale/angle/double
            // and the style only: the session fill *type* is not read, so a stale
            // solid/gradient selector can neither refuse nor change it.
            pattern_parameters(session)?
        } else {
            hatch_parameters(session)?
        };

        let document_id = command.document;
        let (loops, plane, stitched_edges, space) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();

            for reference in refs {
                if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "{} boundary must be a directly addressable entity; entity {} is \
                         referenced through a block instance or a sub-element",
                        if gradient { "GRADIENT" } else { "HATCH" },
                        reference.entity.0
                    )));
                }
            }

            let (code, operation, action) = if gradient {
                ("gradient", "GRADIENT", "gradient")
            } else {
                ("hatch", "HATCH", "hatch")
            };
            let (sources, _diagnostics) =
                Self::resolve_selection(drawing, document_id, refs, code, operation, action, false)
                    .map_err(|error| mode.relabel(error))?;
            let space = hatch_target_space(&sources, if gradient { "gradient" } else { "hatch" })
                .map_err(|error| mode.relabel(error))?;
            let shapes = boundary_shapes(&sources).map_err(|error| mode.relabel(error))?;
            let stitched = stitch_boundary(&shapes).map_err(|error| mode.relabel(error))?;
            let plane = resolve_plane(&shapes, &stitched).map_err(|error| mode.relabel(error))?;
            if gradient && plane.normal.z < 1.0 - 1e-9 {
                return Err(CadError::InvalidInput(format!(
                    "gradient.not_plus_z: GRADIENT needs a boundary in the +Z plane; the resolved \
                     normal is ({}, {}, {})",
                    plane.normal.x, plane.normal.y, plane.normal.z
                )));
            }
            let loops =
                boundary_loops(&shapes, &stitched, &plane).map_err(|error| mode.relabel(error))?;
            let edge_count = stitched.edge_count();
            (loops, plane, edge_count, space)
        };
        let loop_count = loops.len();
        let style = hatch_style(parameters.style)?;
        let (fill, pattern_type) = match &pattern {
            // The typed pattern name: the library entry supplies the name and the
            // definition rows, the session channel the scale/angle/double. The
            // recorded DXF type follows the source (built-in `1`, user file `2`).
            Some(choice) => {
                parameters.scale_and_angle()?;
                let definition = choice.definition();
                let built = definition.to_hatch_pattern(
                    parameters.spacing,
                    parameters.angle_degrees,
                    parameters.double,
                );
                (HatchFill::Pattern(built), choice.pattern_type())
            }
            None => parameters.fill()?,
        };

        // The unique creation-path constructor: every boundary defect (A.3) is a
        // whole-command refusal here, before a transaction exists.
        let hatch = HatchGeometry::new(loops, plane, style, fill, Vec::new(), None, pattern_type)
            .map_err(|error| mode.relabel(hatch_creation_refusal(error)))?;
        let geometry = SemanticGeometry::Hatch(Box::new(hatch));

        let label = if gradient {
            "create gradient"
        } else {
            "create hatch"
        };
        let mut outcome = match &space {
            SpaceId::Paper(layout) => {
                self.commit_new_layout_entity(session, command, geometry, *layout, label)?
            }
            _ => self.commit_new_entity(session, command, geometry, label)?,
        };
        if let Some(object) = outcome.objects.first().copied() {
            outcome.diagnostics.push(hatch_diagnostic(
                object,
                loop_count,
                stitched_edges,
                style,
                &parameters,
                pattern.as_ref(),
            ));
        }
        Ok(outcome)
    }
}

/// The one space a new hatch may be written into: every boundary entity must
/// be directly addressable in the **same** space, and that space is the entity
/// write target.
///
/// Model space and a paper layout are supported (the hatch lives where its
/// boundary lives, exactly like [`Self::hatch_generate_boundary`] writes next to
/// its source). A block-definition boundary is refused by name: the hatch would
/// be shared by every reference. Mixed spaces are refused rather than picking
/// one silently.
fn hatch_target_space(sources: &[cad_db::DbEntity], prefix: &str) -> CadResult<SpaceId> {
    let mut space: Option<SpaceId> = None;
    for source in sources {
        match &source.space {
            SpaceId::Model | SpaceId::Paper(_) => {}
            SpaceId::Block(block) => {
                return Err(CadError::Unsupported(format!(
                    "{prefix}.block_space: boundary entity {} is inside block definition {}; a \
                     hatch is written next to its boundary in model space or a paper layout",
                    source.id.0, block.0
                )))
            }
        }
        match &space {
            None => space = Some(source.space.clone()),
            Some(existing) if existing != &source.space => {
                return Err(CadError::InvalidInput(format!(
                    "{prefix}.mixed_space: boundary entity {} is in {:?} while an earlier \
                     boundary entity is in {existing:?}; one hatch needs one common space",
                    source.id.0, source.space
                )))
            }
            _ => {}
        }
    }
    Ok(space.unwrap_or(SpaceId::Model))
}

/// Resolve the typed `PATTERN <name>` / `PATTERNFILE <path> [<name>]` selection.
///
/// This is the **same** resolution `HATCHEDIT` uses — the built-in library by
/// name, or the user file first and the built-in table second — so creation and
/// editing cannot drift into two pattern readers. The refusal codes are
/// re-labelled from `hatchedit.*` to `hatch.*` (the command that owns the line),
/// with the message's command name following, so a typed `HATCH` line always
/// answers with its own stable code and still names the sources it searched.
fn resolve_typed_pattern(request: &HatchPatternRequest) -> CadResult<PatternChoice> {
    match request {
        HatchPatternRequest::Named(name) => resolve_builtin_pattern(name)
            .map(PatternChoice::Builtin)
            .map_err(hatch_pattern_refusal),
        HatchPatternRequest::File { path, name, text } => {
            resolve_file_pattern(path, name.as_deref(), Some(text.as_str()))
                .map_err(hatch_pattern_refusal)
        }
    }
}
