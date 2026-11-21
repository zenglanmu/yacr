//! `SETENTITYPROPERTY` over a dynamic BIM PSet slot
//! (`bim.pset.{set}.{property}`).
//!
//! These pin the write half of ADR-0042 D4: the same property command and the
//! same one-transaction / one-undo-step granularity as the display-property
//! path, but the value is resolved against each selected element's own stored
//! variant and written through the BIM side table. Every refusal is
//! all-or-nothing, a mixed `(pset, property)` slot is a conflict, and an
//! unchanged value creates no undo entry.

use super::*;
use crate::selection_bim::BimPropertyKey;

use cad_db::{
    BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource, BimValue, BIM_MAX_STRING_BYTES,
};

fn source(id: &str) -> BimSource {
    BimSource {
        connector: "ifc".into(),
        document_id: Some("doc-1".into()),
        id: Some(id.into()),
        native_id: None,
    }
}

fn pset(name: &str, properties: Vec<(&str, BimValue)>) -> BimPropertySet {
    BimPropertySet {
        name: name.into(),
        properties: properties
            .into_iter()
            .map(|(name, value)| BimProperty {
                name: name.into(),
                value,
            })
            .collect(),
    }
}

/// One element carrying all four scalar variants, in one set:
/// `0.0` bool, `0.1` real (unit `m`), `0.2` integer, `0.3` text.
fn all_variants(source_id: &str) -> BimElement {
    BimElement {
        category: Some("IfcWall".into()),
        name: Some("Wall".into()),
        source: source(source_id),
        level: None,
        element_type: None,
        material: None,
        psets: vec![pset(
            "Pset_Test",
            vec![
                ("IsExternal", BimValue::Bool(true)),
                (
                    "Reference",
                    BimValue::Real {
                        value: 3.5,
                        ifc_type: Some("IfcPositiveLengthMeasure".into()),
                        unit: Some("m".into()),
                    },
                ),
                ("Count", BimValue::Integer(7)),
                ("Note", BimValue::Text("original".into())),
            ],
        )],
        evidence: BimEvidence::Declared,
    }
}

/// One element with a single property at `0.0`, named by the caller so a
/// multi-selection can either agree or conflict.
fn one_property(
    source_id: &str,
    set: &str,
    property: &str,
    value: BimValue,
    evidence: BimEvidence,
) -> BimElement {
    BimElement {
        category: Some("IfcWall".into()),
        name: Some("Wall".into()),
        source: source(source_id),
        level: None,
        element_type: None,
        material: None,
        psets: vec![pset(set, vec![(property, value)])],
        evidence,
    }
}

/// A drawing where each `(id, element)` pair is one model-space line carrying
/// exactly that BIM block.
fn bim_app(elements: Vec<(u128, BimElement)>) -> (Application, SessionState) {
    bim_app_in(elements, AppMode::Work)
}

fn bim_app_in(elements: Vec<(u128, BimElement)>, mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (index, (id, element)) in elements.into_iter().enumerate() {
        builder
            .insert_entity(ent(
                id,
                SemanticGeometry::Line {
                    start: point(0.0, index as f64),
                    end: point(1.0, index as f64),
                },
                index as i64,
            ))
            .unwrap();
        builder
            .set_entity_bim(EntityId(id), element)
            .expect("the entity was inserted first");
    }
    let drawing = builder.finish().expect("a valid BIM document");
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn set_property(
    app: &mut Application,
    session: &mut SessionState,
    key: &str,
    value: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: key.into(),
            value: value.into(),
        },
    )
}

fn stored_drawing(app: &Application) -> Arc<DrawingDatabase> {
    app.workspace.documents[&DocumentId(1)].drawing.clone()
}

fn stored_value(app: &Application, entity: u128, key: &str) -> BimValue {
    let slot = BimPropertyKey::parse(key).expect("a canonical slot key");
    stored_drawing(app)
        .bim_element(EntityId(entity))
        .expect("the element has BIM data")
        .psets[slot.set]
        .properties[slot.property]
        .value
        .clone()
}

fn stored_evidence(app: &Application, entity: u128) -> BimEvidence {
    stored_drawing(app)
        .bim_element(EntityId(entity))
        .expect("the element has BIM data")
        .evidence
}

fn undo(app: &mut Application, session: &mut SessionState) -> CommandOutcome {
    execute(app, session, CommandId::Undo, CommandPayload::None).expect("undo")
}

fn redo(app: &mut Application, session: &mut SessionState) -> CommandOutcome {
    execute(app, session, CommandId::Redo, CommandPayload::None).expect("redo")
}

#[test]
fn set_bim_key_writes_the_stored_variant_and_advances_the_revision() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);

    // Bool, real, integer and text each resolve against their own stored
    // variant; each write is one revision and one dirty element shard.
    for (key, draft, expected) in [
        ("bim.pset.0.0", "false", BimValue::Bool(false)),
        (
            "bim.pset.0.1",
            "4.5",
            BimValue::Real {
                value: 4.5,
                ifc_type: Some("IfcPositiveLengthMeasure".into()),
                unit: Some("m".into()),
            },
        ),
        ("bim.pset.0.2", "8", BimValue::Integer(8)),
        ("bim.pset.0.3", "edited", BimValue::Text("edited".into())),
    ] {
        let before = revision(&app);
        let outcome = set_property(&mut app, &mut session, key, draft).expect(key);

        assert_eq!(
            revision(&app),
            Revision(before.0 + 1),
            "{key} is one revision"
        );
        let changes = outcome.changes.expect("the edit publishes a ChangeSet");
        assert!(
            !changes.scope.shards.is_empty(),
            "{key} must dirty the element shard"
        );
        assert_eq!(stored_value(&app, 1, key), expected, "{key}");
    }
}

#[test]
fn set_bim_key_refuses_an_out_of_range_index() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    for key in ["bim.pset.9.0", "bim.pset.0.99"] {
        let error = set_property(&mut app, &mut session, key, "1").unwrap_err();
        assert!(
            format!("{error:?}").contains("property.target_missing"),
            "{key} must be target_missing, got {error:?}"
        );
    }
    assert_eq!(revision(&app), before, "refusals write nothing");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
}

#[test]
fn set_bim_key_refuses_a_malformed_key() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    // None of these is a canonical slot, so they fall through to the display
    // grammar's unsupported-key refusal instead of addressing a slot.
    for key in [
        "bim.pset.x",
        "bim.pset.0",
        "bim.pset.0.0.1",
        "bim.pset.00.0",
    ] {
        let error = set_property(&mut app, &mut session, key, "1").unwrap_err();
        assert!(
            format!("{error:?}").contains("property.unsupported_key"),
            "{key} must be unsupported_key, got {error:?}"
        );
    }
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn set_bim_key_refuses_a_type_mismatch() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    for (key, draft) in [
        ("bim.pset.0.2", "abc"),
        ("bim.pset.0.2", "5.0"),
        ("bim.pset.0.0", "1"),
        ("bim.pset.0.1", "NaN"),
        ("bim.pset.0.1", "inf"),
    ] {
        let error = set_property(&mut app, &mut session, key, draft).unwrap_err();
        assert!(
            format!("{error:?}").contains("property.invalid_value"),
            "{key}={draft} must be invalid_value, got {error:?}"
        );
    }

    // The text budget is its own named refusal: never a truncation.
    let too_long = "x".repeat(BIM_MAX_STRING_BYTES + 1);
    let error = set_property(&mut app, &mut session, "bim.pset.0.3", &too_long).unwrap_err();
    assert!(
        format!("{error:?}").contains("property.bim_too_long"),
        "got {error:?}"
    );

    assert_eq!(revision(&app), before, "refusals write nothing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn set_bim_key_keeps_the_real_unit_and_ifc_type() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);

    // A bare number keeps the stored measure; repeating the exact unit token is
    // accepted too.
    for draft in ["4", "4 m"] {
        set_property(&mut app, &mut session, "bim.pset.0.1", draft).expect(draft);
        assert_eq!(
            stored_value(&app, 1, "bim.pset.0.1"),
            BimValue::Real {
                value: 4.0,
                ifc_type: Some("IfcPositiveLengthMeasure".into()),
                unit: Some("m".into()),
            },
            "{draft} must keep the stored measure"
        );
    }

    // A different unit is refused: the measure is not converted or relabelled.
    let error = set_property(&mut app, &mut session, "bim.pset.0.1", "4 mm").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.invalid_value"),
        "got {error:?}"
    );
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.1"),
        BimValue::Real {
            value: 4.0,
            ifc_type: Some("IfcPositiveLengthMeasure".into()),
            unit: Some("m".into()),
        }
    );
}

#[test]
fn set_bim_key_on_a_multi_selection_writes_every_element_in_its_own_variant() {
    let (mut app, mut session) = bim_app(vec![
        (
            1,
            one_property(
                "wall-1",
                "Pset_Shared",
                "Value",
                BimValue::Real {
                    value: 3.5,
                    ifc_type: Some("IfcPositiveLengthMeasure".into()),
                    unit: Some("m".into()),
                },
                BimEvidence::Declared,
            ),
        ),
        (
            2,
            one_property(
                "wall-2",
                "Pset_Shared",
                "Value",
                BimValue::Integer(3),
                BimEvidence::Declared,
            ),
        ),
    ]);
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    // One draft, two stored variants: the same transaction writes both.
    let outcome = set_property(&mut app, &mut session, "bim.pset.0.0", "5").expect("multi write");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.0"),
        BimValue::Real {
            value: 5.0,
            ifc_type: Some("IfcPositiveLengthMeasure".into()),
            unit: Some("m".into()),
        }
    );
    assert_eq!(stored_value(&app, 2, "bim.pset.0.0"), BimValue::Integer(5));

    // One step restores both elements.
    undo(&mut app, &mut session);
    assert_eq!(revision(&app), Revision(before.0 + 2));
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.0"),
        BimValue::Real {
            value: 3.5,
            ifc_type: Some("IfcPositiveLengthMeasure".into()),
            unit: Some("m".into()),
        }
    );
    assert_eq!(stored_value(&app, 2, "bim.pset.0.0"), BimValue::Integer(3));
    assert!(!app.can_undo(&DocumentId(1)), "exactly one undo step");
}

#[test]
fn set_bim_key_refuses_a_conflicting_multi_selection() {
    let (mut app, mut session) = bim_app(vec![
        (
            1,
            one_property(
                "wall-1",
                "Pset_A",
                "Size",
                BimValue::Real {
                    value: 1.0,
                    ifc_type: None,
                    unit: Some("m".into()),
                },
                BimEvidence::Declared,
            ),
        ),
        (
            2,
            one_property(
                "wall-2",
                "Pset_B",
                "Width",
                BimValue::Real {
                    value: 2.0,
                    ifc_type: None,
                    unit: Some("m".into()),
                },
                BimEvidence::Declared,
            ),
        ),
    ]);
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let error = set_property(&mut app, &mut session, "bim.pset.0.0", "3").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.bim_conflict"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before, "no partial write");
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.0"),
        BimValue::Real {
            value: 1.0,
            ifc_type: None,
            unit: Some("m".into()),
        }
    );
    assert_eq!(
        stored_value(&app, 2, "bim.pset.0.0"),
        BimValue::Real {
            value: 2.0,
            ifc_type: None,
            unit: Some("m".into()),
        }
    );
    assert!(!app.can_undo(&DocumentId(1)));

    // The read side agrees: the same key is a mixed, non-editable row.
    let drawing = stored_drawing(&app);
    let properties = SelectionProperties::extract(&drawing, &session.selection);
    let row = properties
        .rows
        .iter()
        .find(|row| row.key == "bim.pset.0.0")
        .expect("the shared slot is listed");
    assert!(!row.editable, "a conflicting slot cannot be edited");
}

#[test]
fn set_bim_key_refuses_a_non_bim_selection() {
    // Entity 1 carries a BIM block, entity 2 never had one.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(ent(
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
            0,
        ))
        .unwrap();
    builder
        .insert_entity(ent(
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
            1,
        ))
        .unwrap();
    builder
        .set_entity_bim(EntityId(1), all_variants("wall-1"))
        .unwrap();
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = revision(&app);

    // A selection containing one BIM and one plain element refuses as a whole.
    session.selection.replace([ref_of(1), ref_of(2)]);
    let error = set_property(&mut app, &mut session, "bim.pset.0.0", "false").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.unsupported_key"),
        "got {error:?}"
    );
    assert_eq!(
        revision(&app),
        before,
        "the BIM element is not partially written"
    );
    assert_eq!(stored_value(&app, 1, "bim.pset.0.0"), BimValue::Bool(true));
    assert!(!app.can_undo(&DocumentId(1)));

    // The plain element alone is the same named refusal.
    session.selection.replace([ref_of(2)]);
    let error = set_property(&mut app, &mut session, "bim.pset.0.0", "false").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.unsupported_key"),
        "got {error:?}"
    );
}

#[test]
fn set_bim_key_reports_unchanged_without_an_undo_entry() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    let outcome = set_property(&mut app, &mut session, "bim.pset.0.3", "original").expect("no-op");
    assert!(outcome.changes.is_none(), "a no-op publishes no ChangeSet");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "property.unchanged"));
    assert_eq!(revision(&app), before, "no revision bump");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
}

#[test]
fn undo_and_redo_restore_the_bim_value() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    session.selection.replace([ref_of(1)]);
    set_property(&mut app, &mut session, "bim.pset.0.3", "edited").expect("write");
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.3"),
        BimValue::Text("edited".into())
    );
    let edited = revision(&app);

    let undo_outcome = undo(&mut app, &mut session);
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.3"),
        BimValue::Text("original".into()),
        "undo restores the stored value"
    );
    let undo_changes = undo_outcome.changes.expect("undo publishes a ChangeSet");
    assert_eq!(undo_changes.after, Revision(edited.0 + 1), "one revision");
    assert!(
        !undo_changes.scope.shards.is_empty(),
        "undo still dirties the element shard"
    );

    let redo_outcome = redo(&mut app, &mut session);
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.3"),
        BimValue::Text("edited".into()),
        "redo restores the edit"
    );
    let redo_changes = redo_outcome.changes.expect("redo publishes a ChangeSet");
    assert_eq!(redo_changes.after, Revision(edited.0 + 2), "one revision");
    assert!(!redo_changes.scope.shards.is_empty());
}

#[test]
fn set_bim_key_is_refused_in_viewer_mode() {
    let (mut app, mut session) = bim_app_in(vec![(1, all_variants("wall-1"))], AppMode::Viewer);
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    let error = set_property(&mut app, &mut session, "bim.pset.0.3", "edited").unwrap_err();
    assert!(
        matches!(error, CadError::PermissionDenied),
        "a viewer session cannot write, got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.3"),
        BimValue::Text("original".into())
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn set_bim_key_on_a_heuristic_element_is_allowed_and_keeps_the_evidence() {
    let (mut app, mut session) = bim_app(vec![(
        1,
        one_property(
            "wall-1",
            "Pset_Heuristic",
            "Note",
            BimValue::Text("guess".into()),
            BimEvidence::Heuristic,
        ),
    )]);
    session.selection.replace([ref_of(1)]);

    set_property(&mut app, &mut session, "bim.pset.0.0", "corrected").expect("heuristic edit");
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.0"),
        BimValue::Text("corrected".into())
    );
    assert_eq!(
        stored_evidence(&app, 1),
        BimEvidence::Heuristic,
        "editing a value must not promote or erase its provenance"
    );
}

#[test]
fn set_bim_key_refuses_a_block_instance_member_or_sub_element() {
    let (mut app, mut session) = bim_app(vec![(1, all_variants("wall-1"))]);
    let before = revision(&app);

    let instance_member = SelectionRef {
        instance: InstancePath(vec![EntityId(3)]),
        ..ref_of(1)
    };
    let sub_element = SelectionRef {
        sub_element: Some(cad_domain::SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    };
    for bad in [instance_member, sub_element] {
        session.selection.replace([bad]);
        let error = set_property(&mut app, &mut session, "bim.pset.0.3", "edited").unwrap_err();
        assert!(
            format!("{error:?}").contains("property.not_addressable"),
            "got {error:?}"
        );
    }
    assert_eq!(revision(&app), before);
    assert_eq!(
        stored_value(&app, 1, "bim.pset.0.3"),
        BimValue::Text("original".into())
    );
    assert!(!app.can_undo(&DocumentId(1)));
}
