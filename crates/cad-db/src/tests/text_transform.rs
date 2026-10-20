//! `Text` transform contract (ADR 0019 decision B).
//!
//! A `Text` accepts a **positive-determinant similarity**: translation moves
//! `position`, rotation accumulates into `rotation` and a uniform scale
//! multiplies `height`; the string, style, font and alignment are preserved.
//! A non-uniform scale (stretched glyphs) and a mirror (reflected glyphs) cannot
//! be represented by the flat-text domain, so both refuse the whole entity with
//! a named `Unsupported` diagnostic. `cad-geometry`'s snap face applies the same
//! guards; the cross-face parity test lives in `cad-app`
//! (`tests/text_transform_parity.rs`).

use super::*;

fn text_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "AB".into(),
            position: Point3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            style: StyleId(7),
            height: 2.5,
            rotation: 0.25,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
        },
        draw_order: 1,
    }
}

fn stored_text(db: &DrawingDatabase) -> (String, Point3, f64, f64) {
    match &db.entity(EntityId(1)).expect("the text entity").geometry {
        SemanticGeometry::Text {
            text,
            position,
            height,
            rotation,
            style,
            font,
            h_align,
            v_align,
        } => {
            assert_eq!(*style, StyleId(7), "the style identity is preserved");
            assert_eq!(font.as_deref(), Some("arial.ttf"), "the font is preserved");
            assert_eq!(*h_align, TextAlignH::Center, "alignment is preserved");
            assert_eq!(*v_align, TextAlignV::Middle, "alignment is preserved");
            (text.clone(), *position, *height, *rotation)
        }
        other => panic!("expected text, got {other:?}"),
    }
}

fn seeded_db() -> DrawingDatabase {
    let mut db = writable_db();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(EntityId(1), Some(text_entity(1)))],
    )
    .unwrap();
    db
}

#[test]
fn text_translation_moves_only_the_position() {
    let mut db = seeded_db();
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    tx.transform_entity(
        EntityId(1),
        &Transform3::translation(Point3 {
            x: 10.0,
            y: -4.0,
            z: 1.5,
        }),
    )
    .expect("text translates");
    tx.commit().unwrap();
    let (text, position, height, rotation) = stored_text(&db);
    assert_eq!(text, "AB");
    assert_eq!(
        position,
        Point3 {
            x: 11.0,
            y: -2.0,
            z: 4.5
        }
    );
    assert!((height - 2.5).abs() < 1e-12, "translation keeps the height");
    assert!(
        (rotation - 0.25).abs() < 1e-12,
        "translation keeps rotation"
    );
}

#[test]
fn text_rotation_accumulates_about_z_and_keeps_the_height() {
    let mut db = seeded_db();
    // A quarter turn about +Z through the origin.
    let quarter = Transform3 {
        matrix: [
            [0.0, -1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let mut tx = db
        .begin_drawing_transaction("rotate", TransactionId(3))
        .unwrap();
    tx.transform_entity(EntityId(1), &quarter)
        .expect("text rotates");
    tx.commit().unwrap();
    let (_, position, height, rotation) = stored_text(&db);
    assert!(
        (position.x + 2.0).abs() < 1e-12 && (position.y - 1.0).abs() < 1e-12,
        "the position rotates: {position:?}"
    );
    assert!(
        (height - 2.5).abs() < 1e-12,
        "a rotation keeps the glyph height"
    );
    assert!(
        (rotation - (0.25 + std::f64::consts::FRAC_PI_2)).abs() < 1e-12,
        "the stored rotation accumulates: {rotation}"
    );
}

#[test]
fn text_uniform_scale_multiplies_the_height() {
    let mut db = seeded_db();
    let mut tx = db
        .begin_drawing_transaction("scale", TransactionId(4))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::scale(3.0))
        .expect("text scales uniformly");
    tx.commit().unwrap();
    let (_, position, height, rotation) = stored_text(&db);
    assert!((position.x - 3.0).abs() < 1e-12, "{position:?}");
    assert!((height - 7.5).abs() < 1e-12, "height × 3: {height}");
    assert!(
        (rotation - 0.25).abs() < 1e-12,
        "a uniform scale keeps the rotation"
    );
}

#[test]
fn text_refuses_non_uniform_scale_and_mirror_with_a_named_diagnostic() {
    let mut non_uniform = Transform3::identity().matrix;
    non_uniform[0][0] = 2.0;
    non_uniform[1][1] = 0.5;
    let mirror = Transform3::scale(-1.0);
    for (label, transform) in [
        (
            "non-uniform scale",
            Transform3 {
                matrix: non_uniform,
            },
        ),
        ("mirror", mirror),
    ] {
        let mut db = seeded_db();
        let mut tx = db
            .begin_drawing_transaction("reject", TransactionId(5))
            .unwrap();
        let error = tx.transform_entity(EntityId(1), &transform).unwrap_err();
        match error {
            CadError::Unsupported(reason) => {
                assert!(
                    reason.contains("text"),
                    "{label}: the refusal must name the kind: {reason}"
                );
            }
            other => panic!("{label}: expected Unsupported, got {other:?}"),
        }
        // The failed stage left nothing; commit is a no-op and the entity keeps its
        // exact source geometry.
        assert_eq!(tx.staged_len(), 0, "{label}");
        let changes = tx.commit().unwrap();
        assert!(changes.is_empty(), "{label}");
        let (text, position, height, rotation) = stored_text(&db);
        assert_eq!(text, "AB");
        assert_eq!(position.x, 1.0);
        assert_eq!(position.y, 2.0);
        assert_eq!(height, 2.5);
        assert_eq!(rotation, 0.25);
    }
}
