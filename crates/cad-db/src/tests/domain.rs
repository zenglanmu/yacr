//! The document's application domain (ADR-0053 D2/D3).
//!
//! Three rules: the named value is document state and survives a snapshot; an
//! absent value stays absent (older bytes); the derived default never claims a
//! profile the document has no data for.

use cad_domain::ApplicationDomain;

use crate::{DocumentSnapshot, DrawingDatabase, DrawingDatabaseBuilder};

fn snapshot_of(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        cad_domain::UnitContext::drawing_units(),
        cad_domain::DocumentIdentity::Temporary(1),
        Vec::new(),
        cad_domain::Revision(1),
    )
    .expect("snapshot")
}

#[test]
fn a_named_domain_is_document_state_and_survives_a_snapshot() {
    let mut builder = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    builder.set_application_domain(Some(ApplicationDomain::Bim));
    let db = builder.finish().expect("database");
    assert_eq!(db.application_domain(), Some(ApplicationDomain::Bim));

    let snapshot = snapshot_of(&db);
    assert_eq!(
        snapshot.header.application_domain,
        Some(ApplicationDomain::Bim)
    );
    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert_eq!(
        hydrated.application_domain(),
        Some(ApplicationDomain::Bim),
        "the domain is document state, not a carrier preference"
    );
}

#[test]
fn an_unnamed_document_derives_a_conservative_default() {
    // No BIM rows: drafting. Not "unknown", not a format claim.
    let plain = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(2))
        .finish()
        .expect("database");
    assert_eq!(plain.application_domain(), None);
    assert_eq!(
        plain.effective_application_domain(),
        ApplicationDomain::Cad2d
    );
}
