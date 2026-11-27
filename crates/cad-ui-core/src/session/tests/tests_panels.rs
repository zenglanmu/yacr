//! Match Photo / Overlays host reads and their envelope shapes.
//!
//! Match Photo answers `modeled: false` with the stable reason; Overlays answers
//! the renderer-owned channel rows. Both have their own no-lane marker. Contract
//! level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_match_photo_request_round_trips_and_reports_unmodeled() {
    use crate::host_read::{HostReadAnswer, HostReadRequest};
    assert_eq!(
        HostReadRequest::MatchPhoto.kind(),
        crate::match_photo::MATCH_PHOTO_KIND
    );
    assert_eq!(
        serde_json::to_value(&HostReadRequest::MatchPhoto).unwrap(),
        serde_json::json!({ "kind": "matchPhoto" })
    );
    let decoded: HostReadRequest =
        serde_json::from_value(serde_json::json!({ "kind": "matchPhoto" })).expect("decode");
    assert_eq!(decoded, HostReadRequest::MatchPhoto);
    let answer = HostReadAnswer::MatchPhoto(crate::match_photo::MatchPhotoResponse::default());
    assert!(matches!(answer, HostReadAnswer::MatchPhoto(_)));

    let session = fresh_session();
    let status = session.match_photo();
    assert!(!status.modeled);
    assert!(status.reason.contains("matchphoto.not_implemented"));
    assert_eq!(
        crate::host_read::no_seam_refusal(&HostReadRequest::MatchPhoto),
        crate::match_photo::MATCH_PHOTO_UNSUPPORTED
    );
}

#[test]
fn the_overlays_request_round_trips_and_projects_the_channels() {
    use crate::host_read::{HostReadAnswer, HostReadRequest};
    assert_eq!(
        HostReadRequest::Overlays.kind(),
        crate::overlays::OVERLAYS_KIND
    );
    assert_eq!(
        serde_json::to_value(&HostReadRequest::Overlays).unwrap(),
        serde_json::json!({ "kind": "overlays" })
    );
    let decoded: HostReadRequest =
        serde_json::from_value(serde_json::json!({ "kind": "overlays" })).expect("decode");
    assert_eq!(decoded, HostReadRequest::Overlays);
    let answer = HostReadAnswer::Overlays(crate::overlays::OverlaysResponse::default());
    assert!(matches!(answer, HostReadAnswer::Overlays(_)));

    let session = fresh_session();
    let list = session.overlays();
    assert_eq!(list.rows.len(), cad_host::OVERLAY_ROW_KEYS.len());
    let keys: Vec<&str> = list.rows.iter().map(|row| row.key.as_str()).collect();
    assert_eq!(keys, cad_host::OVERLAY_ROW_KEYS.to_vec());
    assert_eq!(
        crate::host_read::no_seam_refusal(&HostReadRequest::Overlays),
        crate::overlays::OVERLAYS_UNSUPPORTED
    );
}
