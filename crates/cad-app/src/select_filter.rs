//! The 2026-11-07 selection/filter/visibility batch (`QSELECT`, `FILTER`,
//! `GETSEL`, `SELECTCOUNT`, `HIDEOBJECTS`, `ISOLATEOBJECTS`,
//! `UNISOLATEOBJECTS`) plus the AutoCAD-2026 dialog semantics of `QSELECT` and
//! `FILTER`.
//!
//! `SSX` is dispatched by [`crate::Application::select_matching_properties`] but
//! implemented in [`crate::ssx`]: it is the Express-Tool criteria builder, not a
//! template-signature query, so it has its own module and contract.
//!
//! Every command here is a **selection or session** operation: it mutates
//! `SessionState::selection` / `SessionState::hidden_entities` and calls
//! [`SessionState::begin_selecting`] when it changes the selection. It never
//! opens a transaction, bumps a drawing revision, writes `cad-history` or
//! produces a `ChangeSet`. The full per-command contract is in
//! [`crate::command_id::select_filter`]; this module is its implementation.
//!
//! ## Candidate set
//!
//! The **entire-drawing** scope enumerates every **top-level entity in the
//! current space** (model space or the active paper layout). AutoCAD's wording
//! is "entire drawing"; current-space semantics are a **declared Build choice**
//! (current-space scoping is not documented by Autodesk). A filter is a data
//! query, not a screen query, so layer off/frozen state does not exclude an
//! entity. The **current-selection** scope reads the existing selection refs.
//!
//! **Nested block objects are never selected or counted**: only refs with an
//! empty instance path and no sub-element are candidates, so a block-definition
//! member is not addressable by QSELECT/FILTER even if it is present in a
//! selection. Zero matches is a named diagnostic, not an error.
//!
//! ## Structured dialog semantics
//!
//! The legacy `CommandPayload::SelectFilter` form is the one-shot
//! `PROPERTY VALUE` line. The structured
//! [`crate::SelectionToolPayload::QSelect`]/`Filter` form carries the rest of the
//! dialog without a dialog:
//!
//! * operators `Equals`, `Not Equal To`, `Greater Than`, `Less Than`,
//!   `Greater Than Or Equal`, `Less Than Or Equal`, `*Wildcard Match*` and
//!   `Select All`;
//! * scope **entire drawing** or **current selection**;
//! * apply mode **include in new set**, **exclude from new set** or **append**;
//! * the **shared modelled property set** (object type, layer, colour, linetype,
//!   lineweight, linetype scale, transparency, space, text content and block
//!   name). AutoCAD does **not** document FILTER and QSELECT property sets as
//!   identical, so this is a declared common subset, not a parity claim.
//!
//! FILTER expressions are typed logical trees: `AND`/`OR` (≥ 1 member), `XOR`
//! (exactly 2) and `NOT` (exactly 1), with balanced Begin/End groups represented
//! by [`FilterExpression::Group`]. The arity rules are validated before any
//! match; an invalid tree is a named refusal and changes nothing. Named filters
//! (`filter.nfl`) are host file I/O: this build has no host bridge and refuses
//! them by name (`filter.named_unsupported`).

use crate::command_payload::{
    validate_expression, validate_predicate, FilterExpression, FilterLogic, SelectionApply,
    SelectionOperator, SelectionPredicate, SelectionProperty, SelectionScope,
};
use crate::property_value::property_value;
use crate::{
    Application, Command, CommandId, CommandOutcome, CommandPayload, SelectionMode, SelectionRef,
    SelectionSet, SelectionToolPayload, SessionState,
};
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{
    CadError, CadResult, Diagnostic, DocumentId, EntityId, InstancePath, LayerId, SemanticGeometry,
    SpaceId,
};

/// Whether `id` belongs to this batch (the dispatch arm in `Application::execute`).
pub(crate) fn batch(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::QSelect
            | CommandId::FilterSelect
            | CommandId::GetSelection
            | CommandId::SelectMatchingProperties
            | CommandId::SelectCount
            | CommandId::FindReport
            | CommandId::FindReplace
            | CommandId::HideObjects
            | CommandId::IsolateObjects
            | CommandId::UnisolateObjects
    )
}

impl Application {
    /// Dispatch one command of this batch to its own handler.
    ///
    /// The caller has already matched the id with [`batch`], so the final arm is
    /// an invariant refusal rather than a fallback that silently succeeds.
    pub(crate) fn dispatch_batch(
        &mut self,
        session: &mut SessionState,
        id: CommandId,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match id {
            CommandId::QSelect | CommandId::FilterSelect | CommandId::GetSelection => {
                self.select_filter(session, command)
            }
            CommandId::SelectMatchingProperties => {
                self.select_matching_properties(session, command)
            }
            CommandId::SelectCount => self.select_count(session, command),
            CommandId::HideObjects => self.hide_objects(session, command),
            CommandId::IsolateObjects => self.isolate_objects(session, command),
            CommandId::UnisolateObjects => self.unisolate_objects(session, command),
            CommandId::FindReport => self.find_report(command),
            CommandId::FindReplace => self.find_replace(command),
            other => Err(CadError::Invariant(format!(
                "{other:?} is not part of the select/filter batch"
            ))),
        }
    }

    /// `QSELECT` / `FILTER` / `GETSEL`: declare a selection query.
    ///
    /// Two payload forms are accepted:
    ///
    /// * the legacy [`CommandPayload::SelectFilter`] `PROPERTY VALUE` one-shot
    ///   line (GETSEL supports only this form), and
    /// * the structured [`SelectionToolPayload::QSelect`] /
    ///   [`SelectionToolPayload::Filter`] form that carries the dialog
    ///   semantics (operators, scope, apply mode and a FILTER expression tree).
    ///
    /// Both forms run the same read-only engine: only
    /// `SessionState::selection` changes, no transaction/revision/history entry
    /// is opened.
    pub(crate) fn select_filter(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::SelectFilter {
                property,
                value,
                mode,
            } => {
                if is_named_filter_query(property) {
                    return Err(named_filter_refusal(value));
                }
                let apply = legacy_apply(command.id, *mode)?;
                let predicate = legacy_predicate(command.id, property, value)?;
                self.run_selection_query(
                    session,
                    command,
                    SelectionQuery::Predicate {
                        object_type: None,
                        predicate,
                    },
                    SelectionScope::EntireDrawing,
                    apply,
                )
            }
            CommandPayload::SelectionTool(payload) => match payload.as_ref() {
                SelectionToolPayload::QSelect(request) if command.id == CommandId::QSelect => {
                    validate_predicate(prefix(command.id), &request.predicate)?;
                    if let Some(object_type) = &request.object_type {
                        if object_type.trim().is_empty() {
                            return Err(CadError::InvalidInput(format!(
                                "{}.empty_object_type: the object-type restriction must not be \
                                 empty",
                                prefix(command.id)
                            )));
                        }
                    }
                    self.run_selection_query(
                        session,
                        command,
                        SelectionQuery::Predicate {
                            object_type: request.object_type.clone(),
                            predicate: request.predicate.clone(),
                        },
                        request.scope,
                        request.apply,
                    )
                }
                SelectionToolPayload::Filter(request) if command.id == CommandId::FilterSelect => {
                    validate_expression(prefix(command.id), &request.expression)?;
                    self.run_selection_query(
                        session,
                        command,
                        SelectionQuery::Expression(request.expression.clone()),
                        request.scope,
                        request.apply,
                    )
                }
                _ => Err(CadError::InvalidInput(format!(
                    "{}.invalid_payload: {} accepts only its own query payload",
                    prefix(command.id),
                    command_name(command.id)
                ))),
            },
            _ => Err(CadError::InvalidInput(format!(
                "{} needs a SelectFilter {{ property, value, mode }} payload or its \
                 structured query payload",
                command_name(command.id)
            ))),
        }
    }

    /// Run one read-only selection query over the current space (or the current
    /// selection) and report exactly what changed.
    fn run_selection_query(
        &self,
        session: &mut SessionState,
        command: &Command,
        query: SelectionQuery,
        scope: SelectionScope,
        apply: SelectionApply,
    ) -> CadResult<CommandOutcome> {
        let document = self.document(command)?;
        let database = &document.drawing;
        let candidates = query_candidates(database, session, command.document, scope)?;
        let evaluated: Vec<(SelectionRef, bool)> = candidates
            .iter()
            .map(|candidate| {
                (
                    model_ref(command.document, candidate.id),
                    query.matches(database, candidate),
                )
            })
            .collect();
        let matched_count = evaluated.iter().filter(|(_, hit)| *hit).count();
        let reported = match apply {
            SelectionApply::IncludeNew => {
                let mut set = SelectionSet::new();
                for (reference, hit) in &evaluated {
                    if *hit {
                        set.insert(reference.clone());
                    }
                }
                let count = set.len();
                session.selection = set;
                if count == 0 {
                    no_match_message(command.id, &session.active_space)
                } else {
                    format!(
                        "{}: replaced the selection with {count} matching object(s)",
                        command_name(command.id)
                    )
                }
            }
            SelectionApply::ExcludeNew => {
                let mut set = SelectionSet::new();
                for (reference, hit) in &evaluated {
                    if !*hit {
                        set.insert(reference.clone());
                    }
                }
                let count = set.len();
                session.selection = set;
                if matched_count == 0 {
                    no_match_message(command.id, &session.active_space)
                } else {
                    format!(
                        "{}: excluded {matched_count} matching object(s); the new selection has \
                         {count} object(s)",
                        command_name(command.id)
                    )
                }
            }
            SelectionApply::Append => {
                let before = session.selection.len();
                for (reference, hit) in &evaluated {
                    if *hit {
                        session.selection.insert(reference.clone());
                    }
                }
                let added = session.selection.len() - before;
                if added == 0 && matched_count == 0 {
                    no_match_message(command.id, &session.active_space)
                } else {
                    format!(
                        "{}: added {added} matching object(s) to the selection",
                        command_name(command.id)
                    )
                }
            }
        };
        session.begin_selecting();
        Ok(selection_outcome(
            format!("{}.result", prefix(command.id)),
            reported,
        ))
    }

    /// `SELECTCOUNT`: report how many model-space entities match the first
    /// selection's five properties. Read-only: the selection is untouched.
    /// (`SSX` itself lives in [`crate::ssx`]; it is a separate Express-Tool
    /// criteria builder and is **not** this template-signature count.)
    pub(crate) fn select_count(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command)?;
        let document = self.document(command)?;
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "selectcount.model_space_only: SELECTCOUNT currently supports model space only"
                    .into(),
            ));
        }
        let template = template_signature(&document.drawing, session, "selectcount")?;
        let database = &document.drawing;
        let count = database
            .model_space()
            .into_iter()
            .filter(|entity| template.matches(database, entity))
            .count();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "selectcount.result".into(),
                message: format!("SELECTCOUNT: {count} model-space object(s) match the selection"),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `HIDEOBJECTS`: hide the current selection (session only) and clear it.
    pub(crate) fn hide_objects(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command)?;
        let _ = self.document(command)?;
        if session.selection.is_empty() {
            return Err(CadError::InvalidInput(
                "hideobjects.selection_required: select at least one object to hide".into(),
            ));
        }
        let hidden: Vec<SelectionRef> = session.selection.refs().to_vec();
        let count = hidden.len();
        for reference in hidden {
            session.hidden_entities.insert(reference);
        }
        session.selection.clear();
        session.begin_selecting();
        Ok(selection_outcome(
            "hideobjects.result".into(),
            format!("HIDEOBJECTS: hid {count} object(s) for this session"),
        ))
    }

    /// `ISOLATEOBJECTS`: hide every model-space entity except the selection.
    pub(crate) fn isolate_objects(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command)?;
        let document = self.document(command)?;
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "isolateobjects.model_space_only: ISOLATEOBJECTS currently supports model space \
                 only"
                    .into(),
            ));
        }
        if session.selection.is_empty() {
            return Err(CadError::InvalidInput(
                "isolateobjects.selection_required: select the objects to keep first".into(),
            ));
        }
        let database = &document.drawing;
        let mut hidden = SelectionSet::new();
        for entity in database.model_space() {
            let reference = model_ref(command.document, entity.id);
            if !session.selection.contains(&reference) {
                hidden.insert(reference);
            }
        }
        let count = hidden.len();
        session.hidden_entities = hidden;
        session.begin_selecting();
        Ok(selection_outcome(
            "isolateobjects.result".into(),
            format!("ISOLATEOBJECTS: hid {count} other object(s) for this session"),
        ))
    }

    /// `UNISOLATEOBJECTS`: clear the session hidden set.
    pub(crate) fn unisolate_objects(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command)?;
        let _ = self.document(command)?;
        let cleared = session.hidden_entities.len();
        session.hidden_entities.clear();
        session.begin_selecting();
        Ok(selection_outcome(
            "unisolateobjects.result".into(),
            format!("UNISOLATEOBJECTS: restored {cleared} hidden object(s)"),
        ))
    }

    fn document(&self, command: &Command) -> CadResult<&crate::Document> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != command.document {
            return Err(CadError::StaleResult);
        }
        Ok(document)
    }
}

/// One selection query: either one predicate (QSELECT's one-row form) or a
/// FILTER expression tree.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectionQuery {
    Predicate {
        object_type: Option<String>,
        predicate: SelectionPredicate,
    },
    Expression(FilterExpression),
}

impl SelectionQuery {
    fn matches(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        match self {
            SelectionQuery::Predicate {
                object_type,
                predicate,
            } => {
                if let Some(object_type) = object_type {
                    if !type_matches(entity, object_type) {
                        return false;
                    }
                }
                predicate.matches(database, entity)
            }
            SelectionQuery::Expression(expression) => {
                expression_matches(expression, database, entity)
            }
        }
    }
}

fn expression_matches(
    expression: &FilterExpression,
    database: &DrawingDatabase,
    entity: &DbEntity,
) -> bool {
    match expression {
        FilterExpression::Predicate(predicate) => predicate.matches(database, entity),
        FilterExpression::Group { logic, members } => match logic {
            FilterLogic::And => members
                .iter()
                .all(|member| expression_matches(member, database, entity)),
            FilterLogic::Or => members
                .iter()
                .any(|member| expression_matches(member, database, entity)),
            // Arity is validated before matching; the `get` calls stay defensive.
            FilterLogic::Xor => match (members.first(), members.get(1)) {
                (Some(first), Some(second)) => {
                    expression_matches(first, database, entity)
                        != expression_matches(second, database, entity)
                }
                _ => false,
            },
            FilterLogic::Not => members
                .first()
                .is_some_and(|member| !expression_matches(member, database, entity)),
        },
    }
}

impl SelectionProperty {
    fn string_value(self, database: &DrawingDatabase, entity: &DbEntity) -> Option<String> {
        match self {
            Self::Type => Some(entity.object.type_key.clone()),
            Self::Layer => database.layer(entity.layer).map(|layer| layer.name.clone()),
            Self::Color | Self::Linetype | Self::Lineweight | Self::Transparency => {
                property_value(database, entity, key(self))
            }
            Self::LinetypeScale => Some(format!(
                "{}",
                cad_db::sanitize_linetype_scale(
                    database.entity_render_attributes(entity.id).linetype_scale
                )
            )),
            Self::Space => Some(space_name(database, &entity.space)),
            Self::Text => match &entity.geometry {
                SemanticGeometry::Text { text, .. } => Some(text.clone()),
                SemanticGeometry::MultiLeader(leader) => Some(leader.text().string.clone()),
                _ => None,
            },
            Self::Name => match &entity.geometry {
                SemanticGeometry::Insert { block, .. } => {
                    database.block_name(*block).map(str::to_string)
                }
                _ => None,
            },
        }
    }

    fn numeric_value(self, database: &DrawingDatabase, entity: &DbEntity) -> Option<f64> {
        match self {
            Self::Lineweight | Self::Transparency => self
                .string_value(database, entity)
                .and_then(|value| value.trim().parse::<f64>().ok()),
            Self::LinetypeScale => Some(cad_db::sanitize_linetype_scale(
                database.entity_render_attributes(entity.id).linetype_scale,
            )),
            _ => None,
        }
    }

    /// The property key `cad_app::property_value` uses, or `None`.
    fn property_key(self) -> Option<&'static str> {
        match self {
            Self::Color => Some("color"),
            Self::Linetype => Some("linetype"),
            Self::Lineweight => Some("lineweight"),
            Self::Transparency => Some("transparency"),
            _ => None,
        }
    }
}

fn key(property: SelectionProperty) -> &'static str {
    property.property_key().unwrap_or("")
}

impl SelectionPredicate {
    fn matches(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        if self.operator == SelectionOperator::SelectAll {
            return true;
        }
        if self.property == SelectionProperty::Type {
            return self.matches_type(entity);
        }
        let Some(candidate) = self.property.string_value(database, entity) else {
            return false;
        };
        self.compare(database, entity, &candidate)
    }

    fn matches_type(&self, entity: &DbEntity) -> bool {
        match self.operator {
            SelectionOperator::Equals => type_matches(entity, &self.value),
            SelectionOperator::NotEqual => !type_matches(entity, &self.value),
            SelectionOperator::WildcardMatch => type_names(entity)
                .iter()
                .any(|name| wildcard_match(&self.value, name)),
            // Relative operators are refused for the type property.
            _ => false,
        }
    }

    fn compare(&self, database: &DrawingDatabase, entity: &DbEntity, candidate: &str) -> bool {
        match self.operator {
            SelectionOperator::Equals | SelectionOperator::NotEqual => {
                let equal = if self.property.is_numeric() {
                    match (
                        self.property.numeric_value(database, entity),
                        self.value.trim().parse::<f64>(),
                    ) {
                        (Some(left), Ok(right)) => (left - right).abs() <= f64::EPSILON * 16.0,
                        _ => candidate.eq_ignore_ascii_case(&self.value),
                    }
                } else {
                    candidate.eq_ignore_ascii_case(&self.value)
                };
                if self.operator == SelectionOperator::Equals {
                    equal
                } else {
                    !equal
                }
            }
            SelectionOperator::WildcardMatch => wildcard_match(&self.value, candidate),
            SelectionOperator::SelectAll => true,
            relative => {
                let Ok(rhs) = self.value.trim().parse::<f64>() else {
                    return false;
                };
                let Some(lhs) = self.property.numeric_value(database, entity) else {
                    return false;
                };
                match relative {
                    SelectionOperator::GreaterThan => lhs > rhs,
                    SelectionOperator::LessThan => lhs < rhs,
                    SelectionOperator::GreaterThanOrEqual => lhs >= rhs,
                    SelectionOperator::LessThanOrEqual => lhs <= rhs,
                    _ => false,
                }
            }
        }
    }
}

/// Parse one legacy `PROPERTY VALUE` line into the shared predicate.
fn legacy_predicate(id: CommandId, property: &str, value: &str) -> CadResult<SelectionPredicate> {
    let prefix = prefix(id);
    if value.trim().is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.empty_value: the filter value must not be empty"
        )));
    }
    let Some(property) = SelectionProperty::parse(property) else {
        let accepted = if id == CommandId::GetSelection {
            "LAYER or TYPE"
        } else {
            "TYPE, LAYER, COLOR, LINETYPE, LINEWEIGHT, LINETYPESCALE, TRANSPARENCY, SPACE, \
             TEXT or NAME"
        };
        return Err(CadError::InvalidInput(format!(
            "{prefix}.unknown_property: '{property}' is not a filter property ({accepted})"
        )));
    };
    if id == CommandId::GetSelection
        && !matches!(property, SelectionProperty::Layer | SelectionProperty::Type)
    {
        return Err(CadError::InvalidInput(
            "getsel.unknown_property: GETSEL accepts LAYER or TYPE only".into(),
        ));
    }
    Ok(SelectionPredicate {
        property,
        operator: SelectionOperator::Equals,
        value: value.trim().to_string(),
    })
}

/// Map the legacy `SelectionMode` onto the dialog apply modes.
fn legacy_apply(id: CommandId, mode: SelectionMode) -> CadResult<SelectionApply> {
    let apply = match (id, mode) {
        (CommandId::QSelect, SelectionMode::Replace) => SelectionApply::IncludeNew,
        (CommandId::QSelect, SelectionMode::Add) => SelectionApply::Append,
        (CommandId::QSelect, SelectionMode::Remove) => SelectionApply::ExcludeNew,
        (CommandId::GetSelection, SelectionMode::Replace) => SelectionApply::IncludeNew,
        (CommandId::FilterSelect, SelectionMode::Add) => SelectionApply::Append,
        _ => {
            return Err(CadError::InvalidInput(format!(
                "{}.invalid_mode: {mode:?} is not an apply mode for {}",
                prefix(id),
                command_name(id)
            )))
        }
    };
    Ok(apply)
}

/// The candidate entities of a query. Only **top-level** refs are ever
/// candidates: a block-instance member (non-empty instance path) or a
/// sub-element is not selected or counted by QSELECT/FILTER.
fn query_candidates<'a>(
    database: &'a DrawingDatabase,
    session: &SessionState,
    document: DocumentId,
    scope: SelectionScope,
) -> CadResult<Vec<&'a DbEntity>> {
    match scope {
        SelectionScope::EntireDrawing => match &session.active_space {
            SpaceId::Model => Ok(database.model_space()),
            SpaceId::Paper(layout) => Ok(database.layout_entities(*layout)),
            SpaceId::Block(_) => Err(CadError::Unsupported(
                "select_filter.block_space: QSELECT/FILTER run in model space or a paper \
                 layout, not inside a block definition"
                    .into(),
            )),
        },
        SelectionScope::CurrentSelection => {
            let mut candidates = Vec::new();
            for reference in session.selection.refs() {
                if reference.document != document
                    || !reference.instance.0.is_empty()
                    || reference.sub_element.is_some()
                {
                    continue;
                }
                let Some(entity) = database.entity(reference.entity) else {
                    continue;
                };
                if entity.space == session.active_space {
                    candidates.push(entity);
                }
            }
            Ok(candidates)
        }
    }
}

/// Whether a legacy property word asks for a **named filter** (`.nfl`).
///
/// Named filters are host file I/O (the AutoCAD `filter.nfl` store) and this
/// build has no host bridge for it, so it is a stable named refusal rather than
/// a silently empty selection.
fn is_named_filter_query(property: &str) -> bool {
    property.trim().eq_ignore_ascii_case("NAMED")
        || property.trim().eq_ignore_ascii_case("NFILTER")
        || property.trim().eq_ignore_ascii_case("FILTER.NFL")
}

/// The named-filter refusal: `.nfl` loading/saving belongs to the host, not the
/// command layer.
fn named_filter_refusal(name: &str) -> CadError {
    let name = name.trim();
    let detail = if name.is_empty() {
        "no filter name given".to_string()
    } else {
        format!("'{name}'")
    };
    CadError::Unsupported(format!(
        "filter.named_unsupported: named filter {detail} is host file I/O (filter.nfl); this \
         build has no host bridge for it"
    ))
}

fn no_match_message(id: CommandId, space: &SpaceId) -> String {
    let scope = match space {
        SpaceId::Model => "model-space",
        SpaceId::Paper(_) => "paper-space",
        SpaceId::Block(_) => "block-space",
    };
    format!("{}: no {scope} entity matches the filter", command_name(id))
}

fn type_names(entity: &DbEntity) -> Vec<String> {
    let mut names = vec![entity.object.type_key.clone()];
    match &entity.geometry {
        SemanticGeometry::Text { .. } => {
            if entity.object.type_key.eq_ignore_ascii_case("AcDbMText") {
                names.push("MTEXT".into());
            } else {
                names.push("TEXT".into());
            }
        }
        SemanticGeometry::Mask { .. } => names.push("WIPEOUT".into()),
        other => names.push(geometry_key(other).to_string()),
    }
    names
}

fn space_name(database: &DrawingDatabase, space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "Model".to_string(),
        SpaceId::Paper(layout) => database
            .layout(*layout)
            .map(|layout| layout.name.clone())
            .unwrap_or_else(|| "Paper".to_string()),
        SpaceId::Block(block) => format!("Block:{}", block.0),
    }
}

/// Anchored, ASCII case-insensitive `*`/`?` wildcard match.
///
/// This is the `*Wildcard Match*` operator's core grammar. The other AutoCAD
/// wildcard tokens (`#`, `@`, `.`, `[...]`, `~`) are **not** implemented and are
/// literal characters here; that boundary is documented rather than silently
/// guessed.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // Iterative backtracking: `star` remembers the last `*` and `mark` the text
    // position it may still extend.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

/// The five-property signature `SSX`/`SELECTCOUNT` compare.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EntitySignature {
    type_key: String,
    kind: &'static str,
    layer: LayerId,
    color: Option<String>,
    linetype: Option<String>,
    space: SpaceId,
}

impl EntitySignature {
    fn of(database: &DrawingDatabase, entity: &DbEntity) -> Self {
        Self {
            type_key: entity.object.type_key.clone(),
            kind: geometry_key(&entity.geometry),
            layer: entity.layer,
            color: property_value(database, entity, "color"),
            linetype: property_value(database, entity, "linetype"),
            space: entity.space.clone(),
        }
    }

    fn matches(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        Self::of(database, entity) == *self
    }
}

/// Resolve the first selection into the template signature, with the named
/// refusals every caller shares.
fn template_signature(
    database: &DrawingDatabase,
    session: &SessionState,
    prefix: &str,
) -> CadResult<EntitySignature> {
    let Some(reference) = session.selection.refs().first() else {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.selection_required: select at least one object first"
        )));
    };
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(CadError::Unsupported(format!(
            "{prefix}.template_not_addressable: the first selected object is a block-instance \
             member or sub-element and has no single entity row to read"
        )));
    }
    let entity = database.entity(reference.entity).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "{prefix}.template_missing: entity {} no longer exists",
            reference.entity.0
        ))
    })?;
    Ok(EntitySignature::of(database, entity))
}

fn model_ref(document: DocumentId, entity: EntityId) -> SelectionRef {
    SelectionRef {
        document,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn require_none_payload(command: &Command) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "{} takes no payload",
            command_name(command.id)
        )))
    }
}

fn prefix(id: CommandId) -> &'static str {
    match id {
        CommandId::QSelect => "qselect",
        CommandId::FilterSelect => "filter",
        CommandId::GetSelection => "getsel",
        CommandId::SelectMatchingProperties => "ssx",
        CommandId::SelectCount => "selectcount",
        CommandId::HideObjects => "hideobjects",
        CommandId::IsolateObjects => "isolateobjects",
        _ => "selectfilter",
    }
}

fn command_name(id: CommandId) -> &'static str {
    match id {
        CommandId::QSelect => "QSELECT",
        CommandId::FilterSelect => "FILTER",
        CommandId::GetSelection => "GETSEL",
        CommandId::SelectMatchingProperties => "SSX",
        CommandId::SelectCount => "SELECTCOUNT",
        CommandId::HideObjects => "HIDEOBJECTS",
        CommandId::IsolateObjects => "ISOLATEOBJECTS",
        CommandId::UnisolateObjects => "UNISOLATEOBJECTS",
        _ => "select filter",
    }
}

fn selection_outcome(code: String, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code,
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

/// Whether an entity's stored type key **or** friendly geometry alias matches
/// `value` (ASCII case-insensitive).
///
/// `MTEXT` is a friendly alias for a `Text` geometry stamped `AcDbMText`; the
/// other aliases come from the geometry discriminant. A `Mask` matches both
/// `MASK` and its AutoCAD name `WIPEOUT`.
pub fn type_matches(entity: &DbEntity, value: &str) -> bool {
    if entity.object.type_key.eq_ignore_ascii_case(value) {
        return true;
    }
    match &entity.geometry {
        // TEXT and MTEXT are the same geometry variant; the stored type key is
        // the only distinction, so each alias names exactly one source class.
        SemanticGeometry::Text { .. } => {
            if entity.object.type_key.eq_ignore_ascii_case("AcDbMText") {
                value.eq_ignore_ascii_case("MTEXT")
            } else {
                value.eq_ignore_ascii_case("TEXT")
            }
        }
        SemanticGeometry::Mask { .. } => {
            value.eq_ignore_ascii_case("MASK") || value.eq_ignore_ascii_case("WIPEOUT")
        }
        other => geometry_key(other).eq_ignore_ascii_case(value),
    }
}

/// The stable friendly name of one geometry discriminant (`LINE`, `CIRCLE`, …).
pub fn geometry_key(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Polyline { .. } => "POLYLINE",
        SemanticGeometry::Circle { .. } => "CIRCLE",
        SemanticGeometry::Arc { .. } => "ARC",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Point(_) => "POINT",
        SemanticGeometry::Mesh(_) => "MESH",
        SemanticGeometry::Insert { .. } => "INSERT",
        SemanticGeometry::Text { .. } => "TEXT",
        SemanticGeometry::Shape { .. } => "SHAPE",
        SemanticGeometry::Opaque { .. } => "OPAQUE",
        SemanticGeometry::Compound(_) => "COMPOUND",
        SemanticGeometry::Dimension(_) => "DIMENSION",
        SemanticGeometry::Hatch(_) => "HATCH",
        SemanticGeometry::MultiLeader(_) => "MULTILEADER",
        SemanticGeometry::Image { .. } => "IMAGE",
        SemanticGeometry::Mask { .. } => "MASK",
    }
}

/// Whether a ref is in the session hidden set (exact entity + instance identity;
/// the sub-element is ignored, matching the renderer's entity-level flag).
pub fn ref_hidden(hidden: &SelectionSet, reference: &SelectionRef) -> bool {
    hidden.refs().iter().any(|candidate| {
        candidate.entity == reference.entity && candidate.instance == reference.instance
    })
}
