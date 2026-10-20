//! SHX glyph bulge (`shx::tessellate_bulge`) vs the shared analytic table.
//!
//! This is the seventh copy of the bulge -> arc math, and the one with a
//! **different input range**: SHX shape data stores a bulge in `-127..=127`
//! which the caller divides by 127 (`bulge / 127.0`), and `tessellate_bulge`
//! then clamps `|bulge|` to 1. A major arc (`|bulge| > 1`) is therefore not
//! reachable here; the clamp is asserted explicitly instead of being silently
//! compared against major-arc expectations.
//!
//! The sampler also rounds its angular step up to whole 10-degree steps with at
//! least one step, so for the very small bulge it degenerates to the bare chord
//! and cannot discriminate the apex side — that case is skipped for the apex
//! check only (the on-circle and span checks still run). Everything else is
//! checked exactly like the other copies.
//!
//! Cases come from `cad-domain/tests/support/bulge_cases.rs`.

#[path = "../../../cad-domain/tests/support/bulge_cases.rs"]
mod bulge_cases;

use crate::shx::tessellate_bulge;

/// The SHX sampler's angular step, in radians: whole 10-degree steps.
fn shx_half_step(case: &bulge_cases::Case) -> f64 {
    let span = case.included_angle.abs();
    let steps = (span / (std::f64::consts::PI / 18.0)).floor().max(1.0);
    0.5 * span / steps
}

#[test]
fn shx_glyph_bulge_samples_match_the_analytic_arc() {
    bulge_cases::assert_table_is_hand_computed();
    for case in bulge_cases::table() {
        // The SHX input range: |bulge| is clamped to 1 by the decoder.
        let bulge = case.bulge.clamp(-1.0, 1.0);
        let clamped = bulge_cases::analytic(&case.name, case.p0, case.p1, bulge);
        let samples = tessellate_bulge(case.p0, case.p1, bulge);
        bulge_cases::assert_samples(
            "cad-representation SHX glyph (tessellate_bulge)",
            &clamped,
            &samples,
            1e-9 * (1.0 + clamped.radius),
        );
        if samples.len() < 3 {
            // One 10-degree step leaves only the endpoints: no sample can show
            // which side the arc bows to, so the apex check is not applicable.
            assert!(
                clamped.included_angle.abs() < std::f64::consts::PI / 18.0,
                "{}: {}-sample arc sweeps {} rad, expected a sub-step span",
                case.name,
                samples.len(),
                clamped.included_angle
            );
            continue;
        }
        let tol = 2.0 * clamped.radius * (1.0 - shx_half_step(&clamped).cos()) + 1e-9;
        bulge_cases::assert_apex(
            "cad-representation SHX glyph (tessellate_bulge)",
            &clamped,
            &samples,
            tol,
        );
    }
}

/// The SHX decoder clamps `|bulge|` to 1; a major arc must render as the
/// semicircle its clamped bulge describes, never as a mirrored arc.
#[test]
fn shx_glyph_bulge_is_clamped_to_a_semicircle() {
    let case = bulge_cases::analytic("major", [0.0, 0.0], [2.0, 0.0], 2.0);
    let clamped = bulge_cases::analytic("major", case.p0, case.p1, 1.0);
    let samples = tessellate_bulge(case.p0, case.p1, 2.0);
    bulge_cases::assert_samples(
        "cad-representation SHX clamp (tessellate_bulge)",
        &clamped,
        &samples,
        1e-9,
    );
    // r = |chord| / 2 for the clamped semicircle, not the 1.25 of bulge 2.
    assert!((clamped.radius - 1.0).abs() < 1e-12);
}
