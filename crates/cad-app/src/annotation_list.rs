//! Annotation management projection: list, visibility and selection (spec F09).
//!
//! F09 asks for a management channel over existing annotations: a read-only
//! list, a hide/show state that round-trips, a selection for edit/delete, and
//! undo/redo consistency. The list is a pure projection of the authoritative
//! [`cad_db::AnnotationDatabase`]; nothing here mutates an annotation or the
//! DWG.
//!
//! **Visibility is a session override, not a stored field.** The annotation
//! database has no `hidden` column and the sidecar format carries none
//! (`docs/annotations.md` §5), so inventing a persisted flag would be fake data.
//! Instead this module mirrors the F03 layer-override design: an id-keyed map
//! of temporary visibility that never raises the annotation revision, never
//! creates a history entry and never touches the sidecar.

use std::collections::BTreeMap;

use cad_db::{Annotation, AnnotationDatabase, AnnotationGeometry};
use cad_domain::AnnotationId;

/// A read-only projection of one annotation row for the management panel.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationRow {
    pub id: AnnotationId,
    /// Stable machine key of the geometry kind (`text`, `leader`, …).
    pub kind: &'static str,
    /// User-facing kind label.
    pub kind_label: &'static str,
    /// The annotation's text payload; empty when the kind carries none.
    pub text: String,
    /// Effective visibility the scene should honour (override ⊕ visible).
    pub visible: bool,
    /// Whether a temporary session override is active for this annotation.
    pub overridden: bool,
    /// Whether this annotation is the current management selection.
    pub selected: bool,
}

impl AnnotationRow {
    pub fn is_overridden(&self) -> bool {
        self.overridden
    }

    pub fn is_hidden(&self) -> bool {
        !self.visible
    }
}

/// Stable machine key and label of an annotation geometry kind.
///
/// Kept here (not translated) so the UI model and any lookup share one source of
/// truth, exactly like the measurement/annotation tool kinds.
pub fn geometry_kind(geometry: &AnnotationGeometry) -> (&'static str, &'static str) {
    match geometry {
        AnnotationGeometry::Text(_) => ("text", "文字"),
        AnnotationGeometry::Leader(_) => ("leader", "引线"),
        AnnotationGeometry::Rectangle(_) => ("rectangle", "矩形"),
        AnnotationGeometry::Ellipse { .. } => ("ellipse", "椭圆"),
        AnnotationGeometry::Freehand(_) => ("freehand", "自由线"),
        AnnotationGeometry::Cloud(_) => ("cloud", "云线"),
        AnnotationGeometry::Measurement(_) => ("measurement", "测量"),
    }
}

/// Session-scoped temporary annotation visibility, keyed by annotation id.
///
/// `None` for an id means "no override" and the annotation is shown; the
/// database has no stored hidden flag to defer to, so visible is the honest
/// default and a hide is always an explicit user action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnotationVisibilitySet {
    overrides: BTreeMap<AnnotationId, bool>,
}

impl AnnotationVisibilitySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, id: AnnotationId, visible: bool) {
        self.overrides.insert(id, visible);
    }

    pub fn remove(&mut self, id: AnnotationId) -> Option<bool> {
        self.overrides.remove(&id)
    }

    pub fn clear(&mut self) {
        self.overrides.clear();
    }

    pub fn get(&self, id: AnnotationId) -> Option<bool> {
        self.overrides.get(&id).copied()
    }

    /// Effective visibility: an override wins, otherwise the annotation shows.
    pub fn effective(&self, id: AnnotationId) -> bool {
        self.get(id).unwrap_or(true)
    }

    pub fn is_hidden(&self, id: AnnotationId) -> bool {
        !self.effective(id)
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (AnnotationId, bool)> + '_ {
        self.overrides.iter().map(|(id, v)| (*id, *v))
    }
}

/// Build the management-list projection for a document's annotations.
///
/// Order follows the database's own id order (a `BTreeMap`, deterministic). No
/// synthetic rows are invented; an empty database yields an empty list, which
/// the UI shows as an explicit empty state.
pub fn annotation_rows(
    database: &AnnotationDatabase,
    visibility: &AnnotationVisibilitySet,
    selected: Option<AnnotationId>,
) -> Vec<AnnotationRow> {
    database
        .annotations()
        .map(|annotation| annotation_row(annotation, visibility, selected))
        .collect()
}

fn annotation_row(
    annotation: &Annotation,
    visibility: &AnnotationVisibilitySet,
    selected: Option<AnnotationId>,
) -> AnnotationRow {
    let (kind, kind_label) = geometry_kind(&annotation.geometry);
    let override_visible = visibility.get(annotation.id);
    AnnotationRow {
        id: annotation.id,
        kind,
        kind_label,
        text: annotation.text.clone(),
        // No stored flag exists, so the database plays the role of a
        // permanently-visible layer and the override decides.
        visible: visibility.effective(annotation.id),
        overridden: override_visible.is_some(),
        selected: selected == Some(annotation.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{Annotation, AnnotationGeometry, AnnotationStyle};
    use cad_domain::{DatabaseId, Point3, Precision, SpaceId};

    fn ann(id: u128, geometry: AnnotationGeometry, text: &str) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry,
            text: text.into(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    fn database_with_two() -> AnnotationDatabase {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        db.apply_annotation_changes(
            "seed",
            cad_domain::TransactionId(1),
            vec![
                (
                    AnnotationId(1),
                    Some(ann(
                        1,
                        AnnotationGeometry::Text(Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        }),
                        "note",
                    )),
                ),
                (
                    AnnotationId(2),
                    Some(ann(
                        2,
                        AnnotationGeometry::Cloud(vec![
                            Point3 {
                                x: 0.0,
                                y: 0.0,
                                z: 0.0,
                            },
                            Point3 {
                                x: 1.0,
                                y: 0.0,
                                z: 0.0,
                            },
                            Point3 {
                                x: 1.0,
                                y: 1.0,
                                z: 0.0,
                            },
                        ]),
                        "",
                    )),
                ),
            ],
        )
        .unwrap();
        db
    }

    #[test]
    fn rows_come_from_the_database_and_report_kind_and_text() {
        let db = database_with_two();
        let rows = annotation_rows(&db, &AnnotationVisibilitySet::new(), None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, AnnotationId(1));
        assert_eq!(rows[0].kind, "text");
        assert_eq!(rows[0].kind_label, "文字");
        assert_eq!(rows[0].text, "note");
        assert!(rows[0].visible);
        assert!(!rows[0].is_overridden());
        assert_eq!(rows[1].kind, "cloud");
        assert_eq!(rows[1].text, "");
    }

    #[test]
    fn empty_database_yields_no_rows() {
        let db = AnnotationDatabase::new(DatabaseId(1));
        assert!(annotation_rows(&db, &AnnotationVisibilitySet::new(), None).is_empty());
    }

    #[test]
    fn visibility_hides_and_shows_without_touching_the_database() {
        let db = database_with_two();
        let before = db.revision();
        let mut visibility = AnnotationVisibilitySet::new();
        visibility.set(AnnotationId(1), false);

        let rows = annotation_rows(&db, &visibility, None);
        let first = rows.iter().find(|r| r.id == AnnotationId(1)).unwrap();
        assert!(!first.visible);
        assert!(first.is_overridden());
        // Showing it again round-trips.
        visibility.set(AnnotationId(1), true);
        let rows = annotation_rows(&db, &visibility, None);
        assert!(
            rows.iter()
                .find(|r| r.id == AnnotationId(1))
                .unwrap()
                .visible
        );
        // The database revision never moved.
        assert_eq!(db.revision(), before);
    }

    #[test]
    fn removing_an_override_restores_the_visible_default() {
        let mut visibility = AnnotationVisibilitySet::new();
        visibility.set(AnnotationId(3), false);
        assert!(visibility.is_hidden(AnnotationId(3)));
        assert_eq!(visibility.remove(AnnotationId(3)), Some(false));
        assert!(!visibility.is_hidden(AnnotationId(3)));
        assert!(visibility.is_empty());
    }

    #[test]
    fn selection_is_reflected_on_exactly_one_row() {
        let db = database_with_two();
        let rows = annotation_rows(&db, &AnnotationVisibilitySet::new(), Some(AnnotationId(2)));
        assert!(!rows[0].selected);
        assert!(rows[1].selected);
    }
}
