//! The 2026-11-07 selection/filter/visibility batch (`QSELECT`, `FILTER`,
//! `GETSEL`, `SSX`, `SELECTCOUNT`, `HIDEOBJECTS`, `ISOLATEOBJECTS`,
//! `UNISOLATEOBJECTS`).
//!
//! Every command here is a **selection or session** operation: it mutates
//! `SessionState::selection` / `SessionState::hidden_entities` and calls
//! [`SessionState::begin_selecting`] when it changes the selection. It never
//! opens a transaction, bumps a drawing revision, writes `cad-history` or
//! produces a `ChangeSet`. The full per-command contract is in
//! [`crate::command_id::select_filter`]; this module is its implementation.
//!
//! The filter candidate set is every **model-space entity** in the drawing. A
//! filter is a data query, not a screen query, so layer off/frozen state does
//! not exclude an entity from a `QSELECT` result (a hidden layer can hide it
//! from the canvas, not from the data). Zero matches is a named diagnostic, not
//! an error.

use crate::property_value::property_value;
use crate::{
    Application, Command, CommandId, CommandOutcome, CommandPayload, SelectionMode, SelectionRef,
    SelectionSet, SessionState,
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

    /// `QSELECT` / `FILTER` / `GETSEL`: declare a property filter.
    pub(crate) fn select_filter(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::SelectFilter {
            property,
            value,
            mode,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(format!(
                "{} needs a SelectFilter {{ property, value, mode }} payload",
                command_name(command.id)
            )));
        };
        if !matches!(mode, SelectionMode::Replace | SelectionMode::Add) {
            return Err(CadError::InvalidInput(format!(
                "{}.invalid_mode: a filter is a set query; only Replace and Add are meaningful",
                prefix(command.id)
            )));
        }
        // The command identity is the merge-mode authority: QSELECT/GETSEL replace,
        // FILTER adds. A payload that disagrees is refused instead of silently
        // performing the other command's merge.
        let expected = if command.id == CommandId::FilterSelect {
            SelectionMode::Add
        } else {
            SelectionMode::Replace
        };
        if *mode != expected {
            return Err(CadError::InvalidInput(format!(
                "{}.invalid_mode: this selector needs {:?}, not {:?}",
                prefix(command.id),
                expected,
                mode
            )));
        }
        let document = self.document(command)?;
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(format!(
                "{}.model_space_only: this command currently supports model space only",
                prefix(command.id)
            )));
        }
        let filter = PropertyFilter::parse(command.id, property, value)?;
        let database = &document.drawing;

        let mut matched = Vec::new();
        for entity in database.model_space() {
            if filter.matches(database, entity) {
                matched.push(model_ref(command.document, entity.id));
            }
        }

        let code = format!("{}.result", prefix(command.id));
        let reported = match mode {
            SelectionMode::Replace => {
                session.selection.replace(matched);
                let count = session.selection.len();
                if count == 0 {
                    format!(
                        "{}: no model-space entity matches the filter",
                        command_name(command.id)
                    )
                } else {
                    format!(
                        "{}: replaced the selection with {count} matching object(s)",
                        command_name(command.id)
                    )
                }
            }
            _ => {
                let before = session.selection.len();
                for reference in matched {
                    session.selection.insert(reference);
                }
                let added = session.selection.len() - before;
                if added == 0 {
                    format!(
                        "{}: no model-space entity matches the filter",
                        command_name(command.id)
                    )
                } else {
                    format!(
                        "{}: added {added} matching object(s) to the selection",
                        command_name(command.id)
                    )
                }
            }
        };
        session.begin_selecting();
        Ok(selection_outcome(code, reported))
    }

    /// `SSX`: add every model-space entity matching the first selection's five
    /// properties (type key + geometry kind, layer, colour, linetype, space).
    pub(crate) fn select_matching_properties(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command)?;
        let document = self.document(command)?;
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "ssx.model_space_only: SSX currently supports model space only".into(),
            ));
        }
        let template = template_signature(&document.drawing, session, "ssx")?;
        let database = &document.drawing;
        let before = session.selection.len();
        for entity in database.model_space() {
            if template.matches(database, entity) {
                session
                    .selection
                    .insert(model_ref(command.document, entity.id));
            }
        }
        let added = session.selection.len() - before;
        session.begin_selecting();
        Ok(selection_outcome(
            "ssx.result".into(),
            format!("SSX: added {added} matching object(s) to the selection"),
        ))
    }

    /// `SELECTCOUNT`: report how many model-space entities match the first
    /// selection's five properties. Read-only: the selection is untouched.
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

/// One parsed declarative filter (`PROPERTY VALUE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterProperty {
    Type,
    Layer,
    Color,
    Linetype,
}

impl FilterProperty {
    /// Parse the typed property word, ASCII case-insensitive.
    pub fn parse(text: &str) -> Option<Self> {
        if text.eq_ignore_ascii_case("TYPE") {
            Some(Self::Type)
        } else if text.eq_ignore_ascii_case("LAYER") {
            Some(Self::Layer)
        } else if text.eq_ignore_ascii_case("COLOR") {
            Some(Self::Color)
        } else if text.eq_ignore_ascii_case("LINETYPE") {
            Some(Self::Linetype)
        } else {
            None
        }
    }
}

/// A parsed filter, validated against the command that received it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PropertyFilter {
    property: FilterProperty,
    value: String,
}

impl PropertyFilter {
    fn parse(id: CommandId, property: &str, value: &str) -> CadResult<Self> {
        let prefix = prefix(id);
        if value.trim().is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.empty_value: the filter value must not be empty"
            )));
        }
        let Some(property) = FilterProperty::parse(property.trim()) else {
            let accepted = if id == CommandId::GetSelection {
                "LAYER or TYPE"
            } else {
                "TYPE, LAYER, COLOR or LINETYPE"
            };
            return Err(CadError::InvalidInput(format!(
                "{prefix}.unknown_property: '{property}' is not a filter property ({accepted})"
            )));
        };
        if id == CommandId::GetSelection
            && !matches!(property, FilterProperty::Layer | FilterProperty::Type)
        {
            return Err(CadError::InvalidInput(
                "getsel.unknown_property: GETSEL accepts LAYER or TYPE only".into(),
            ));
        }
        Ok(Self {
            property,
            value: value.trim().to_string(),
        })
    }

    fn matches(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        match self.property {
            FilterProperty::Type => type_matches(entity, &self.value),
            FilterProperty::Layer => database
                .layer(entity.layer)
                .is_some_and(|layer| layer.name.eq_ignore_ascii_case(&self.value)),
            FilterProperty::Color => property_value(database, entity, "color")
                .is_some_and(|value| value.eq_ignore_ascii_case(&self.value)),
            FilterProperty::Linetype => property_value(database, entity, "linetype")
                .is_some_and(|value| value.eq_ignore_ascii_case(&self.value)),
        }
    }
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
