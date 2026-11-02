//! External-reference (XREF) resolution and merge-at-import.
//!
//! Spec v2.0 §7.3, §16.2 and ADR 0025: an xref is *read-only content merged into
//! the host database at import time*. Nothing here keeps a live link or
//! re-reads a file later; the host database is the sole authority after import.
//!
//! The module owns three things:
//!
//! 1. The host-side loading policy ([`XrefOptions`]) and its private session
//!    state (active path stack + remaining byte budget). No filesystem access
//!    happens here: [`cad_resources::XrefFileSource`] is implemented by the
//!    carrier.
//! 2. Resolution of every top-level xref block discovered by the importer,
//!    including nested references inside the referenced drawings. Cycle
//!    detection compares the **lexically normalized** path from
//!    [`cad_resources::check_xref_path`]: canonicalisation would need host I/O
//!    and is deliberately left to the carrier.
//! 3. Splicing a referenced [`DrawingDatabase`] into the host builder with id
//!    remapping. Referenced layers and blocks are namespaced, model-space
//!    entities become members of the host xref block, and paper-space content is
//!    deliberately not merged (an xref is model content).
//!
//! # Namespacing (viewer naming, not round-trip)
//!
//! Layers use the host xref block name as a prefix joined with `|`, composing
//! through nesting: `outer|inner|Layer`. Blocks use the same chain with
//! `prefix$0$Name`, so an anonymous `*D0` in a referenced drawing becomes
//! `XREF$0$*D0` and cannot collide with the host's own `*D0`. These names are a
//! stable *viewer* naming scheme; the importer makes no claim that a re-export
//! restores the original external-reference graph.
//!
//! # Status honesty
//!
//! Every failure keeps its specific status (`Missing`, `BrowserUnavailable`,
//! `RefusedPath`, `Cycle`, `DepthExceeded`, `SkippedOverlay`, `Unsupported`,
//! `Unloaded`) plus a stable `import.xref_*` diagnostic. A reference is never
//! turned into an empty success; only a file that really loaded with no
//! model-space geometry is `Loaded`, and that gets an informational diagnostic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_resources::{
    check_xref_path, ResourceLimits, XrefFileSource, XrefPathVerdict, XrefSourceError,
};

use super::*;

mod remap;
pub(crate) use remap::remap_geometry;
use remap::remap_visibility;
#[cfg(test)]
pub(crate) use remap::UNMAPPED_BLOCK;

/// Default total byte budget for all referenced files in one import (1 GiB).
pub const DEFAULT_XREF_BYTE_BUDGET: u64 = 1024 * 1024 * 1024;

/// Session state for one synchronous xref tree.
///
/// The public [`XrefOptions`] stays a plain five-field policy value (so callers
/// may build it with a struct literal). The per-tree byte budget and active
/// path stack live here instead. Resolution is synchronous and recursion happens
/// on the importing thread, so a thread-local session is sufficient and no
/// shared owner is needed. A root resolution creates it and clears it with an
/// RAII guard; nested imports inherit it.
struct XrefSession {
    budget: u64,
    stack: Vec<PathBuf>,
}

thread_local! {
    static XREF_SESSION: RefCell<Option<XrefSession>> = const { RefCell::new(None) };
}

/// Clears the thread-local session when the *root* resolution finishes.
struct XrefSessionGuard {
    owner: bool,
}

impl Drop for XrefSessionGuard {
    fn drop(&mut self) {
        if self.owner {
            XREF_SESSION.with(|session| *session.borrow_mut() = None);
        }
    }
}

fn session_start(total_bytes_budget: u64) -> XrefSessionGuard {
    let owner = XREF_SESSION.with(|session| {
        let mut slot = session.borrow_mut();
        if slot.is_some() {
            false
        } else {
            *slot = Some(XrefSession {
                budget: total_bytes_budget,
                stack: Vec::new(),
            });
            true
        }
    });
    XrefSessionGuard { owner }
}

fn session_depth() -> usize {
    XREF_SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .map_or(0, |session| session.stack.len())
    })
}

fn session_remaining() -> u64 {
    XREF_SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .map_or(0, |session| session.budget)
    })
}

/// Charge `bytes` before parsing. Returns `false` when the budget is spent.
fn session_try_charge(bytes: u64) -> bool {
    XREF_SESSION.with(|session| {
        let mut slot = session.borrow_mut();
        let Some(session) = slot.as_mut() else {
            return false;
        };
        if bytes > session.budget {
            return false;
        }
        session.budget -= bytes;
        true
    })
}

fn session_contains(path: &Path) -> bool {
    XREF_SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.stack.iter().any(|entry| entry == path))
    })
}

fn session_push(path: PathBuf) {
    XREF_SESSION.with(|session| {
        if let Some(session) = session.borrow_mut().as_mut() {
            session.stack.push(path);
        }
    });
}

fn session_pop() {
    XREF_SESSION.with(|session| {
        if let Some(session) = session.borrow_mut().as_mut() {
            let _ = session.stack.pop();
        }
    });
}

/// Current xref nesting depth: the number of referenced files currently being
/// imported. The host drawing is depth `0`.
pub(crate) fn xref_current_depth() -> usize {
    session_depth()
}

/// Host policy and file access for external references.
///
/// Construct with [`XrefOptions::new`] or a struct literal, optionally tune the
/// public fields, and pass it as `ImportRequest::xref`. One value drives the
/// whole nested tree: a referenced drawing's own xrefs are resolved with the
/// same source, roots and budget.
///
/// ```ignore
/// let mut options = XrefOptions::new(source, "/drawings");
/// options.roots = vec![PathBuf::from("/drawings/lib")];
/// options.max_depth = 4;
/// ```
pub struct XrefOptions {
    /// Carrier that actually reads referenced bytes.
    pub source: Arc<dyn XrefFileSource>,
    /// Directory relative references in the host drawing resolve against.
    /// Nested imports replace this with the referenced file's parent directory.
    pub drawing_dir: PathBuf,
    /// Additional lexical roots an absolute reference may live under.
    pub roots: Vec<PathBuf>,
    /// Maximum nesting depth. `0` allows only the host's direct attachments.
    pub max_depth: usize,
    /// Total bytes every referenced file in this tree may consume.
    pub total_bytes_budget: u64,
}

impl XrefOptions {
    /// Create options for a host drawing at `drawing_dir`.
    pub fn new(source: Arc<dyn XrefFileSource>, drawing_dir: impl Into<PathBuf>) -> Self {
        Self {
            source,
            drawing_dir: drawing_dir.into(),
            roots: Vec::new(),
            max_depth: ResourceLimits::default().max_xref_depth,
            total_bytes_budget: DEFAULT_XREF_BYTE_BUDGET,
        }
    }

    /// Add allowed absolute roots, keeping the configured budget/depth.
    pub fn with_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots;
        self
    }

    /// Replace the total byte budget for this tree.
    pub fn with_budget(mut self, total_bytes_budget: u64) -> Self {
        self.total_bytes_budget = total_bytes_budget;
        self
    }

    /// Replace the nesting depth limit.
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    fn nested(&self, path: &Path) -> XrefOptions {
        let drawing_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.drawing_dir.clone());
        XrefOptions {
            source: Arc::clone(&self.source),
            drawing_dir,
            roots: self.roots.clone(),
            max_depth: self.max_depth,
            total_bytes_budget: session_remaining(),
        }
    }
}

/// Map an xref status to the render support scale.
///
/// `Loaded` is not forced here: its members decide the block's status through
/// the normal fixpoint. Every non-loaded state is an explicit loss, so it can
/// propagate through nested INSERTs and never become `Verified`.
pub(crate) fn xref_support_status(status: ExternalReferenceStatus) -> SupportStatus {
    match status {
        ExternalReferenceStatus::Loaded => SupportStatus::Verified,
        ExternalReferenceStatus::Unloaded | ExternalReferenceStatus::SkippedOverlay => {
            SupportStatus::Partial
        }
        ExternalReferenceStatus::Missing
        | ExternalReferenceStatus::BrowserUnavailable
        | ExternalReferenceStatus::RefusedPath
        | ExternalReferenceStatus::Cycle
        | ExternalReferenceStatus::DepthExceeded
        | ExternalReferenceStatus::Unsupported => SupportStatus::Unsupported,
    }
}

impl<'a> ImporterBuilder<'a> {
    /// Resolve every xref block of the drawing being imported.
    ///
    /// Called after [`ImporterBuilder::read_entities`] and before
    /// [`ImporterBuilder::resolve_block_status`]. Nested imports recursively
    /// invoke the same importer with a deeper [`XrefOptions`].
    pub(crate) fn resolve_xrefs(&mut self) -> CadResult<()> {
        let options = self.request.xref.clone();
        // A root resolution owns the thread-local session; nested imports find
        // it already active and inherit the same budget/stack.
        let _session = session_start(
            options
                .as_ref()
                .map_or(DEFAULT_XREF_BYTE_BUDGET, |options| {
                    options.total_bytes_budget
                }),
        );
        let depth = session_depth();
        let ids: Vec<BlockId> = self.xref_blocks.keys().copied().collect();
        for id in ids {
            self.check_cancelled()?;
            let Some(info) = self.xref_blocks.get(&id).cloned() else {
                continue;
            };
            // A disabled attachment is never read. It stays `Unloaded` and the
            // INSERT reports that state (see `insert_semantics`).
            if info.unloaded {
                self.push_xref_diagnostic(
                    "import.xref_unloaded",
                    &info,
                    "attachment is marked unloaded; file was not read",
                );
                self.finish_xref(id, ExternalReferenceStatus::Unloaded, None, None, depth)?;
                continue;
            }
            // No loader policy: the browser/no-filesystem shape. The row was
            // already recorded as BrowserUnavailable by `read_blocks`. This
            // check precedes the empty-path check so an empty-path xref with no
            // loader stays BrowserUnavailable and its INSERT message agrees
            // with the row.
            let Some(options) = options.clone() else {
                self.push_xref_diagnostic(
                    "import.xref_unavailable",
                    &info,
                    "this runtime has no external-reference file source",
                );
                continue;
            };
            if info.raw_path.trim().is_empty() {
                self.push_xref_diagnostic(
                    "import.xref_missing",
                    &info,
                    "attachment declares no file path",
                );
                self.finish_xref(
                    id,
                    ExternalReferenceStatus::Missing,
                    None,
                    Some("empty path".into()),
                    depth,
                )?;
                continue;
            }
            let limits = ResourceLimits {
                max_xref_depth: options.max_depth,
                ..ResourceLimits::default()
            };
            if limits.check_xref_depth(Some(&info.name), depth).is_err() {
                self.push_xref_diagnostic(
                    "import.xref_depth",
                    &info,
                    &format!(
                        "nesting depth {depth} exceeds the configured maximum {}",
                        options.max_depth
                    ),
                );
                self.finish_xref(
                    id,
                    ExternalReferenceStatus::DepthExceeded,
                    None,
                    Some(format!("depth {depth} > {}", options.max_depth)),
                    depth,
                )?;
                continue;
            }
            // Overlays are only skipped when they are nested. A top-level
            // overlay is an ordinary attachment.
            if depth > 0 && info.overlay {
                self.push_xref_diagnostic(
                    "import.xref_overlay_skipped",
                    &info,
                    "nested overlay is not resolved",
                );
                self.finish_xref(
                    id,
                    ExternalReferenceStatus::SkippedOverlay,
                    None,
                    Some("nested overlay".into()),
                    depth,
                )?;
                continue;
            }
            let normalized =
                match check_xref_path(&info.raw_path, &options.drawing_dir, &options.roots) {
                    XrefPathVerdict::Allowed { normalized } => normalized,
                    XrefPathVerdict::Refused { reason } => {
                        self.push_xref_diagnostic(
                            "import.xref_refused",
                            &info,
                            &format!("path refused ({reason})"),
                        );
                        self.finish_xref(
                            id,
                            ExternalReferenceStatus::RefusedPath,
                            None,
                            Some(reason.to_string()),
                            depth,
                        )?;
                        continue;
                    }
                };
            self.check_cancelled()?;
            if session_contains(&normalized) {
                self.push_xref_diagnostic(
                    "import.xref_cycle",
                    &info,
                    "reference cycle detected; not recursing",
                );
                self.finish_xref(
                    id,
                    ExternalReferenceStatus::Cycle,
                    None,
                    Some("cycle".into()),
                    depth,
                )?;
                continue;
            }

            match options.source.read(&normalized) {
                Ok(None) => {
                    self.push_xref_diagnostic(
                        "import.xref_unavailable",
                        &info,
                        "carrier has no filesystem for external references",
                    );
                    self.finish_xref(
                        id,
                        ExternalReferenceStatus::BrowserUnavailable,
                        None,
                        Some("no carrier filesystem".into()),
                        depth,
                    )?;
                }
                Ok(Some(bytes)) => {
                    // Charge before parsing; the full file must fit the
                    // remaining tree budget or it is not parsed at all.
                    if !session_try_charge(bytes.len() as u64) {
                        self.push_xref_diagnostic(
                            "import.xref_budget",
                            &info,
                            &format!(
                                "referenced file is {} bytes, over the remaining byte budget",
                                bytes.len()
                            ),
                        );
                        self.finish_xref(
                            id,
                            ExternalReferenceStatus::Unsupported,
                            None,
                            Some("xref byte budget exhausted".into()),
                            depth,
                        )?;
                        continue;
                    }
                    session_push(normalized.clone());
                    let nested_request = ImportRequest {
                        document: DocumentId(0),
                        database: DatabaseId(0),
                        bytes: Arc::clone(&bytes),
                        limits: self.request.limits,
                        generation: self.request.generation,
                        xref: Some(Arc::new(options.nested(&normalized))),
                    };
                    // Progress is intentionally not forwarded: the nested
                    // entity counts belong to a different document and would
                    // make the host bar jump backwards. Cancellation is.
                    let nested =
                        AcadrustImporter::default().import(&nested_request, self.cancelled);
                    session_pop();
                    match nested {
                        Err(CadError::Cancelled) => return Err(CadError::Cancelled),
                        Err(error) => {
                            self.push_xref_diagnostic(
                                "import.xref_unreadable",
                                &info,
                                &format!("referenced drawing could not be decoded: {error}"),
                            );
                            self.finish_xref(
                                id,
                                ExternalReferenceStatus::Unsupported,
                                Some(normalized.display().to_string()),
                                Some(error.to_string()),
                                depth,
                            )?;
                        }
                        Ok(drawing) => {
                            let empty_model = !drawing
                                .database
                                .entities()
                                .any(|entity| matches!(entity.space, SpaceId::Model));
                            self.splice_xref(id, &info, &drawing)?;
                            self.finish_xref(
                                id,
                                ExternalReferenceStatus::Loaded,
                                Some(normalized.display().to_string()),
                                None,
                                depth,
                            )?;
                            if empty_model {
                                self.push_xref_diagnostic(
                                    "import.xref_empty",
                                    &info,
                                    "referenced drawing has no model-space geometry",
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    let (status, code) = match &error {
                        XrefSourceError::Refused(_) => {
                            (ExternalReferenceStatus::RefusedPath, "import.xref_refused")
                        }
                        XrefSourceError::Missing(_) => {
                            (ExternalReferenceStatus::Missing, "import.xref_unreadable")
                        }
                        XrefSourceError::Unreadable(_) => (
                            ExternalReferenceStatus::Unsupported,
                            "import.xref_unreadable",
                        ),
                    };
                    self.push_xref_diagnostic(
                        code,
                        &info,
                        &format!("referenced file could not be read: {error}"),
                    );
                    self.finish_xref(
                        id,
                        status,
                        Some(normalized.display().to_string()),
                        Some(error.reason().to_string()),
                        depth,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Merge one successfully parsed referenced database into the host builder.
    ///
    /// The caller has already charged the bytes and detected cycles. This method
    /// never reads the source again.
    fn splice_xref(
        &mut self,
        host_block: BlockId,
        info: &XrefBlock,
        drawing: &ImportedDrawing,
    ) -> CadResult<()> {
        let prefix = info.name.clone();
        // Mark the host attachment loaded before computing member statuses, so
        // nothing treats it as a pending/missing placeholder.
        self.xref_pending.remove(&host_block);
        self.xref_block_status
            .insert(host_block, ExternalReferenceStatus::Loaded);

        // ---- Layers -------------------------------------------------------
        let mut layer_map: HashMap<LayerId, LayerId> = HashMap::new();
        for layer in drawing.database.layers() {
            let target = format!("{prefix}|{}", layer.name);
            let id = if let Some(id) = self.xref_layer_names.get(&target).copied() {
                // Host override (VISRETAIN-like): keep the host row exactly as
                // it is and only remap the referenced entity onto it.
                id
            } else if let Some(id) = self.layer_ids.get(&target).copied() {
                // A non-xref-dependent host row already owns the exact target
                // name. Reusing it is the only way to avoid two rows with one
                // name; the host attributes win, which is a declared override,
                // not a silent merge, so say so.
                self.xref_layer_names.insert(target.clone(), id);
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: "import.xref_layer_collision".into(),
                    message: format!(
                        "xref '{prefix}' layer '{}' reuses the existing host layer \
                         '{target}'; the host layer attributes win",
                        layer.name
                    ),
                });
                id
            } else {
                let id = LayerId(self.next_layer);
                self.next_layer += 1;
                self.builder.insert_layer(Layer {
                    id,
                    name: target.clone(),
                    visible: layer.visible,
                })?;
                let mut attributes = drawing.database.layer_attributes(layer.id);
                attributes.xref_dependent = true;
                self.builder.set_layer_attributes(id, attributes)?;
                self.xref_layer_names.insert(target, id);
                id
            };
            layer_map.insert(layer.id, id);
        }

        // ---- Blocks -------------------------------------------------------
        // Namespace every referenced definition (including anonymous blocks)
        // and allocate fresh host ids. Nothing reuses a referenced id.
        let mut block_map: HashMap<BlockId, BlockId> = HashMap::new();
        let mut block_names: HashMap<BlockId, String> = HashMap::new();
        for block in drawing.database.blocks() {
            let name = drawing
                .database
                .block_name(block.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("*U{}", block.id.0));
            let target = format!("{prefix}${}${}", 0, name);
            let id = BlockId(self.next_block);
            self.next_block += 1;
            block_map.insert(block.id, id);
            block_names.insert(id, target);
        }

        // Nested xref statuses: seed the status maps *before* member entities
        // are classified so an INSERT that reaches a missing nested reference
        // already inherits the loss.
        for reference in drawing.database.external_references() {
            let Some(old_block) = reference.block else {
                continue;
            };
            let Some(new_block) = block_map.get(&old_block).copied() else {
                continue;
            };
            if !reference.status.is_loaded() {
                self.xref_block_status.insert(new_block, reference.status);
                self.block_status
                    .insert(new_block, xref_support_status(reference.status));
                self.xref_causes.insert(
                    new_block,
                    XrefCause {
                        name: reference.name.clone(),
                        path: reference.raw_path.clone(),
                        status: reference.status,
                    },
                );
            }
            let name = block_names
                .get(&new_block)
                .cloned()
                .unwrap_or_else(|| format!("{prefix}${}${}", 0, reference.name));
            self.record_spliced_reference(
                name,
                reference.raw_path.clone(),
                reference.resolved_path.clone(),
                new_block,
                reference.overlay,
                reference.depth as usize,
                reference.status,
                reference.reason.clone(),
            )?;
        }

        // ---- Entities -----------------------------------------------------
        let mut entity_map: HashMap<EntityId, EntityId> = HashMap::new();
        let mut members: HashMap<BlockId, Vec<EntityId>> = HashMap::new();
        let mut paper_skipped = 0usize;
        let mut block_member_dropped = 0usize;
        let mut unmapped_blocks: Vec<BlockId> = Vec::new();
        for entity in drawing.database.entities() {
            let target_block = match entity.space {
                SpaceId::Model => host_block,
                SpaceId::Block(old) => match block_map.get(&old).copied() {
                    Some(block) => block,
                    None => {
                        // No mapped definition; nothing may dangle into it, but
                        // the loss is counted instead of silently dropping it.
                        block_member_dropped += 1;
                        continue;
                    }
                },
                // An xref is model content: paper-space sheets are not merged.
                SpaceId::Paper(_) => {
                    paper_skipped += 1;
                    continue;
                }
            };
            // Referenced content is untrusted like the host drawing: it counts
            // against the same entity budget as source entities.
            if self.entity_total >= self.request.limits.max_entities {
                self.dropped += 1;
                continue;
            }
            let old_id = entity.id;
            let new_id = EntityId(self.next_entity);
            let new_object = ObjectId(self.next_object);
            self.next_entity += 1;
            self.next_object += 1;
            entity_map.insert(old_id, new_id);
            let layer = layer_map
                .get(&entity.layer)
                .copied()
                .unwrap_or_else(|| self.host_layer_zero());
            let geometry = remap_geometry(&entity.geometry, &block_map, &mut unmapped_blocks);
            let geometry_support = display_support(&geometry);
            let insert_blocks = referenced_blocks(&geometry);
            self.builder.insert_entity(DbEntity {
                object: DbObject {
                    id: new_object,
                    type_key: entity.object.type_key.clone(),
                    revision: entity.object.revision,
                    source_handle: entity.object.source_handle.clone(),
                },
                id: new_id,
                layer,
                space: SpaceId::Block(target_block),
                geometry,
                draw_order: entity.draw_order,
            })?;
            self.builder.set_entity_render_attributes(
                new_id,
                drawing.database.entity_render_attributes(old_id),
            )?;
            let mut render = geometry_support.0;
            let mut pick = geometry_support.1;
            for block in &insert_blocks {
                let status = self
                    .block_status
                    .get(block)
                    .copied()
                    .unwrap_or(SupportStatus::Unverified);
                render = weaker(render, status);
                pick = weaker(pick, status);
            }
            self.block_members
                .entry(target_block)
                .or_default()
                .push(BlockMember {
                    render,
                    insert_blocks: insert_blocks.clone(),
                });
            // The reference list is kept in `block_members` for the status
            // fixpoint; spliced entities are not host entities, so no
            // report/capability row exists to reclassify here.
            members.entry(target_block).or_default().push(new_id);
            self.entity_total += 1;
        }
        if paper_skipped > 0 {
            self.push_xref_diagnostic(
                "import.xref_paper_skipped",
                info,
                &format!(
                    "{paper_skipped} paper-space entities were not merged (an xref is model content)"
                ),
            );
        }
        if block_member_dropped > 0 {
            self.push_xref_diagnostic(
                "import.xref_block_member_dropped",
                info,
                &format!(
                    "{block_member_dropped} block-space entities had no mapped block definition and were dropped"
                ),
            );
        }
        for old in unmapped_blocks {
            let name = drawing
                .database
                .block_name(old)
                .map(str::to_string)
                .unwrap_or_else(|| format!("*U{}", old.0));
            self.push_xref_diagnostic(
                "import.xref_block_unmapped",
                info,
                &format!(
                    "referenced block '{name}' ({old:?}) has no host mapping; replaced with the unmapped sentinel"
                ),
            );
        }

        // ---- Block definitions -------------------------------------------
        for (old_block, new_block) in &block_map {
            let block_members = members.remove(new_block).unwrap_or_default();
            let visibility = drawing
                .database
                .block_dynamic_visibility(*old_block)
                .map(|visibility| remap_visibility(visibility, &entity_map));
            self.builder.insert_block(BlockDefinition {
                id: *new_block,
                entities: block_members.clone(),
                dynamic_visibility: visibility,
            })?;
            self.block_entities.insert(*new_block, block_members);
            // An empty referenced block still gets a status entry so a nested
            // INSERT that reaches it is not silently considered unknown.
            self.block_members.entry(*new_block).or_default();
            if let Some(name) = block_names.get(new_block) {
                let base = drawing
                    .database
                    .block_base_point(*old_block)
                    .unwrap_or_default();
                self.builder.set_block_metadata(*new_block, name, base)?;
            }
        }

        // The host xref block keeps its own name/base metadata and receives
        // the merged member list. Existing members (normally none for a source
        // xref block) are preserved rather than dropped.
        let mut merged = self.block_entities.remove(&host_block).unwrap_or_default();
        merged.extend(members.remove(&host_block).unwrap_or_default());
        self.builder.insert_block(BlockDefinition {
            id: host_block,
            entities: merged.clone(),
            dynamic_visibility: None,
        })?;
        self.block_entities.insert(host_block, merged);

        // Propagate stable xref diagnostics from the nested import. Object ids
        // are from another database, so they are deliberately dropped rather
        // than remapped into a misleading host id.
        for diagnostic in &drawing.report.diagnostics {
            if diagnostic.code.starts_with("import.xref") {
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: diagnostic.code.clone(),
                    message: diagnostic.message.clone(),
                });
            }
        }

        Ok(())
    }

    /// Apply the final block-status fixpoint to every INSERT-bearing entity.
    ///
    /// Entity classification (and the pre-fixpoint capability/render snapshot)
    /// runs before [`ImporterBuilder::resolve_xrefs`]. A pending xref therefore
    /// looks like an ordinary empty block at that point. This pass folds the
    /// terminal status into the report, capabilities and model render chain:
    ///
    /// * every recorded entity whose block chain reaches a non-loaded xref is
    ///   weakened by both the cause's support status and the resolved block
    ///   status (so a nested unresolved reference weakens its host INSERT);
    /// * the entity's capability row is weakened (never left `Verified`);
    /// * model-space losses weaken `model_render` and enter its type list;
    /// * one stable `import.xref_insert_missing` names the xref, status and path.
    pub(crate) fn reclassify_xref_inserts(&mut self) {
        // Host xref blocks: their terminal status is now known. Nested causes
        // were registered by `splice_xref`; `or_insert` keeps the first cause.
        let host_causes: Vec<(BlockId, XrefCause)> = self
            .xref_blocks
            .iter()
            .filter_map(|(id, info)| {
                let status = self.xref_block_status.get(id).copied()?;
                (!status.is_loaded()).then(|| {
                    (
                        *id,
                        XrefCause {
                            name: info.name.clone(),
                            path: info.raw_path.clone(),
                            status,
                        },
                    )
                })
            })
            .collect();
        for (id, cause) in host_causes {
            self.xref_causes.entry(id).or_insert(cause);
        }

        let chains = std::mem::take(&mut self.insert_chains);
        for chain in chains {
            let mut status = SupportStatus::Verified;
            let mut cause = None;
            for block in &chain.insert_blocks {
                status = weaker(
                    status,
                    self.block_status
                        .get(block)
                        .copied()
                        .unwrap_or(SupportStatus::Unverified),
                );
                if cause.is_none() {
                    cause = self.xref_cause_for_block(*block, &mut BTreeSet::new());
                }
            }
            let Some(cause) = cause else {
                continue;
            };
            let status = weaker(status, xref_support_status(cause.status));
            if status == SupportStatus::Verified {
                continue;
            }
            if let Some(entry) = self.capabilities.get_mut(&chain.class_name) {
                entry.semantic = weaker(entry.semantic, status);
                entry.render = weaker(entry.render, status);
                entry.pick = weaker(entry.pick, status);
                entry.measure = weaker(entry.measure, status);
            }
            if matches!(chain.space, SpaceId::Model) {
                self.model_render = weaker(self.model_render, status);
                self.model_render_types.insert(chain.class_name.clone());
            }
            self.diagnostics.push(Diagnostic {
                object: Some(chain.object),
                code: "import.xref_insert_missing".into(),
                message: format!(
                    "INSERT reaches unresolved xref '{}' ({}): {}",
                    cause.name,
                    if cause.path.is_empty() {
                        "no path"
                    } else {
                        cause.path.as_str()
                    },
                    cause.status.as_str()
                ),
            });
        }
    }

    /// Find the first non-loaded xref reachable from `block`, following nested
    /// block members. Cycles terminate through `visited`.
    fn xref_cause_for_block(
        &self,
        block: BlockId,
        visited: &mut BTreeSet<BlockId>,
    ) -> Option<XrefCause> {
        if !visited.insert(block) {
            return None;
        }
        if let Some(cause) = self.xref_causes.get(&block) {
            return Some(cause.clone());
        }
        for member in self.block_members.get(&block).into_iter().flatten() {
            for child in &member.insert_blocks {
                if let Some(cause) = self.xref_cause_for_block(*child, visited) {
                    return Some(cause);
                }
            }
        }
        None
    }

    fn host_layer_zero(&self) -> LayerId {
        self.layer_ids.get("0").copied().unwrap_or(LayerId(0))
    }

    fn push_xref_diagnostic(&mut self, code: &str, info: &XrefBlock, detail: &str) {
        self.diagnostics.push(Diagnostic {
            object: None,
            code: code.into(),
            message: format!(
                "xref '{}' ({}): {detail}",
                info.name,
                if info.raw_path.is_empty() {
                    "no path"
                } else {
                    info.raw_path.as_str()
                }
            ),
        });
    }

    /// Record the terminal status of one xref block and its provenance row.
    fn finish_xref(
        &mut self,
        id: BlockId,
        status: ExternalReferenceStatus,
        resolved_path: Option<String>,
        reason: Option<String>,
        depth: usize,
    ) -> CadResult<()> {
        self.xref_pending.remove(&id);
        self.xref_block_status.insert(id, status);
        let Some(info) = self.xref_blocks.get(&id).cloned() else {
            return Ok(());
        };
        // The builder's provenance insert is append-only. A terminal row added
        // by `read_blocks` (Unloaded / BrowserUnavailable) is already recorded;
        // never attempt a duplicate insert. The key is the block id, so a
        // nested namespaced name that collides with a host name cannot suppress
        // an unrelated attachment.
        if self.xref_recorded.insert(id) {
            self.builder.insert_external_reference(ExternalReference {
                name: info.name,
                raw_path: info.raw_path,
                resolved_path,
                block: Some(id),
                overlay: info.overlay,
                depth: depth as u32,
                status,
                reason,
            })?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_spliced_reference(
        &mut self,
        name: String,
        raw_path: String,
        resolved_path: Option<String>,
        block: BlockId,
        overlay: bool,
        depth: usize,
        status: ExternalReferenceStatus,
        reason: Option<String>,
    ) -> CadResult<()> {
        if !self.xref_recorded.insert(block) {
            return Ok(());
        }
        self.builder.insert_external_reference(ExternalReference {
            name,
            raw_path,
            resolved_path,
            block: Some(block),
            overlay,
            depth: depth as u32,
            status,
            reason,
        })
    }
}
