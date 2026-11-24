//! Persisted entity groups → `AcDbGroup` objects (OBJECTS section).
//!
//! Each row is written as one `GROUP` record owned by the `ACAD_GROUP`
//! dictionary, with the dictionary entry key carrying the group name (DXF does
//! not repeat the name inside the group body), the stored description, the
//! selectable flag and the **written** entity handles.
//!
//! A member whose entity was not written (paper-space export is deferred in v1,
//! or the geometry was refused) is named in the report as
//! `export.group_member_not_written`; the group itself is still written with its
//! resolvable members, so a group is never silently dropped. The group name and
//! description are plain strings and always round-trip.

use std::collections::BTreeMap;

use acadrust::objects::{Dictionary, Group, ObjectType};
use acadrust::types::Handle;
use acadrust::CadDocument;
use cad_db::DrawingDatabase;
use cad_domain::{EntityId, ObjectId};

use super::report::{ExportDiagnostic, ExportOutcome, ExportReport};

/// Write every group row into `doc` and append the explicit member losses.
pub(super) fn write_groups(
    doc: &mut CadDocument,
    database: &DrawingDatabase,
    handles: &BTreeMap<EntityId, Handle>,
    report: &mut ExportReport,
) {
    let rows: Vec<&cad_db::GroupRow> = database.groups().collect();
    if rows.is_empty() {
        return;
    }
    let dictionary = group_dictionary(doc);
    for row in rows {
        let handle = doc.allocate_handle();
        let mut group = Group::new(&row.name);
        group.handle = handle;
        group.owner = dictionary;
        group.description = row.description.clone();
        group.selectable = row.selectable;
        // An anonymous AutoCAD group is a `*A…`-named dictionary entry; the
        // unnamed flag is derived from the name because the persisted model does
        // not store a separate flag (the command layer never creates one).
        group.unnamed = row.name.starts_with('*');
        for member in &row.members {
            match handles.get(member) {
                Some(written) => group.add_entity(*written),
                None => {
                    report.counts.dropped += 1;
                    report.entries.push(ExportDiagnostic {
                        object: Some(ObjectId(member.0)),
                        code: "export.group_member_not_written".into(),
                        outcome: ExportOutcome::Dropped,
                        message: format!(
                            "group '{}' names entity {}, which was not written (paper-space \
                             export is deferred in v1, or the entity was refused); the group is \
                             written with its remaining members",
                            row.name, member.0
                        ),
                    });
                }
            }
        }
        doc.objects.insert(handle, ObjectType::Group(group));
        if let Some(ObjectType::Dictionary(dict)) = doc.objects.get_mut(&dictionary) {
            dict.add_entry(row.name.clone(), handle);
        }
    }
}

/// The `ACAD_GROUP` dictionary handle, created under the named-objects root when
/// the document has none (or when the header names a missing object).
fn group_dictionary(doc: &mut CadDocument) -> Handle {
    let existing = doc.header.acad_group_dict_handle;
    if !existing.is_null() && matches!(doc.objects.get(&existing), Some(ObjectType::Dictionary(_)))
    {
        return existing;
    }
    let handle = doc.allocate_handle();
    let root = doc.header.named_objects_dict_handle;
    let mut dict = Dictionary::new();
    dict.handle = handle;
    dict.owner = root;
    doc.objects.insert(handle, ObjectType::Dictionary(dict));
    if let Some(ObjectType::Dictionary(root_dict)) = doc.objects.get_mut(&root) {
        root_dict.add_entry("ACAD_GROUP", handle);
    }
    doc.header.acad_group_dict_handle = handle;
    handle
}
