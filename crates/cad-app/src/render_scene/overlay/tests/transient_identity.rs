//! Transient-overlay identity (ADR-0028 D1).
//!
//! A preview batch is drawn, not committed: its `SelectionRef` carries a
//! synthetic id derived from the overlay kind. It names no database entity, has
//! no handle concept and can never be a pick/selection identity.

use super::*;

/// ADR-0028 D1: a transient overlay batch carries a synthetic source id — not a
/// real database `EntityId` and with no handle concept — so a preview can never
/// be click-selected and is never part of the pick catalogue.
#[test]
fn preview_batches_never_carry_a_pickable_entity_identity() {
    let db = drawing();
    let preview = MeasurementPreview {
        kind: crate::measure_tool::MeasurementToolKind::Distance,
        points: vec![p(0.0, 0.0), p(1.0, 0.0)],
        cursor: None,
        remaining: 0,
        ready: true,
    };
    // No snap hints, default options: the same overlay the host draws.
    let overlay = preview_overlay(
        Some(&preview),
        DocumentId(1),
        false,
        &PreviewOptions::default(),
    );
    let catalogue: Vec<SelectionRef> = crate::picking::drawing_pick_items(&db, DocumentId(1))
        .into_iter()
        .map(|item| item.source)
        .collect();
    for source in overlay.batches.iter().flat_map(|b| &b.sources) {
        assert!(db.entity(source.entity).is_none(), "no committed entity");
        assert!(source.instance.0.is_empty());
        assert!(source.sub_element.is_none());
        assert!(!catalogue.contains(source), "not a pick catalogue ref");
    }
}
