//! Block attribute model contracts (ADR-0055).
//!
//! These cover the parts of the M1 slice that live in `cad-db`: geometry
//! validation, the two transform surfaces' rules, and the fact that an
//! `INSERT`'s values stay in the block's local frame while the reference moves.

use super::*;

fn text() -> AttributeText {
    AttributeText {
        position: Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        height: 2.5,
        rotation: 0.25,
        style: StyleId(3),
        width_factor: 1.5,
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
        wrap_width: Some(8.0),
        line_spacing: TextLineSpacing::default(),
        font: Some("arial.ttf".into()),
        oblique: 0.125,
    }
}

fn definition() -> AttributeDefinition {
    AttributeDefinition {
        tag: "PART".into(),
        prompt: "Part?".into(),
        default_value: "A-100".into(),
        flags: AttributeFlags {
            invisible: false,
            constant: false,
            verify: true,
            preset: false,
            locked_position: false,
            annotative: true,
        },
        order: 2,
        text: text(),
        source_handle: Some("2A".into()),
    }
}

fn value() -> AttributeValue {
    AttributeValue {
        tag: "PART".into(),
        value: "A-100".into(),
        flags: AttributeFlags::default(),
        text: text(),
        layer: LayerId(0),
        appearance: AttributeAppearance::default(),
        source_handle: Some("2B".into()),
    }
}

#[test]
fn an_attribute_definition_transforms_like_text() {
    let geometry = SemanticGeometry::AttributeDefinition(Box::new(definition()));
    let transform = Transform3 {
        matrix: [
            [2.0, 0.0, 0.0, 10.0],
            [0.0, 2.0, 0.0, -5.0],
            [0.0, 0.0, 2.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let moved = crate::transform_geometry(&geometry, &transform).expect("similarity accepted");
    let SemanticGeometry::AttributeDefinition(moved) = moved else {
        panic!("expected a definition");
    };
    assert_eq!(moved.tag, "PART");
    assert_eq!(moved.default_value, "A-100");
    assert_eq!(moved.flags, definition().flags);
    assert_eq!(
        moved.text.position,
        Point3 {
            x: 12.0,
            y: -1.0,
            z: 1.0,
        }
    );
    assert!((moved.text.height - 5.0).abs() < 1e-9);
    assert!((moved.text.rotation - 0.25).abs() < 1e-9);
    assert!((moved.text.oblique - 0.125).abs() < 1e-9);
    assert_eq!(moved.text.width_factor, 1.5);
    assert_eq!(moved.text.wrap_width, Some(16.0));
    assert_eq!(moved.text.style, StyleId(3));
    assert_eq!(moved.source_handle.as_deref(), Some("2A"));
}

#[test]
fn an_attribute_definition_refuses_a_mirror_or_non_uniform_scale() {
    let geometry = SemanticGeometry::AttributeDefinition(Box::new(definition()));
    assert!(matches!(
        crate::transform_geometry(&geometry, &Transform3::scale(-1.0)),
        Err(CadError::Unsupported(_))
    ));
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 2.0;
    matrix[1][1] = 3.0;
    assert!(matches!(
        crate::transform_geometry(&geometry, &Transform3 { matrix }),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn moving_an_insert_keeps_its_values_block_local() {
    let geometry = SemanticGeometry::Insert {
        block: BlockId(4),
        transform: Transform3::translation(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }),
        attributes: vec![value()],
    };
    let moved = crate::transform_geometry(
        &geometry,
        &Transform3::translation(Point3 {
            x: 100.0,
            y: 0.0,
            z: 0.0,
        }),
    )
    .expect("insert moves");
    let SemanticGeometry::Insert {
        transform,
        attributes,
        ..
    } = moved
    else {
        panic!("expected an insert");
    };
    // The instance transform composes; the value text is untouched.
    assert_eq!(
        transform.apply_point(Point3::default()),
        Point3 {
            x: 101.0,
            y: 0.0,
            z: 0.0,
        }
    );
    assert_eq!(attributes, vec![value()]);
}

#[test]
fn validation_refuses_an_empty_tag_and_unusable_text() {
    let mut empty_tag = definition();
    empty_tag.tag = "  ".into();
    assert!(
        crate::validate_geometry(&SemanticGeometry::AttributeDefinition(Box::new(empty_tag)))
            .is_err()
    );

    let mut bad_height = definition();
    bad_height.text.height = 0.0;
    assert!(
        crate::validate_geometry(&SemanticGeometry::AttributeDefinition(Box::new(bad_height)))
            .is_err()
    );

    let mut bad_value = value();
    bad_value.text.width_factor = 0.0;
    assert!(crate::validate_geometry(&SemanticGeometry::Insert {
        block: BlockId(1),
        transform: Transform3::identity(),
        attributes: vec![bad_value],
    })
    .is_err());

    // A duplicate tag is legal at the model level.
    let duplicate = SemanticGeometry::Insert {
        block: BlockId(1),
        transform: Transform3::identity(),
        attributes: vec![value(), value()],
    };
    assert!(crate::validate_geometry(&duplicate).is_ok());
}
