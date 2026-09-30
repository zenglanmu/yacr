use cad_domain::*;

#[test]
fn stale_background_results_are_rejected_for_every_version_dimension() {
    let current = TaskStamp {
        document: DocumentId(1),
        generation: 2,
        object_revision: Revision(3),
        dependency_version: 4,
        configuration_version: 5,
    };
    assert_eq!(current.validate(&current), Ok(()));
    for dimension in 0..5 {
        let mut old = current.clone();
        match dimension {
            0 => old.document = DocumentId(99),
            1 => old.generation += 1,
            2 => old.object_revision = Revision(99),
            3 => old.dependency_version += 1,
            _ => old.configuration_version += 1,
        }
        assert_eq!(old.validate(&current), Err(CadError::StaleResult));
    }
}

#[test]
fn pending_is_a_structured_error_not_empty_success() {
    assert_eq!(
        pending::<()>("contract.example"),
        Err(CadError::NotImplemented("contract.example"))
    );
}
