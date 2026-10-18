use super::*;

/// Build the **sub-element** selection highlight overlay for `selection`.
///
/// Whole-entity refs are the renderer's in-stream flags path (see
/// [`selection_entity_flags`]): they produce no batch here, so a whole-entity
/// selection never double-highlights. Only mesh-face sub-elements still need a
/// small separate draw, because the static merged streams do not re-upload per
/// selection change (the user explicitly authorized a separate draw for the few
/// selected sub-elements). Tool previews are built by [`preview_overlay`].
///
/// Only the selected **sub-element** refs are resolved: each ref's own `INSERT`
/// chain is walked with [`crate::picking::resolve_pick_transform`] instead of
/// scanning the whole drawing, so a block child reached through two placements
/// becomes two distinct refs and highlights independently without cloning every
/// entity's geometry. The resolved geometry is tessellated by the default
/// representation provider (the same discretisation the base scene uses) and
/// converted to overlay batches by [`cad_scene::highlight_batches`].
///
/// A ref that no longer resolves (deleted entity, foreign document, or an
/// `INSERT` root that only exists as an expansion) produces an explicit
/// `highlight.unresolved` diagnostic and **no** batch — never a fabricated one.
/// Model space is the highlighted space; paper-space refs resolve only when they
/// are reached through the model-space expansion.
pub fn selection_highlight(
    database: &DrawingDatabase,
    selection: &SelectionSet,
    document: DocumentId,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    // Entity-level refs are the flags path; only sub-elements are overlaid.
    let sub_element_refs: Vec<SelectionRef> = selection
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_some())
        .cloned()
        .collect();
    if sub_element_refs.is_empty() {
        return out;
    }

    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(document, cad_domain::TolerancePolicy::default(), stamp);
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }

    let mut representations: Vec<DisplayRepresentation> = Vec::new();
    let mut build_reasons: Vec<Diagnostic> = Vec::new();
    // Two refs that share an `(entity, instance)` but differ in sub-element are
    // the *same* object to the old full scan, which matched both against one
    // item; resolve each such object exactly once so the batches stay identical.
    let mut resolved: HashSet<(DocumentId, EntityId, Vec<EntityId>)> = HashSet::new();
    for reference in &sub_element_refs {
        let key = (
            reference.document,
            reference.entity,
            reference.instance.0.clone(),
        );
        if !resolved.insert(key) {
            continue;
        }
        let Some(transform) = crate::picking::resolve_pick_transform(database, document, reference)
        else {
            // The full scan would not have produced this ref either; the missing
            // representation becomes an explicit `highlight.unresolved`.
            continue;
        };
        match representation_for_ref(&registry, database, reference, &transform, &context) {
            Some(representation) => representations.push(representation),
            None => {
                // The ref resolved against this database, so a missing entity
                // here is an internal inconsistency, not a user error; report it
                // rather than drawing something wrong.
                build_reasons.push(Diagnostic {
                    object: None,
                    code: "highlight.entity-missing".into(),
                    message: format!(
                        "selection entity {} is present in the selection but not in the database",
                        reference.entity.0
                    ),
                });
            }
        }
    }

    let HighlightScene {
        batches,
        completeness,
        diagnostics,
        ..
    } = cad_scene::highlight_batches(
        &sub_element_refs,
        representations.iter(),
        |_| true,
        &HighlightOptions::default(),
    );
    out.batches = batches;
    out.diagnostics = diagnostics;
    out.completeness = completeness;
    for reason in build_reasons {
        out.diagnostics.push(reason);
    }
    out
}

/// The entity-level flags resolved from an [`OverlayInputs`].
#[derive(Debug, Clone, PartialEq)]
pub struct EntityFlagScene {
    /// One `(SelectionRef, bits)` per entity-level ref, in selection/hover/hidden
    /// order, with the bits of the same placement OR-ed. Bits are
    /// [`cad_scene::FLAG_SELECTED`] / [`FLAG_HOVERED`] / [`FLAG_HIDDEN`].
    pub flags: Vec<(SelectionRef, u32)>,
    /// `highlight.unresolved` per entity-level ref the database does not resolve.
    pub diagnostics: Vec<Diagnostic>,
    pub completeness: Completeness,
}

/// Resolve the entity-level selection/hover/hidden inputs into renderer flags.
///
/// This is the flags twin of [`selection_highlight`]: it reuses the same
/// resolve-only-refs logic (`resolve_pick_transform` per ref, never a full pick
/// scan) but produces `(SelectionRef, bits)` pairs for the renderer's
/// `set_entity_flags` instead of overlay geometry. A ref that does not resolve
/// (deleted entity, foreign document, or an `INSERT` root that only exists as an
/// expansion) produces an explicit `highlight.unresolved` diagnostic and **no**
/// flag — never a silent miss. Sub-element refs are ignored here: they are the
/// overlay path.
///
/// `inputs.visibility.selection_highlight` gates the selected tint (turning the
/// selection highlight off clears it); hover and hidden are independent transient
/// states and always participate.
pub fn selection_entity_flags(
    database: &DrawingDatabase,
    inputs: &OverlayInputs,
    document: DocumentId,
) -> EntityFlagScene {
    let mut out = EntityFlagScene {
        flags: Vec::new(),
        diagnostics: Vec::new(),
        completeness: Completeness::Complete,
    };
    let resolve = |reference: &SelectionRef, bits: u32, out: &mut EntityFlagScene| {
        if crate::picking::resolve_pick_transform(database, document, reference).is_none() {
            let message = format!(
                "selected entity {} is not present in the document",
                reference.entity.0
            );
            out.diagnostics.push(Diagnostic {
                object: None,
                code: "highlight.unresolved".into(),
                message: message.clone(),
            });
            out.completeness = out
                .completeness
                .clone()
                .combine(Completeness::Missing(vec![message]));
            return;
        }
        match out
            .flags
            .iter_mut()
            .find(|(existing, _)| same_entity_instance(existing, reference))
        {
            Some(entry) => entry.1 |= bits,
            None => out.flags.push((reference.clone(), bits)),
        }
    };

    if inputs.visibility.selection_highlight {
        for reference in inputs
            .selection
            .refs()
            .iter()
            .filter(|reference| reference.sub_element.is_none())
        {
            resolve(reference, cad_scene::FLAG_SELECTED, &mut out);
        }
    }
    for reference in inputs
        .hovered
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_none())
    {
        resolve(reference, cad_scene::FLAG_HOVERED, &mut out);
    }
    for reference in inputs
        .hidden
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_none())
    {
        resolve(reference, cad_scene::FLAG_HIDDEN, &mut out);
    }
    out
}

/// Whether two refs name the same entity reached through the same instance
/// chain, ignoring the sub-element (which the highlight face filter applies).
///
/// The flags resolver merges bits by this identity, and the test-only full-scan
/// reference uses the same predicate.
pub(super) fn same_entity_instance(a: &SelectionRef, b: &SelectionRef) -> bool {
    a.document == b.document && a.entity == b.entity && a.instance == b.instance
}

/// Build a `DisplayRepresentation` for one resolved selection ref.
///
/// The provider tessellates the entity's `SemanticGeometry`; the ref's
/// accumulated `INSERT` transform is then applied to each fragment and the
/// fragment's `source` is set to the ref's `(entity, instance)` so two
/// placements never collapse to one. Returns `None` when the database entity no
/// longer exists.
fn representation_for_ref(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    reference: &SelectionRef,
    transform: &Transform3,
    context: &RepresentationContext,
) -> Option<DisplayRepresentation> {
    let entity = database.entity(reference.entity)?;
    let mut representation = match registry.build(entity, context) {
        Ok(representation) => representation,
        Err(error) => DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Missing(vec![error.to_string()]),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "highlight.representation".into(),
                message: error.to_string(),
            }],
        },
    };
    for fragment in &mut representation.fragments {
        fragment.primitive = fragment.primitive.transformed(transform);
        // The sub-element is applied by `highlight_batches`'s face filter, so the
        // fragment carries only the `(entity, instance)` identity (as the old
        // full-scan item did).
        fragment.source = SelectionRef {
            sub_element: None,
            ..reference.clone()
        };
    }
    Some(representation)
}
