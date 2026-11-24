//! Import of `AcDbGroup` objects into the persisted group table.
//!
//! acadrust exposes a group as `ObjectType::Group`; its name lives in the owning
//! `ACAD_GROUP` dictionary (the DXF reader does not restore it onto the object,
//! the DWG reader does). This reader therefore resolves:
//!
//! 1. the name from the object when present, else from any dictionary entry that
//!    points at the group handle, else a deterministic `*G<handle>` fallback for
//!    an unnamed group that has no dictionary key;
//! 2. every member handle through the handle → entity-id map the entity import
//!    filled ([`ImporterBuilder::entity_by_handle`]).
//!
//! A member handle that was not imported (an entity over the limit, an
//! unsupported record, a block member the source dropped) is **named in a
//! diagnostic**; the group row is still written with its resolvable members, so
//! the group itself is never silently dropped.

use std::collections::{BTreeSet, HashMap};

use acadrust::objects::{Group as AcadGroup, ObjectType};
use acadrust::types::Handle;
use cad_db::{GroupRow, GroupSeq};
use cad_domain::{CadError, CadResult, Diagnostic, EntityId};

use crate::ImporterBuilder;

impl<'a> ImporterBuilder<'a> {
    /// Read every `OBJECTS`-section group into the builder's group table.
    ///
    /// Called after the entity walk (handles must already map to entity ids) and
    /// before [`crate::DrawingDatabaseBuilder::finish`].
    pub(crate) fn read_groups(&mut self) -> CadResult<()> {
        let groups: Vec<(Handle, &AcadGroup)> = self
            .acad
            .objects
            .iter()
            .filter_map(|(handle, object)| match object {
                ObjectType::Group(group) => Some((*handle, group)),
                _ => None,
            })
            .collect();
        if groups.is_empty() {
            return Ok(());
        }
        let group_handles: BTreeSet<Handle> = groups.iter().map(|(handle, _)| *handle).collect();
        let mut dictionary_names: HashMap<Handle, String> = HashMap::new();
        for object in self.acad.objects.values() {
            let ObjectType::Dictionary(dictionary) = object else {
                continue;
            };
            for (name, handle) in &dictionary.entries {
                if group_handles.contains(handle) {
                    dictionary_names
                        .entry(*handle)
                        .or_insert_with(|| name.clone());
                }
            }
        }

        let mut rows: Vec<GroupRow> = Vec::with_capacity(groups.len());
        let mut used_names: BTreeSet<String> = BTreeSet::new();
        for (index, (handle, group)) in groups.iter().enumerate() {
            let source_name = if !group.name.trim().is_empty() {
                group.name.clone()
            } else if let Some(name) = dictionary_names.get(handle) {
                name.clone()
            } else {
                format!("*G{:X}", handle.value())
            };
            let name = unique_name(&source_name, &mut used_names);
            if !name.eq_ignore_ascii_case(source_name.trim()) {
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: "import.group_renamed".into(),
                    message: format!(
                        "source group {source_name:?} collides case-insensitively with another \
                         imported group; it is stored as {name:?} so both groups survive"
                    ),
                });
            }
            let mut members: Vec<EntityId> = Vec::with_capacity(group.entities.len());
            let mut missing = 0usize;
            for entity_handle in &group.entities {
                match self.entity_by_handle.get(&entity_handle.value()) {
                    Some(id) => {
                        if !members.contains(id) {
                            members.push(*id);
                        }
                    }
                    None => missing += 1,
                }
            }
            if missing > 0 {
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: "import.group_member_missing".into(),
                    message: format!(
                        "group {name:?} names {missing} source handle(s) that were not imported; \
                         the group is kept with its {} resolvable member(s)",
                        members.len()
                    ),
                });
            }
            rows.push(GroupRow {
                seq: GroupSeq(index as u64 + 1),
                name,
                description: group.description.clone(),
                selectable: group.selectable,
                members,
            });
        }
        self.builder
            .set_groups(rows)
            .map_err(|error| CadError::InvalidInput(format!("import.group_invalid: {error}")))
    }
}

/// Make `name` unique case-insensitively, appending `#2`, `#3`, … on collision.
fn unique_name(name: &str, used: &mut BTreeSet<String>) -> String {
    let base = name.trim();
    if used.insert(base.to_ascii_lowercase()) {
        return base.to_string();
    }
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{base}#{suffix}");
        if used.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
        suffix += 1;
    }
}
