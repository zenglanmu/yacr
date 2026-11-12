//! The `XCLIP` display attribute (`EntityRenderAttributes::clip`).
//!
//! The clip boundary is display state, so these tests pin the same contract the
//! other display attributes have: one revision + `ChangeMask::STYLE` per write,
//! a display-state round trip an undo can restore, and delete cleanup.

use super::*;

/// Insert entity 1 through the sanctioned path and return the database.
fn writable_db_with_line() -> DrawingDatabase {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create line", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.commit().unwrap();
    db
}

fn sample_clip() -> BlockClip {
    BlockClip {
        vertices: vec![[0.0, 0.0], [2.0, 1.0]],
        inside: true,
        visible: true,
    }
}

#[test]
fn render_attributes_default_to_no_clip() {
    assert_eq!(EntityRenderAttributes::default().clip, None);
}

#[test]
fn clip_write_is_one_revision_and_a_style_mask() {
    let mut db = writable_db_with_line();
    let before = db.revision();
    let mut attributes = db.entity_render_attributes(EntityId(1));
    attributes.clip = Some(sample_clip());

    let mut tx = db
        .begin_drawing_transaction("xclip", TransactionId(2))
        .unwrap();
    tx.update_entity_display(EntityId(1), attributes).unwrap();
    let changes = tx.commit().unwrap();

    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision bump");
    assert_eq!(
        db.entity_render_attributes(EntityId(1)).clip,
        Some(sample_clip())
    );
    // The entity row is unchanged, but the update entry must carry STYLE so the
    // representation is rebuilt with the new boundary.
    let mask = changes
        .changes
        .iter()
        .find_map(|change| match change {
            ObjectChange::Update(object, mask) if object.0 == 1 => Some(*mask),
            _ => None,
        })
        .expect("an update for entity 1");
    assert!(mask.contains(ChangeMask::STYLE), "got {mask:?}");
    assert!(mask.invalidates_representation());
}

#[test]
fn clip_display_state_round_trips_and_restores() {
    let mut db = writable_db_with_line();
    let before_state = db.entity_display_state(EntityId(1));
    assert_eq!(before_state.attributes.clip, None);

    let mut attributes = db.entity_render_attributes(EntityId(1));
    attributes.clip = Some(sample_clip());
    let mut tx = db
        .begin_drawing_transaction("xclip", TransactionId(2))
        .unwrap();
    tx.update_entity_display(EntityId(1), attributes).unwrap();
    tx.commit().unwrap();
    let after_state = db.entity_display_state(EntityId(1));
    assert_eq!(after_state.attributes.clip, Some(sample_clip()));

    // An undo restores the captured display state: the boundary disappears.
    db.restore_entity_display_state(EntityId(1), &before_state);
    assert_eq!(db.entity_render_attributes(EntityId(1)).clip, None);
}

#[test]
fn deleting_the_entity_clears_its_clip() {
    let mut db = writable_db_with_line();
    let mut attributes = db.entity_render_attributes(EntityId(1));
    attributes.clip = Some(sample_clip());
    let mut tx = db
        .begin_drawing_transaction("xclip", TransactionId(2))
        .unwrap();
    tx.update_entity_display(EntityId(1), attributes).unwrap();
    tx.commit().unwrap();

    let mut tx = db
        .begin_drawing_transaction("erase", TransactionId(3))
        .unwrap();
    tx.delete_entity(EntityId(1)).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        db.entity_render_attributes(EntityId(1)).clip,
        None,
        "delete cleanup drops the clip with the rest of the display state"
    );
}
