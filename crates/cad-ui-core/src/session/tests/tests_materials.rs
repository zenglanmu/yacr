//! The Materials tray's document-material read (`cad_materials` /
//! `kind: "materials"`).
//!
//! The projection is a plain copy of the host row; painting stays a command.
//! Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_materials_request_round_trips_and_has_its_own_no_lane_marker() {
    use crate::host_read::{no_seam_refusal, HostReadAnswer, HostReadRequest};
    assert_eq!(
        HostReadRequest::Materials.kind(),
        crate::material::MATERIALS_KIND
    );
    assert_eq!(
        serde_json::to_value(&HostReadRequest::Materials).unwrap(),
        serde_json::json!({ "kind": "materials" })
    );
    let decoded: HostReadRequest =
        serde_json::from_value(serde_json::json!({ "kind": "materials" })).expect("decode");
    assert_eq!(decoded, HostReadRequest::Materials);
    let answer = HostReadAnswer::Materials(crate::material::MaterialsResponse::default());
    assert!(matches!(answer, HostReadAnswer::Materials(_)));
    assert_eq!(
        no_seam_refusal(&HostReadRequest::Materials),
        crate::material::MATERIALS_UNSUPPORTED
    );

    let session = fresh_session();
    let list = session.materials();
    assert!(list.materials.is_empty(), "no document, no fabricated rows");
    assert_eq!(list.total, 0);
    assert!(!list.truncated);
}

#[test]
fn the_wire_row_copies_the_host_material_fields() {
    use crate::material::MaterialsResponse;
    let wire = MaterialsResponse::from_host(cad_host::MaterialList {
        rows: vec![cad_host::MaterialListRow {
            name: "Red".into(),
            color: [178, 34, 34],
            transparency: 0.25,
        }],
        total: 1,
        truncated: false,
    });
    assert_eq!(
        serde_json::to_value(&wire).unwrap(),
        serde_json::json!({
            "materials": [{ "name": "Red", "color": [178, 34, 34], "transparency": 0.25 }],
            "total": 1,
            "truncated": false
        })
    );
}
