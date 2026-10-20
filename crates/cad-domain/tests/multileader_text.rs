//! `MultiLeaderGeometry::with_text`: in-place annotation replacement that keeps
//! the derived display consistent with the stored string (ADR-0021 D4).
//!
//! The database's `update_entity` replaces an entity **without** rebuilding
//! `display`, and `validate_geometry` only checks that the stored display is
//! finite — never that it still matches `text`. These tests pin the domain
//! construct that closes that gap: the text child changes, everything else is
//! carried over, and disabling content kinds or an empty string write nothing.

use cad_domain::{
    CadError, MLeaderContent, MLeaderLine, MLeaderPathType, MLeaderReserved, MLeaderRoot,
    MLeaderStyleValues, MLeaderText, MultiLeaderGeometry, MultiLeaderParts, PlaneFrame, Point3,
    SemanticGeometry, StyleId,
};

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn style() -> MLeaderStyleValues {
    MLeaderStyleValues {
        path_type: MLeaderPathType::StraightLineSegments,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 2.0,
        landing_gap: 0.1,
        arrowhead_size: 0.5,
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: Some("arial.ttf".into()),
        scale_factor: 1.0,
    }
}

/// A created leader with one text and a straight two-point leader line.
fn text_leader(text: &str) -> MultiLeaderGeometry {
    MultiLeaderGeometry::created(
        style(),
        "Standard".into(),
        2,
        point(0.0, 0.0),
        point(10.0, 5.0),
        text.into(),
    )
    .expect("a valid created leader")
}

/// A content-less leader (matches the migrated `172 = 0` shape).
fn content_less_leader() -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: style(),
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 5.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![point(0.0, 0.0), point(8.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    geometry
}

/// The strings of the display's `Text` children, in display order.
fn display_texts(geometry: &MultiLeaderGeometry) -> Vec<String> {
    geometry
        .display()
        .iter()
        .filter_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn with_text_changes_the_string_and_rebuilds_the_display() {
    let before = text_leader("OLD");
    assert_eq!(display_texts(&before), vec!["OLD".to_string()]);

    let after = before
        .with_text("NEW\\PPARA")
        .expect("a non-empty text is accepted");

    assert_eq!(after.text().string, "NEW\\PPARA");
    // The display's text child was regenerated, not left stale.
    assert_eq!(display_texts(&after), vec!["NEW\\PPARA".to_string()]);
    // The leader primitives are unchanged in count (one rebuild, same shape).
    assert_eq!(after.display().len(), before.display().len());
}

#[test]
fn with_text_preserves_the_mask_style_width_and_attachment() {
    let before = text_leader("OLD");
    let after = before.with_text("NEW").expect("the replacement");

    assert_eq!(after.override_mask(), before.override_mask());
    assert_eq!(after.style(), before.style());
    assert_eq!(after.style_name(), before.style_name());
    assert_eq!(after.content(), MLeaderContent::Text);
    assert_eq!(after.text().width, before.text().width);
    assert_eq!(after.text().location, before.text().location);
    assert_eq!(after.text().rotation, before.text().rotation);
    assert_eq!(after.text().height, before.text().height);
    assert_eq!(
        after.text().attachment_point,
        before.text().attachment_point
    );
    assert_eq!(after.text().left_attachment, before.text().left_attachment);
    assert_eq!(after.roots(), before.roots());
    assert_eq!(after.lines(), before.lines());
    assert_eq!(after.reserved(), before.reserved());
    assert_eq!(after.plane(), before.plane());
    // The receiver is untouched.
    assert_eq!(before.text().string, "OLD");
    assert_eq!(display_texts(&before), vec!["OLD".to_string()]);
}

#[test]
fn with_text_refuses_non_text_content_and_an_empty_string() {
    let content_less = content_less_leader();
    let error = content_less.with_text("NEW").unwrap_err();
    assert!(
        matches!(&error, CadError::Unsupported(message) if message.contains("mleader.content_not_text")),
        "{error:?}"
    );
    // Nothing was written to the receiver.
    assert_eq!(content_less.text().string, "");

    let leader = text_leader("OLD");
    for empty in ["", "   ", "\t\n"] {
        let error = leader.with_text(empty).unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.contains("mleader.text_required")),
            "{empty:?}: {error:?}"
        );
    }
    assert_eq!(leader.text().string, "OLD");
    assert_eq!(display_texts(&leader), vec!["OLD".to_string()]);
}
