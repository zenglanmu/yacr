//! The kind catalogue's key/index/label round-trip contract.
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! The shell mirrors [`DrawToolKind::ALL`] by index, so every kind must round-trip
//! through `key`/`from_key`, `label`/`from_label` and `index`/`from_index`.

use super::*;

#[test]
fn keys_indices_and_labels_round_trip_and_reject_unknowns() {
    for (index, kind) in DrawToolKind::ALL.iter().copied().enumerate() {
        assert_eq!(kind.index(), index);
        assert_eq!(DrawToolKind::from_index(index as i32), Some(kind));
        assert_eq!(DrawToolKind::from_key(kind.key()), Some(kind));
        assert_eq!(DrawToolKind::from_label(kind.label()), Some(kind));
    }
    assert_eq!(DrawToolKind::from_key("arc"), Some(DrawToolKind::Arc));
    assert_eq!(DrawToolKind::from_key("elbow"), None);
    assert_eq!(DrawToolKind::from_index(-1), None);
    assert_eq!(
        DrawToolKind::from_index(DrawToolKind::ALL.len() as i32),
        None
    );
    assert_eq!(DrawToolKind::from_label("半径"), None);
}
