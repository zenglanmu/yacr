//! HATCH boundary and fill conversion (ADR 0018 B).
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// Convert a HATCH into a first-class [`SemanticGeometry::Hatch`] and record
    /// the pattern-row provenance diagnostic.
    ///
    /// A predefined (`76=1`) pattern whose file carries no definition rows is
    /// imported with the built-in transcribed table's rows (ADR U-1). Those rows
    /// are **ours, not vendor-verified**, so that substitution is reported as
    /// [`HATCH_PATTERN_TABLE_CODE`] — the drawing must not look like it was
    /// drawn from the file's own definition. The diagnostic is recorded once per
    /// import (same `reported_reasons` fold as the mesh reasons), and only when
    /// the rows were actually used: a solid or gradient fill draws no pattern
    /// rows, so it gets no provenance claim.
    pub(crate) fn hatch_semantics(&mut self, h: &Hatch) -> (SemanticGeometry, Completeness) {
        let out = Self::hatch_geometry(h);
        let (SemanticGeometry::Hatch(hatch), _) = &out else {
            return out;
        };
        if !matches!(hatch.fill(), HatchFill::Pattern(_)) {
            return out;
        }
        let PatternRowSource::Table(table) = pattern_row_source(h) else {
            return out;
        };
        if self
            .reported_reasons
            .insert(format!("{HATCH_PATTERN_TABLE_CODE}:{}", table.name))
        {
            self.diagnostics.push(Diagnostic {
                object: None,
                code: HATCH_PATTERN_TABLE_CODE.into(),
                message: format!(
                    "HATCH pattern '{}' has no line definition rows in the file; the rows come \
                     from the built-in transcribed table for '{}' (our transcription, ADR U-1, \
                     not vendor-verified)",
                    h.pattern.name, table.name
                ),
            });
        }
        out
    }

    /// Convert a HATCH into a first-class [`SemanticGeometry::Hatch`].
    ///
    /// The display (boundary polylines plus the style-filtered fill, pattern or
    /// gradient) is derived by [`HatchGeometry::from_import`]; a fill that
    /// cannot be generated is reported as `Partial` with the boundaries still
    /// present, never faked and never rejected (ADR B6).
    pub(crate) fn hatch_geometry(h: &Hatch) -> (SemanticGeometry, Completeness) {
        let plane = HatchPlane::new(p3(h.normal), h.elevation);
        let params = TessellationParams {
            tolerance: hatch_tolerance(h),
            ..TessellationParams::default()
        };
        let mut loops: Vec<HatchLoop> = Vec::new();
        for path in &h.paths {
            let mut points: Vec<[f64; 2]> = Vec::new();
            for edge in &path.edges {
                append_boundary_edge(edge, &mut points, &params);
            }
            dedup_loop(&mut points);
            if points.len() < 3 {
                continue;
            }
            // The edges above are sampled into the loop, so the stored bulges are
            // zero; the analytic source edges are kept separately for export.
            let bulges = vec![0.0; points.len()];
            loops.push(HatchLoop {
                points,
                bulges,
                flags: HatchLoopFlags(path.flags.bits()),
                source_edges: import_source_edges(path),
            });
        }
        if loops.is_empty() {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbHatch".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["hatch has no usable boundary".into()]),
            );
        }
        let solid = h.is_solid || h.pattern.name.eq_ignore_ascii_case("SOLID");
        // A gradient is checked before the solid flag: a gradient HATCH is
        // stored as a "solid" hatch with gradient metadata, so testing `solid`
        // first would silently draw it as a single flat colour.
        let fill = match import_gradient(h) {
            Some(gradient) => HatchFill::Gradient(gradient),
            None if solid => HatchFill::Solid,
            None => HatchFill::Pattern(import_pattern(h)),
        };
        let pattern_fill = matches!(fill, HatchFill::Pattern(_));
        let seeds: Vec<[f64; 2]> = h.seed_points.iter().map(|p| [p.x, p.y]).collect();
        // Group 47 is only written for a derived boundary (ADR B3/U-10). The
        // reader always yields an `f64`, so presence is inferred from a non-zero
        // finite value; 0 means "not present".
        let pixel_size = (h.pixel_size.is_finite() && h.pixel_size != 0.0).then_some(h.pixel_size);
        let (geometry, mut reasons) = HatchGeometry::from_import(
            loops,
            plane,
            hatch_style(h.style),
            fill,
            seeds,
            pixel_size,
            hatch_pattern_type(h.pattern_type),
        );
        // A predefined (`76=1`) name outside the implemented table keeps no rows
        // — nothing is fabricated — so the hatch stays `Partial`. The display's
        // generic reason is extended with the pattern name instead of inventing
        // a second vocabulary.
        if pattern_fill {
            if let PatternRowSource::Unresolved(name) = pattern_row_source(h) {
                reasons = name_unresolved_pattern(reasons, name);
            }
        }
        let completeness = if reasons.is_empty() {
            Completeness::Complete
        } else {
            Completeness::Partial(reasons)
        };
        (SemanticGeometry::Hatch(Box::new(geometry)), completeness)
    }
}

/// Stable diagnostic code: the HATCH pattern rows were taken from the built-in
/// transcribed predefined table because the file carried none.
///
/// The rows are our hand transcription (ADR U-1), not vendor-verified, which is
/// why the substitution is reported instead of staying silent.
pub(crate) const HATCH_PATTERN_TABLE_CODE: &str = "import.hatch_pattern_table";

/// The display's reason for a pattern with no line families.
///
/// `cad-domain` owns the wording (`HatchGeometry::build_display`); it is repeated
/// here only so the importer can extend it with the unresolved name instead of
/// adding a second, competing phrase.
const NO_LINE_FAMILIES: &str = "hatch pattern has no line families";

/// Extend [`NO_LINE_FAMILIES`] with the predefined name this build does not
/// implement.
///
/// The hatch still shows its boundaries and no pattern rows are invented; the
/// reason now says *which* name is unimplemented, which the generic phrase did
/// not.
fn name_unresolved_pattern(mut reasons: Vec<String>, name: &str) -> Vec<String> {
    let extended = format!(
        "{NO_LINE_FAMILIES}: predefined pattern '{name}' is not in the implemented pattern table"
    );
    match reasons
        .iter_mut()
        .find(|reason| reason.as_str() == NO_LINE_FAMILIES)
    {
        Some(slot) => *slot = extended,
        None => reasons.push(extended),
    }
    reasons
}
