//! HATCH boundary and fill conversion (ADR 0018 B).
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
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
        let seeds: Vec<[f64; 2]> = h.seed_points.iter().map(|p| [p.x, p.y]).collect();
        // Group 47 is only written for a derived boundary (ADR B3/U-10). The
        // reader always yields an `f64`, so presence is inferred from a non-zero
        // finite value; 0 means "not present".
        let pixel_size = (h.pixel_size.is_finite() && h.pixel_size != 0.0).then_some(h.pixel_size);
        let (geometry, reasons) = HatchGeometry::from_import(
            loops,
            plane,
            hatch_style(h.style),
            fill,
            seeds,
            pixel_size,
            hatch_pattern_type(h.pattern_type),
        );
        let completeness = if reasons.is_empty() {
            Completeness::Complete
        } else {
            Completeness::Partial(reasons)
        };
        (SemanticGeometry::Hatch(Box::new(geometry)), completeness)
    }
}
