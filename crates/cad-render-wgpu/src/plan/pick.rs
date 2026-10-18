use super::*;

use std::collections::HashMap;

/// The id-pass value for "no geometry": the cleared background.
pub(crate) const PICK_KEY_NONE: u32 = 0;

/// Upper bound on renderer pick keys within one device generation. The key space
/// grows with the batches appended since the last full clear; the bound keeps the
/// flags table (indexed by key) finite and is reported when exhausted rather than
/// silently wrapping.
pub(crate) const MAX_PICK_KEYS: u32 = 1 << 20;

/// Machine key: the batch carries no source at all, so the key cannot name an
/// entity.
pub(crate) const PICK_REASON_NO_SOURCE: &str = "pick-batch-has-no-source";

/// Machine key: the batch carries several sources whose geometry is
/// indistinguishable after representation, so the hit is ambiguous.
pub(crate) const PICK_REASON_MULTI_SOURCE: &str = "pick-multi-source-batch";

/// Machine key: a mesh-face sub-element is not derivable from the id pass (wgpu
/// exposes no primitive index to the fragment stage); CPU refinement is the
/// documented fallback.
pub(crate) const PICK_REASON_SUB_ELEMENT: &str = "pick-sub-element-requires-cpu-refinement";

/// The CPU-side resolution of one pick key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PickEntry {
    /// Index into the *combined* batch+image list this entry was built from:
    /// batches first (`0..batches.len()`), then images (`batches.len() + i`).
    /// Internal bookkeeping for the table; not part of the public hit surface.
    pub(crate) batch: u32,
    /// Every source that can own a fragment of this key, in the batch's own
    /// order.
    pub(crate) sources: Vec<SelectionRef>,
    /// `true` when one of this entry's sources shares its low-32 `EntityId` with
    /// another distinct identity elsewhere in the table.
    pub(crate) collision: bool,
}

/// A repeated renderer pick key in the resident set.
///
/// Keys are allocated monotonically within a device generation, so a duplicate
/// can only come from the allocator being restarted while keys are still
/// resident — a lifecycle error, never a legitimate state. The table cannot be
/// built unambiguously, so it is reported rather than resolved "first wins".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DuplicatePickKey {
    /// The key carried by two different owners.
    pub(crate) key: u32,
    /// Combined batch/image index of the first owner.
    pub(crate) first: u32,
    /// Combined batch/image index of the second owner.
    pub(crate) second: u32,
}

/// The resolution of one key, owned so a caller can hold it past the table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedPick {
    pub(crate) batch: u32,
    pub(crate) sources: Vec<SelectionRef>,
    /// The exact identity when the key names exactly one source.
    pub(crate) source: Option<SelectionRef>,
    /// Machine key explaining a non-exact resolution.
    pub(crate) reason: Option<&'static str>,
    pub(crate) collision: bool,
}

/// Whole-entity identity (`document + entity + instance`) → the pick keys whose
/// entry carries that identity as a **whole-entity** source. The lazy index the
/// entity-flag path builds (see `PickTable::ensure_source_index`).
type SourceIdentityIndex = HashMap<(DocumentId, EntityId, Vec<EntityId>), Vec<u32>>;

/// Key → (batch, full identities) resolution, plus the detected truncation
/// collisions.
///
/// Indexed by key, so `entries[0]` is the background slot and gaps (retired keys)
/// are `None`. Pure data: built from the resident batches and unit-tested without
/// a GPU.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PickTable {
    entries: Vec<Option<PickEntry>>,
    collisions: Vec<PickCollision>,
    /// Highest key present (0 when the table has no entries).
    max_key: u32,
    /// Whole-entity identity → pick keys, so an entity-flag update is
    /// `O(refs)` instead of `O(refs × entries × sources)`. Only sources that are
    /// themselves whole-entity (`sub_element.is_none()`) are indexed: a
    /// sub-element-carrying source is the sub-element overlay and must never be
    /// matched by an entity-level flag (a face highlight would otherwise be
    /// double-tinted, or discarded by `FLAG_HIDDEN`).
    ///
    /// `None` when the index has not been built: it is only ever read by
    /// [`Self::keys_for_source`], which is only called from the entity-flag path,
    /// so a scene that never issues an entity-flag request never pays for it
    /// (see [`Self::ensure_source_index`]). [`Self::replace_suffix`] keeps an
    /// already-built index exact and leaves an unbuilt one unbuilt.
    source_index: Option<SourceIdentityIndex>,
    /// Truncated `EntityId` (`entity.0 as u32`) → the ascending pick keys whose
    /// entry carries a source with that low-32 value. Retained so an incremental
    /// [`Self::replace_suffix`] keeps `EntityId`-truncation collision detection
    /// **exact** without rescanning every resident source: a fresh
    /// [`Self::build`] derives the identical index, so the two agree by
    /// construction (see [`Self::collision_identities_for`]).
    truncated_index: std::collections::BTreeMap<u32, Vec<u32>>,
}

impl PickTable {
    /// Build the table over `keys[i] -> sources[i]` pairs (the resident batches
    /// and images).
    ///
    /// `key` values must be `1..=MAX_PICK_KEYS`; a value outside that range is
    /// skipped (it can never be written to the id target). A repeated key is a
    /// programming/lifecycle error and is returned as [`DuplicatePickKey`] rather
    /// than silently resolved.
    ///
    /// The entries are **consumed** (their `sources` moved into the table) so a
    /// caller that already owns them pays no second clone; the renderer clones
    /// each resident batch's `sources` exactly once, into this argument.
    ///
    /// `build_source_index` is `false` when the caller knows the table will not
    /// be asked an entity-flag question before the next rebuild (see
    /// [`Self::ensure_source_index`]): the identity index is then left unbuilt.
    pub(crate) fn build(
        entries: Vec<(u32, Vec<SelectionRef>)>,
        build_source_index: bool,
    ) -> Result<PickTable, DuplicatePickKey> {
        let declared_max = entries.iter().map(|(key, _)| *key).max().unwrap_or(0);
        let mut slots: Vec<Option<PickEntry>> = vec![None; declared_max as usize + 1];
        for (batch, (key, sources)) in entries.into_iter().enumerate() {
            if key == PICK_KEY_NONE || key > MAX_PICK_KEYS {
                continue;
            }
            let slot = &mut slots[key as usize];
            if let Some(existing) = slot.as_ref() {
                return Err(DuplicatePickKey {
                    key,
                    first: existing.batch,
                    second: batch as u32,
                });
            }
            *slot = Some(PickEntry {
                batch: batch as u32,
                sources,
                collision: false,
            });
        }
        // `max_key` is the highest key **present** (the doc contract), so the
        // slot vector is trimmed of trailing gaps: a fresh build and an
        // incremental update then size the table identically even when the
        // largest keys were retired without replacement.
        while slots.last().is_some_and(Option::is_none) {
            slots.pop();
        }
        let max_key = slots.len().saturating_sub(1) as u32;
        let mut table = PickTable {
            entries: slots,
            collisions: Vec::new(),
            max_key,
            source_index: None,
            truncated_index: std::collections::BTreeMap::new(),
        };
        table.rebuild_truncated_index();
        table.rebuild_collisions();
        if build_source_index {
            table.ensure_source_index();
        }
        Ok(table)
    }

    /// Replace the resident suffix incrementally: retire `retired_keys` (the pick
    /// keys of the dropped batches) and append `added` entries whose combined
    /// batch/image indices start at `base`.
    ///
    /// Valid **only** when the kept prefix is a true prefix of the resident batch
    /// list (the caller guarantees `keep_prefix <= the resident batch count`, and
    /// keys are never renumbered or reused within a device generation), so every
    /// entry outside `retired_keys` is unchanged and the new keys are strictly
    /// greater than every existing key. `max_key` is recomputed as the highest
    /// key **present**, exactly as a fresh [`Self::build`] computes it (it can
    /// shrink when the largest keys are retired without replacement). The result
    /// is **identical** to a fresh [`Self::build`] over the resident set: the
    /// entry mapping, the `keys_for_source` answers, the truncation-collision set
    /// and `max_key` all agree (fenced by the differential test).
    ///
    /// A repeated key (a new key that is already resident) is a lifecycle error
    /// and is returned as [`DuplicatePickKey`], exactly as in [`Self::build`].
    pub(crate) fn replace_suffix(
        &mut self,
        base: u32,
        retired_keys: &[u32],
        added: Vec<(u32, Vec<SelectionRef>)>,
    ) -> Result<(), DuplicatePickKey> {
        // Truncated ids touched by this update; only these can change a collision.
        let mut affected: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        // Validate the added keys **before** mutating, so a duplicate leaves the
        // table untouched (a fresh build refuses the same way).
        let retired: std::collections::HashSet<u32> = retired_keys.iter().copied().collect();
        let mut added_keys: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (index, (key, _)) in added.iter().enumerate() {
            if *key == PICK_KEY_NONE || *key > MAX_PICK_KEYS {
                continue;
            }
            if !added_keys.insert(*key) {
                return Err(DuplicatePickKey {
                    key: *key,
                    first: base + index as u32,
                    second: base + index as u32,
                });
            }
            if !retired.contains(key) {
                if let Some(Some(existing)) = self.entries.get(*key as usize) {
                    return Err(DuplicatePickKey {
                        key: *key,
                        first: existing.batch,
                        second: base + index as u32,
                    });
                }
            }
        }
        // 1. Retire the dropped entries.
        for &key in retired_keys {
            if key == PICK_KEY_NONE || key > MAX_PICK_KEYS {
                continue;
            }
            let Some(slot) = self.entries.get_mut(key as usize) else {
                continue;
            };
            let Some(entry) = slot.take() else {
                continue;
            };
            for source in &entry.sources {
                let truncated = source.entity.0 as u32;
                affected.insert(truncated);
                if let Some(keys) = self.truncated_index.get_mut(&truncated) {
                    keys.retain(|existing| *existing != key);
                    if keys.is_empty() {
                        self.truncated_index.remove(&truncated);
                    }
                }
                if let Some(index) = self.source_index.as_mut() {
                    if source.sub_element.is_none() {
                        let identity = (source.document, source.entity, source.instance.0.clone());
                        if let Some(keys) = index.get_mut(&identity) {
                            keys.retain(|existing| *existing != key);
                            if keys.is_empty() {
                                index.remove(&identity);
                            }
                        }
                    }
                }
            }
        }
        // 2. Append the new entries. `base` is the combined index of the first
        //    new entry, so `base + i` matches `build`'s enumerate position.
        let declared_max = added.iter().map(|(key, _)| *key).max().unwrap_or(0);
        if declared_max as usize >= self.entries.len() {
            self.entries.resize(declared_max as usize + 1, None);
        }
        for (index, (key, sources)) in added.into_iter().enumerate() {
            if key == PICK_KEY_NONE || key > MAX_PICK_KEYS {
                continue;
            }
            for source in &sources {
                let truncated = source.entity.0 as u32;
                affected.insert(truncated);
                let keys = self.truncated_index.entry(truncated).or_default();
                if keys.last() != Some(&key) {
                    keys.push(key);
                }
            }
            if let Some(index_map) = self.source_index.as_mut() {
                for source in &sources {
                    if source.sub_element.is_some() {
                        continue;
                    }
                    let keys = index_map
                        .entry((source.document, source.entity, source.instance.0.clone()))
                        .or_default();
                    if keys.last() != Some(&key) {
                        keys.push(key);
                    }
                }
            }
            self.entries[key as usize] = Some(PickEntry {
                batch: base + index as u32,
                sources,
                collision: false,
            });
        }
        // Trim trailing gaps and take `max_key` as the highest key **present**, so
        // the table is sized exactly as a fresh build would size it even when the
        // retired suffix carried the largest keys (see `Self::build`).
        while self.entries.last().is_some_and(Option::is_none) {
            self.entries.pop();
        }
        self.max_key = self.entries.len().saturating_sub(1) as u32;
        // 3. Recompute collisions exactly, only for the touched truncated ids.
        self.update_collisions(&affected);
        Ok(())
    }

    /// Build the whole-entity identity index if it is not built yet.
    ///
    /// Called by the entity-flag path (and only there): `keys_for_source` reads
    /// it, and a scene that never issues an entity-flag request never needs it.
    pub(crate) fn ensure_source_index(&mut self) {
        if self.source_index.is_some() {
            return;
        }
        let mut index: SourceIdentityIndex = HashMap::new();
        for (key, entry) in self.entries.iter().enumerate() {
            let Some(entry) = entry else { continue };
            for source in &entry.sources {
                if source.sub_element.is_some() {
                    continue;
                }
                let keys = index
                    .entry((source.document, source.entity, source.instance.0.clone()))
                    .or_default();
                let key = key as u32;
                if keys.last() != Some(&key) {
                    keys.push(key);
                }
            }
        }
        self.source_index = Some(index);
    }

    /// The combined batch/image index of the entry owning `key`, or `None` when
    /// `key` is not a live entry. O(1); used by the renderer to prove its table
    /// still covers the kept batch prefix before an incremental update.
    pub(crate) fn batch_index_of_key(&self, key: u32) -> Option<u32> {
        self.entries
            .get(key as usize)?
            .as_ref()
            .map(|entry| entry.batch)
    }

    /// Rebuild [`Self::truncated_index`] from `entries` (fresh-build path only).
    pub(super) fn rebuild_truncated_index(&mut self) {
        self.truncated_index.clear();
        for (key, entry) in self.entries.iter().enumerate() {
            let Some(entry) = entry else { continue };
            for source in &entry.sources {
                let keys = self
                    .truncated_index
                    .entry(source.entity.0 as u32)
                    .or_default();
                let key = key as u32;
                if keys.last() != Some(&key) {
                    keys.push(key);
                }
            }
        }
    }

    /// The distinct full identities that share the low-32 `truncated` value, in
    /// key order then source order — the exact group
    /// `detect_truncation_collisions` reported before the index existed.
    pub(super) fn distinct_identities_for(&self, truncated: u32) -> Vec<SelectionRef> {
        let Some(keys) = self.truncated_index.get(&truncated) else {
            return Vec::new();
        };
        let mut identities: Vec<SelectionRef> = Vec::new();
        for &key in keys {
            let Some(Some(entry)) = self.entries.get(key as usize) else {
                continue;
            };
            for source in &entry.sources {
                if source.entity.0 as u32 != truncated {
                    continue;
                }
                if !identities.contains(source) {
                    identities.push(source.clone());
                }
            }
        }
        identities
    }

    /// Recompute [`Self::collisions`] and every entry's `collision` flag from the
    /// current `truncated_index` (fresh-build path only).
    ///
    /// A single pass over the truncated ids: `collisions` stays sorted by
    /// `truncated_key` because the index is a `BTreeMap` (ascending iteration).
    /// Non-colliding ids (the common case) are rejected by
    /// [`Self::collision_identities_for`] without allocating.
    pub(super) fn rebuild_collisions(&mut self) {
        let mut collisions: Vec<PickCollision> = Vec::new();
        let mut colliding: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for &truncated in self.truncated_index.keys() {
            if let Some(identities) = self.collision_identities_for(truncated) {
                colliding.insert(truncated);
                collisions.push(PickCollision {
                    truncated_key: truncated,
                    identities,
                });
            }
        }
        self.collisions = collisions;
        if !colliding.is_empty() {
            for entry in self.entries.iter_mut().flatten() {
                entry.collision = entry
                    .sources
                    .iter()
                    .any(|source| colliding.contains(&(source.entity.0 as u32)));
            }
        }
    }

    /// Recompute the collision set and the entry flags, touching only the
    /// truncated ids in `affected`.
    ///
    /// A truncated id is colliding iff two or more **distinct** full identities
    /// carry it. `affected` must contain every truncated id whose identity set
    /// changed (the added and retired sources); every other id's colliding status
    /// is unchanged, so its collision entry and its entries' flags are left
    /// untouched — the whole point of the incremental update (a scene with many
    /// pre-existing collisions must not re-derive them on every commit).
    pub(super) fn update_collisions(&mut self, affected: &std::collections::BTreeSet<u32>) {
        if affected.is_empty() {
            return;
        }
        // Re-derive only the affected ids' collision entries.
        let mut inserts: Vec<PickCollision> = Vec::new();
        for &truncated in affected {
            if let Some(identities) = self.collision_identities_for(truncated) {
                inserts.push(PickCollision {
                    truncated_key: truncated,
                    identities,
                });
            }
        }
        // `collisions` is kept sorted by `truncated_key`; drop the affected ids'
        // stale entries and merge the fresh ones back in order.
        self.collisions
            .retain(|collision| !affected.contains(&collision.truncated_key));
        inserts.sort_by_key(|collision| collision.truncated_key);
        for collision in inserts {
            let position = self
                .collisions
                .partition_point(|existing| existing.truncated_key < collision.truncated_key);
            self.collisions.insert(position, collision);
        }
        // Only the affected ids' entries can have flipped their flag.
        let colliding: std::collections::BTreeSet<u32> = self
            .collisions
            .iter()
            .map(|collision| collision.truncated_key)
            .collect();
        for &truncated in affected {
            let Some(keys) = self.truncated_index.get(&truncated).cloned() else {
                continue;
            };
            for key in keys {
                let Some(Some(entry)) = self.entries.get_mut(key as usize) else {
                    continue;
                };
                entry.collision = entry
                    .sources
                    .iter()
                    .any(|source| colliding.contains(&(source.entity.0 as u32)));
            }
        }
    }

    /// The distinct identities carrying `truncated` **iff** it is a truncation
    /// collision (two or more distinct full identities), else `None`.
    ///
    /// The cheap path (every source equal to the first) allocates nothing, so a
    /// fresh build does not pay a `Vec` per non-colliding `EntityId`.
    pub(super) fn collision_identities_for(&self, truncated: u32) -> Option<Vec<SelectionRef>> {
        let keys = self.truncated_index.get(&truncated)?;
        let mut first: Option<&SelectionRef> = None;
        for &key in keys {
            let Some(Some(entry)) = self.entries.get(key as usize) else {
                continue;
            };
            for source in &entry.sources {
                if source.entity.0 as u32 != truncated {
                    continue;
                }
                match first {
                    None => first = Some(source),
                    Some(existing) if existing != source => {
                        return Some(self.distinct_identities_for(truncated));
                    }
                    Some(_) => {}
                }
            }
        }
        None
    }

    /// Resolve a key read back from the id target. `None` for the background
    /// (`0`) and for a key the current table does not know.
    pub(crate) fn resolve(&self, key: u32) -> Option<ResolvedPick> {
        let entry = self.entries.get(key as usize)?.as_ref()?;
        let source = (entry.sources.len() == 1).then(|| entry.sources[0].clone());
        let reason = if entry.sources.is_empty() {
            Some(PICK_REASON_NO_SOURCE)
        } else if entry.sources.len() > 1 {
            Some(PICK_REASON_MULTI_SOURCE)
        } else {
            None
        };
        Some(ResolvedPick {
            batch: entry.batch,
            sources: entry.sources.clone(),
            source,
            reason,
            collision: entry.collision,
        })
    }

    /// The detected `EntityId` truncation collisions, in key order.
    pub(crate) fn collisions(&self) -> &[PickCollision] {
        &self.collisions
    }

    /// Every pick key whose entry names `source` as one of its **whole-entity**
    /// sources, in key order.
    ///
    /// Identity is `document + entity + instance`; the sub-element is ignored on
    /// the **query** side because the flags path is whole-entity (a mesh face is
    /// a separate overlay). A batch source that itself carries a sub-element is
    /// the sub-element overlay and is **excluded**: an entity-level flag must not
    /// tint (or hide) the face overlay batch. A key is returned once even when
    /// several of its sources match: the caller sets the whole batch's bits,
    /// which is the documented batch-granularity limit for a multi-source run.
    ///
    /// The caller must have ensured the index exists
    /// ([`Self::ensure_source_index`]); the entity-flag path always does. An
    /// unbuilt index answers "no key", which is why the flags path builds it
    /// before querying rather than relying on a silent empty result.
    pub(crate) fn keys_for_source(&self, source: &SelectionRef) -> Vec<u32> {
        let identity = (source.document, source.entity, source.instance.0.clone());
        self.source_index
            .as_ref()
            .and_then(|index| index.get(&identity).cloned())
            .unwrap_or_default()
    }

    /// Highest key present (0 when empty).
    pub(crate) fn max_key(&self) -> u32 {
        self.max_key
    }

    /// Number of live entries.
    pub(crate) fn len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }
}
