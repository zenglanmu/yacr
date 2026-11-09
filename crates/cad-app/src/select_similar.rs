//! Read-only `SELECTSIMILAR` over the pick-item catalogue of the active space.
//!
//! The match follows AutoCAD 2026 semantics. Only objects of the **same type**
//! ([`SemanticGeometry`] discriminant) are ever considered, and the default
//! similarity set is **layer + object type only**. Every other modelled
//! dimension is opt-in in [`SelectSimilarSettings`]: colour, linetype,
//! lineweight, `Name` (block references) and `Object style` (text/dimension
//! style). Transparency is not an AutoCAD criterion and is not compared.
//!
//! `Name` applies to block-reference content: the pick catalogue expands an
//! `INSERT` into its members (the same expansion `SELECTALL` uses), and every
//! member carries the instance path of the reference. A member's `Name` is
//! therefore the **root `INSERT`'s block name**, resolved from that path, so two
//! members of different block references never match on `Name` while members of
//! the same reference always do. Block definitions are not drawing objects and
//! are never compared.
//!
//! `Object style` compares the resolved text-style name for `TEXT`/`MTEXT` and
//! the dimension style name for `DIMENSION`. A type that carries no style in the
//! database is simply not comparable on that dimension (the criterion is
//! skipped rather than treating a missing row as a value).
//!
//! The accepted set is reported through `selectsimilar.settings`, so a caller
//! never has to infer what was matched.
//!
//! Model space and paper space are both supported: the catalogue belongs to
//! [`SessionState::active_space`], every seed must live in that same space
//! (`selectsimilar.space` otherwise), and a paper-space layout is enumerated
//! from its own top-level entities with `INSERT`s expanded exactly like the
//! model-space pick builder. A block-space object is never addressable and is
//! refused by name.
//!
//! The catalogue is otherwise the same one `SELECTALL` uses: the shared
//! [`validate_branch_with_vendor`] preflight (cycles, depth limits, unresolved
//! dynamic visibility, missing block members and unsupported opaque geometry are
//! explicit refusals) and the same [`ref_visibility`] filter. Every currently
//! selected ref must resolve to a visible catalogue item; a ref that does not is
//! [`CadError::StaleResult`] rather than a silent no-op.
//!
//! The result is a **union** with the current selection (order preserved), never
//! a replacement. No database write, revision bump, history entry or `ChangeSet`
//! is produced, so the command is legal in Viewer and Work.

use crate::picking::{drawing_pick_items_with_vendor, ref_visibility};
use crate::select_all::validate_branch_with_vendor;
use crate::{
    Application, Command, CommandOutcome, CommandPayload, SelectSimilarSettings, SelectionSet,
    SelectionToolPayload, SessionState,
};
use cad_db::{DbEntity, DrawingDatabase, EntityRenderAttributes, MAX_INSTANCE_DEPTH};
use cad_domain::{
    BlockId, CadError, CadResult, Diagnostic, EntityId, InstancePath, SelectionRef,
    SemanticGeometry, SpaceId,
};
use cad_spatial::PickItem;

/// One candidate the similarity comparison can see.
struct SimilarItem {
    source: SelectionRef,
    geometry: SemanticGeometry,
    /// The top-level space the item was enumerated from (`Model` or the active
    /// paper layout). A block member inherits the space of its root `INSERT`.
    space: SpaceId,
    layer: cad_domain::LayerId,
    attributes: EntityRenderAttributes,
    /// `Name`: the block name when this item is an `INSERT` reference.
    block_name: Option<String>,
    /// `Object style`: the resolved text-style name for TEXT/MTEXT.
    text_style: Option<String>,
    /// `Object style`: the dimension style name for DIMENSION.
    dim_style: Option<String>,
}

impl Application {
    pub(crate) fn select_similar(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let settings = select_similar_settings(command)?;
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != command.document {
            return Err(CadError::StaleResult);
        }
        if session.selection.is_empty() {
            return Err(CadError::InvalidInput(
                "selectsimilar.selection_required: select at least one object first".into(),
            ));
        }

        let database = &document.drawing;
        let space = session.active_space.clone();
        if !matches!(space, SpaceId::Model | SpaceId::Paper(_)) {
            return Err(CadError::Unsupported(
                "selectsimilar.space: block-space objects are not addressable".into(),
            ));
        }

        // Every seed must live in the active space; a selection from another
        // space is a named refusal rather than a silently mixed match.
        for reference in session.selection.refs() {
            let Some(seed_space) = ref_top_space(database, reference) else {
                return Err(CadError::StaleResult);
            };
            if seed_space != space {
                return Err(CadError::Unsupported(
                    "selectsimilar.space: every selected object must be in the active space".into(),
                ));
            }
        }

        let scope = database.root_layer_scope();
        let vendor = self.vendor_representations.as_deref();
        let mut items = Vec::new();
        build_catalogue(
            database,
            command.document,
            session,
            space.clone(),
            vendor,
            &mut items,
        )?;

        // The visible catalogue: every ref's chain must resolve (a stale chain
        // is StaleResult, exactly like before) and a hidden item is not a match.
        let mut visible = Vec::with_capacity(items.len());
        for item in items {
            let shown = ref_visibility(
                database,
                &session.layer_overrides,
                &session.hidden_entities,
                scope,
                &item.source,
            )
            .ok_or(CadError::StaleResult)?;
            if shown {
                visible.push(item);
            }
        }

        // Resolve every current selection into a catalogue seed. A ref that is
        // not in the visible catalogue (deleted, foreign, hidden or otherwise
        // unresolved) cannot be the input of a real similarity match.
        let mut seeds: Vec<usize> = Vec::new();
        for reference in session.selection.refs() {
            let index = visible
                .iter()
                .position(|item| SelectionSet::same_object(&item.source, reference))
                .ok_or(CadError::StaleResult)?;
            if !seeds.contains(&index) {
                seeds.push(index);
            }
        }

        let before = session.selection.len();
        for item in &visible {
            if seeds
                .iter()
                .any(|seed| similar(&visible[*seed], item, &settings))
            {
                session.selection.insert(item.source.clone());
            }
        }
        let added = session.selection.len() - before;
        session.begin_selecting();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "selectsimilar.result".into(),
                    message: format!(
                        "SELECTSIMILAR: added {added} similar object(s) to the selection"
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "selectsimilar.settings".into(),
                    message: format!("SELECTSIMILAR: matching on {}", settings_label(&settings)),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }
}

/// Read the optional typed settings; `None` means AutoCAD's all-on default.
fn select_similar_settings(command: &Command) -> CadResult<SelectSimilarSettings> {
    match &command.payload {
        CommandPayload::None => Ok(SelectSimilarSettings::default()),
        CommandPayload::SelectionTool(payload) => match payload.as_ref() {
            SelectionToolPayload::SelectSimilar(settings) => Ok(*settings),
            _ => Err(CadError::InvalidInput(
                "selectsimilar.invalid_payload: SELECTSIMILAR accepts only its own settings".into(),
            )),
        },
        _ => Err(CadError::InvalidInput(
            "selectsimilar.invalid_payload: SELECTSIMILAR takes no payload or its typed settings"
                .into(),
        )),
    }
}

/// The top-level space a ref belongs to: the first `INSERT` of its instance
/// path, or the entity itself for a top-level ref.
fn ref_top_space(database: &DrawingDatabase, reference: &SelectionRef) -> Option<SpaceId> {
    let id = reference
        .instance
        .0
        .first()
        .copied()
        .unwrap_or(reference.entity);
    database.entity(id).map(|entity| entity.space.clone())
}

/// Enumerate the active space with the same preflight/visibility rules
/// `SELECTALL` uses.
fn build_catalogue(
    database: &DrawingDatabase,
    document: cad_domain::DocumentId,
    session: &SessionState,
    space: SpaceId,
    vendor: Option<&crate::vendor::VendorRepresentationStore>,
    out: &mut Vec<SimilarItem>,
) -> CadResult<()> {
    let scope = database.root_layer_scope();
    match space {
        SpaceId::Model => {
            for entity in database.model_space() {
                validate_branch_with_vendor(
                    database,
                    session,
                    entity,
                    scope,
                    &mut Vec::new(),
                    vendor,
                )?;
            }
            for item in drawing_pick_items_with_vendor(database, document, vendor) {
                if let Some(candidate) = candidate_from_pick_item(database, &item, SpaceId::Model) {
                    out.push(candidate);
                }
            }
        }
        SpaceId::Paper(layout) => {
            for entity in database.layout_entities(layout) {
                validate_branch_with_vendor(
                    database,
                    session,
                    entity,
                    scope,
                    &mut Vec::new(),
                    vendor,
                )?;
            }
            let mut stack: Vec<BlockId> = Vec::new();
            let paper = SpaceId::Paper(layout);
            for entity in database.layout_entities(layout) {
                collect_entity(
                    database,
                    document,
                    entity,
                    paper.clone(),
                    &[],
                    0,
                    &mut stack,
                    out,
                );
            }
        }
        SpaceId::Block(_) => {}
    }
    Ok(())
}

fn candidate_from_pick_item(
    database: &DrawingDatabase,
    item: &PickItem,
    space: SpaceId,
) -> Option<SimilarItem> {
    let entity = database.entity(item.source.entity)?;
    let (_, text_style, dim_style) = classify(database, &item.geometry);
    let block_name = reference_block_name(database, &item.source.instance.0, &item.geometry);
    Some(SimilarItem {
        source: item.source.clone(),
        geometry: item.geometry.clone(),
        space,
        layer: entity.layer,
        attributes: database.entity_render_attributes(item.source.entity),
        block_name,
        text_style,
        dim_style,
    })
}

/// The block name a catalogue item belongs to, if any.
///
/// A member expanded out of a reference carries the reference's instance path;
/// its `Name` is the **root** `INSERT`'s block name (the reference the user
/// picked). A top-level `INSERT` geometry is also accepted for completeness,
/// although the pick catalogue normally expands it.
fn reference_block_name(
    database: &DrawingDatabase,
    path: &[EntityId],
    geometry: &SemanticGeometry,
) -> Option<String> {
    let block = match path.first() {
        Some(root) => match database.entity(*root).map(|entity| &entity.geometry) {
            Some(SemanticGeometry::Insert { block, .. }) => Some(*block),
            _ => None,
        },
        None => match geometry {
            SemanticGeometry::Insert { block, .. } => Some(*block),
            _ => None,
        },
    }?;
    database.block_name(block).map(str::to_string)
}

/// The `Name` and `Object style` values a geometry carries, if the database can
/// resolve them. A missing style row is `None` (the criterion is not
/// comparable), never a fabricated name.
fn classify(
    database: &DrawingDatabase,
    geometry: &SemanticGeometry,
) -> (Option<String>, Option<String>, Option<String>) {
    match geometry {
        SemanticGeometry::Insert { block, .. } => {
            (database.block_name(*block).map(str::to_string), None, None)
        }
        SemanticGeometry::Text { style, .. } => (
            None,
            database.style(*style).map(|style| style.name.clone()),
            None,
        ),
        SemanticGeometry::Dimension(dimension) => {
            (None, None, Some(dimension.style_name().to_string()))
        }
        _ => (None, None, None),
    }
}

/// Expand one paper-space top-level entity into candidates, mirroring the
/// model-space pick expansion (an `INSERT` contributes its members, never
/// itself; cycles/depth limits are cut exactly like the preflight).
#[allow(clippy::too_many_arguments)]
fn collect_entity(
    database: &DrawingDatabase,
    document: cad_domain::DocumentId,
    entity: &DbEntity,
    space: SpaceId,
    path: &[EntityId],
    depth: usize,
    stack: &mut Vec<BlockId>,
    out: &mut Vec<SimilarItem>,
) {
    if let SemanticGeometry::Insert { block, .. } = &entity.geometry {
        if depth >= MAX_INSTANCE_DEPTH || stack.contains(block) {
            return;
        }
        stack.push(*block);
        let mut child_path = path.to_vec();
        child_path.push(entity.id);
        for member in database.block_entities(*block) {
            collect_entity(
                database,
                document,
                member,
                space.clone(),
                &child_path,
                depth + 1,
                stack,
                out,
            );
        }
        stack.pop();
        return;
    }
    let (_, text_style, dim_style) = classify(database, &entity.geometry);
    let block_name = reference_block_name(database, path, &entity.geometry);
    out.push(SimilarItem {
        source: SelectionRef {
            document,
            entity: entity.id,
            instance: InstancePath(path.to_vec()),
            sub_element: None,
        },
        geometry: entity.geometry.clone(),
        space,
        layer: entity.layer,
        attributes: database.entity_render_attributes(entity.id),
        block_name,
        text_style,
        dim_style,
    });
}

/// Whether `candidate` matches `seed` on every enabled dimension.
///
/// Object type is mandatory: AutoCAD never considers a different object type
/// similar, regardless of the Settings toggles.
fn similar(seed: &SimilarItem, candidate: &SimilarItem, settings: &SelectSimilarSettings) -> bool {
    if seed.space != candidate.space {
        return false;
    }
    if std::mem::discriminant(&seed.geometry) != std::mem::discriminant(&candidate.geometry) {
        return false;
    }
    if settings.layer && seed.layer != candidate.layer {
        return false;
    }
    if settings.color && seed.attributes.color != candidate.attributes.color {
        return false;
    }
    if settings.linetype && seed.attributes.linetype != candidate.attributes.linetype {
        return false;
    }
    if settings.lineweight && seed.attributes.lineweight != candidate.attributes.lineweight {
        return false;
    }
    if settings.name {
        if let (Some(seed_name), Some(candidate_name)) = (&seed.block_name, &candidate.block_name) {
            if seed_name != candidate_name {
                return false;
            }
        }
    }
    if settings.object_style {
        if let (Some(seed_style), Some(candidate_style)) = (&seed.text_style, &candidate.text_style)
        {
            if seed_style != candidate_style {
                return false;
            }
        }
        if let (Some(seed_style), Some(candidate_style)) = (&seed.dim_style, &candidate.dim_style) {
            if seed_style != candidate_style {
                return false;
            }
        }
    }
    true
}

fn settings_label(settings: &SelectSimilarSettings) -> String {
    let mut parts: Vec<&str> = vec!["type"];
    if settings.layer {
        parts.push("layer");
    }
    if settings.color {
        parts.push("color");
    }
    if settings.linetype {
        parts.push("linetype");
    }
    if settings.lineweight {
        parts.push("lineweight");
    }
    if settings.name {
        parts.push("name");
    }
    if settings.object_style {
        parts.push("object style");
    }
    parts.join(", ")
}
