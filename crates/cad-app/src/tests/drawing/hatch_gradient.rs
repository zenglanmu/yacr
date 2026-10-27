//! Gradient HATCH creation contract tests (pattern type code `3`).
//!
//! Pins the **creation channel** of a two-colour gradient: the two stops and
//! their ACI-resolved sRGB triples, the kind name actually written, the exact
//! degrees -> radians conversion (and its export/reimport round trip), the
//! fixed `reserved`/`shift`, the `76=1` pattern type a gradient records, the
//! explicit refusal surface (nothing written, no transaction) and the branch
//! ordering that keeps the pattern spacing out of a gradient.
//!
//! Everything below is **our implementation** (ADR U-1): no vendor file,
//! reference renderer or golden image was compared. `pattern_type=Predefined`
//! (`76=1`) for a gradient follows the DXF "solid fill + 450-470 block" shape and
//! is an **unverified vendor convention** (declared), not a round-tripped claim.

use super::hatch::{
    assert_refused, circle, entity_count, filled_area, hatch_app, run, set_flag, set_number,
    the_hatch, undo_depth,
};
use super::*;

use cad_domain::hatch::GradientStop;
use cad_domain::HatchGradient;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// A closed axis-aligned rectangle `[x0, x1] x [y0, y1]`, so the ramp covers a
/// non-square extent (which also keeps `SPHERICAL` visibly distinct from a
/// symmetric case).
fn rect(id: u128, x0: f64, x1: f64, y0: f64, y1: f64, z: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Polyline {
            points: vec![p3(x0, y0, z), p3(x1, y0, z), p3(x1, y1, z), p3(x0, y1, z)],
            bulges: vec![0.0; 4],
            closed: true,
        },
    )
}

/// Configure a gradient: pattern type code `3`, the kind, the **degrees** angle
/// and the two ACI stop colours.
fn gradient(session: &mut SessionState, kind: f64, angle_degrees: f64, color_a: f64, color_b: f64) {
    set_number(
        session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_GRADIENT,
    );
    set_number(session, ToolParameter::HatchGradientKind, kind);
    set_number(session, ToolParameter::HatchGradientAngle, angle_degrees);
    set_number(session, ToolParameter::HatchGradientColorA, color_a);
    set_number(session, ToolParameter::HatchGradientColorB, color_b);
}

/// The two-colour gradient stored on the created hatch.
fn two_color(fill: &HatchFill) -> &HatchGradient {
    let HatchFill::Gradient(gradient) = fill else {
        panic!("expected a gradient fill, got {fill:?}");
    };
    assert!(
        matches!(gradient, HatchGradient::TwoColor { .. }),
        "a created gradient is the two-colour variant: {gradient:?}"
    );
    gradient
}

#[test]
fn a_created_gradient_writes_two_stops_linear_radians_and_76_one() {
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    gradient(&mut session, HATCH_GRADIENT_LINEAR, 30.0, 1.0, 5.0);
    let out = run(&mut app, &mut session, &[1]).expect("HATCH succeeds");

    let hatch = the_hatch(&app);
    assert_eq!(
        hatch.pattern_type(),
        HatchPatternType::Predefined,
        "a gradient records 76=1 (solid fill + 450-470 block)"
    );
    let HatchGradient::TwoColor {
        name,
        reserved,
        angle_raw,
        shift,
        stops,
    } = two_color(hatch.fill())
    else {
        unreachable!("checked by two_color")
    };
    // The channel spelling of the kind is written, never the DXF `CYLINDER`
    // alias the domain bakes as spherical.
    assert_eq!(name, "LINEAR");
    assert_eq!(*reserved, 0, "the reserved field is fixed 0");
    assert_eq!(*shift, 0.0, "the ramp start is fixed 0");
    // The one conversion of this path: degrees on the channel, radians stored.
    assert!(
        (*angle_raw - 30.0f64.to_radians()).abs() < 1e-12,
        "angle {angle_raw} is not 30 degrees in radians"
    );
    // The ACI indices resolve through the canonical palette: ACI 1 = red,
    // ACI 5 = blue, at the two ends of the ramp.
    assert_eq!(
        stops,
        &vec![
            GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            },
            GradientStop {
                value: 1.0,
                rgb: [0, 0, 255],
            },
        ]
    );

    // The gradient is baked into a real fill mesh (not a boundary-only hatch).
    assert!(filled_area(hatch) > 0.0, "the gradient fill is baked");

    // One transaction / one undo step, like every other HATCH.
    assert_eq!(out.objects.len(), 1);
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(entity_count(&app), 2, "the boundary entity is retained");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "one undo removes the gradient");
    assert_eq!(undo_depth(&app), 0);

    // The summary diagnostic echoes the gradient instead of a pattern phrase.
    let message = out
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.hatch")
        .map(|diagnostic| diagnostic.message.clone())
        .expect("a hatch summary diagnostic");
    assert!(
        message.contains("gradient LINEAR (angle 30 deg, ACI 1 -> 5, stops 0/1)"),
        "diagnostic must echo the gradient: {message}"
    );
    assert!(
        message.contains("associative hatch NOT IMPLEMENTED"),
        "the deferral must stay: {message}"
    );
}

#[test]
fn the_degrees_to_radians_conversion_survives_export_and_reimport() {
    // The pin between the two conventions: the creation channel takes degrees
    // and stores radians once; the import path stores group 460 verbatim. If
    // either side ever converts again, the reimported angle stops being 30
    // degrees in radians.
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 10.0)]);
    gradient(&mut session, HATCH_GRADIENT_LINEAR, 30.0, 1.0, 5.0);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let created = the_hatch(&app).clone();
    let HatchGradient::TwoColor { angle_raw, .. } = two_color(created.fill()) else {
        unreachable!("two_color checked the variant")
    };
    assert!((*angle_raw - 30.0f64.to_radians()).abs() < 1e-12);

    let (bytes, report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    assert!(
        report.counts.converted >= 1,
        "a hatch is always Converted: {:?}",
        report.counts
    );

    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports");
    let (entity, hatch) = imported
        .database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some((*entity, hatch.as_ref())),
            _ => None,
        })
        .expect("the export re-imports as a first-class hatch");
    assert_eq!(entity.object.type_key, "AcDbHatch");
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined);
    let HatchGradient::TwoColor {
        name,
        reserved,
        angle_raw,
        shift,
        stops,
    } = two_color(hatch.fill())
    else {
        unreachable!("two_color checked the variant")
    };
    assert_eq!(name, "LINEAR");
    assert_eq!(*reserved, 0);
    assert_eq!(*shift, 0.0);
    assert!(
        (*angle_raw - 30.0f64.to_radians()).abs() < 1e-9,
        "the reimported angle must still be 30 degrees in radians, got {angle_raw}"
    );
    assert_eq!(stops.len(), 2, "both stops round-trip");
    assert_eq!(stops[0].rgb, [255, 0, 0]);
    assert_eq!(stops[1].rgb, [0, 0, 255]);
    assert!((stops[0].value - 0.0).abs() < 1e-9);
    assert!((stops[1].value - 1.0).abs() < 1e-9);
}

#[test]
fn the_spherical_kind_writes_the_spherical_name() {
    // `SPHERICAL` is the only radial spelling this build offers; the DXF
    // `CYLINDER` alias is never written (the domain bakes it as spherical).
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    gradient(
        &mut session,
        HATCH_GRADIENT_SPHERICAL,
        0.0,
        HATCH_GRADIENT_DEFAULT_ACI_A,
        HATCH_GRADIENT_DEFAULT_ACI_B,
    );
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    let HatchGradient::TwoColor { name, .. } = two_color(hatch.fill()) else {
        unreachable!("two_color checked the variant")
    };
    assert_eq!(name, "SPHERICAL");
    // Declared limitation: on a rectangle the radial ramp still covers the
    // bounding extent, so it looks flat along neither axis but is not a circle.
    assert!(filled_area(hatch) > 0.0, "the radial ramp is baked");
}

#[test]
fn a_gradient_refuses_an_out_of_set_kind_colour_angle_or_pattern_type() {
    // The typed channel refuses every one of these at set time; each case injects
    // the value past the channel so the **command** refusal is what is pinned
    // (defence in depth), together with "nothing written, no transaction".
    let cases: [(&str, ToolParameter, ToolValue, &str); 7] = [
        (
            "kind 2",
            ToolParameter::HatchGradientKind,
            ToolValue::Number(2.0),
            "hatchGradientKind",
        ),
        (
            "colour 256",
            ToolParameter::HatchGradientColorA,
            ToolValue::Number(256.0),
            "hatchGradientColorA",
        ),
        (
            "colour -1",
            ToolParameter::HatchGradientColorB,
            ToolValue::Number(-1.0),
            "hatchGradientColorB",
        ),
        (
            "colour 1.5",
            ToolParameter::HatchGradientColorA,
            ToolValue::Number(1.5),
            "hatchGradientColorA",
        ),
        (
            "non-finite angle",
            ToolParameter::HatchGradientAngle,
            ToolValue::Number(f64::NAN),
            "hatchGradientAngle",
        ),
        (
            "infinite angle",
            ToolParameter::HatchGradientAngle,
            ToolValue::Number(f64::INFINITY),
            "hatchGradientAngle",
        ),
        (
            "pattern type 4",
            ToolParameter::HatchPatternType,
            ToolValue::Number(4.0),
            "hatchPatternType",
        ),
    ];
    for (label, parameter, value, needle) in cases {
        let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
        // A fully valid gradient first, so only the injected value can refuse.
        gradient(&mut session, HATCH_GRADIENT_LINEAR, 30.0, 1.0, 5.0);
        session.force_tool_parameter(parameter, value);
        assert_refused(&mut app, &mut session, &[1], needle);
        assert_eq!(entity_count(&app), 1, "{label}: nothing written");
        assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
        // The channel itself refuses the same value (a non-gradient default set
        // is refused too, so the command never has to trust it).
        assert!(
            session.set_tool_parameter(parameter, value).is_err(),
            "{label}: the typed channel must refuse {value:?}"
        );
    }
}

#[test]
fn a_gradient_refuses_the_aci_zero_placeholder_colour() {
    // ACI 0 is ByBlock: it has no sRGB triple, so it cannot be a stop colour. The
    // channel accepts the documented ACI domain (`0..=255`) and the command
    // refuses the placeholder **by name** instead of writing black.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    gradient(&mut session, HATCH_GRADIENT_LINEAR, 0.0, 0.0, 5.0);
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        "HATCH gradient colour A ACI 0 is the ByBlock placeholder",
    );
    assert_eq!(entity_count(&app), 1, "nothing written");
    assert_eq!(undo_depth(&app), 0, "no transaction");
    // The other stop is refused the same way.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    gradient(&mut session, HATCH_GRADIENT_LINEAR, 0.0, 1.0, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        "HATCH gradient colour B ACI 0 is the ByBlock placeholder",
    );
    assert_eq!(entity_count(&app), 1, "nothing written");
    assert_eq!(undo_depth(&app), 0, "no transaction");
}

#[test]
fn a_gradient_ignores_the_pattern_spacing_angle_and_double_flag() {
    // Documents the branch ordering: the gradient branch returns before
    // `scale_and_angle`, so a stale zero spacing (which has no line family and
    // is refused for the pattern kinds) must not refuse a gradient. The double
    // flag and the style are equally irrelevant to a gradient definition.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    gradient(&mut session, HATCH_GRADIENT_LINEAR, 30.0, 1.0, 5.0);
    session.force_tool_parameter(ToolParameter::HatchSpacing, ToolValue::Number(0.0));
    session.force_tool_parameter(ToolParameter::HatchAngle, ToolValue::Number(f64::NAN));
    set_flag(&mut session, ToolParameter::HatchDouble, true);
    set_number(&mut session, ToolParameter::HatchStyle, HATCH_STYLE_IGNORE);
    run(&mut app, &mut session, &[1]).expect("a gradient ignores the pattern spacing/angle");

    let hatch = the_hatch(&app);
    let HatchGradient::TwoColor {
        name, angle_raw, ..
    } = two_color(hatch.fill())
    else {
        unreachable!("two_color checked the variant")
    };
    assert_eq!(name, "LINEAR");
    assert!(
        (*angle_raw - 30.0f64.to_radians()).abs() < 1e-12,
        "the gradient angle is its own channel, not the pattern angle"
    );
    // The same injected values still refuse a user-defined pattern, so this is a
    // branch-ordering fact rather than a weakened validation.
    let (mut app, mut session) = hatch_app(vec![rect(1, 0.3, 10.3, 0.7, 10.7, 0.0)]);
    set_number(
        &mut session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_USER_DEFINED,
    );
    session.force_tool_parameter(ToolParameter::HatchSpacing, ToolValue::Number(0.0));
    assert_refused(
        &mut app,
        &mut session,
        &[1],
        "tool parameter 'hatchSpacing' must be finite and > 0",
    );
}
