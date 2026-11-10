//! `SSX` — the AutoCAD Express Tool selection-set builder.
//!
//! `SSX` is **not** `SELECTSIMILAR`: it builds an `ssget`-style filter list from
//! explicit criteria and returns the matching objects. This module owns the
//! typed criteria, the session-held filter being edited and the apply step. The
//! current-space matching engine lives here; the command contract is documented
//! in [`crate::command_id::select_filter`].
//!
//! ## Criteria
//!
//! The documented criteria are `Block`, `Color`, `Entity`, `Flag`, `LAyer`,
//! `LType`, `Style`, `Thickness` and `Vector`, plus the `Pick` template seed.
//! Each criterion holds **one value**: setting an existing criterion overwrites
//! it, and `Clear` removes it. The filter is **session state**
//! ([`SessionState::ssx_filter`]): it is not a drawing write, a revision bump or
//! an undo entry.
//!
//! Matching uses only **modelled** properties. `Flag` (the DXF entity-flags bit /
//! attributes-follow toggle), `Thickness` and `Vector` (extrusion) have no
//! `cad-db` field, so setting one is recorded but applying the filter is the
//! named `ssx.<criterion>_not_modelled` refusal, never a silently weaker match.
//! `Style` is the resolved text-style name of TEXT/MTEXT, a MULTILEADER's
//! annotation text style or a DIMENSION's text style.
//!
//! ## Result
//!
//! The match set **replaces** the session selection. AutoCAD places the old
//! selection in its `Previous` selection set; this build has no `Previous` set,
//! so the pre-command selection is only read by `Pick` (the template) and is
//! then replaced — the documented mapping of the two models.
//!
//! One `ssx.result` diagnostic reports the new selection size and one
//! `ssx.criteria` diagnostic echoes the active filter, so a caller can see what
//! an overwrite or a clear actually left active. A filter with no criterion is
//! refused (`ssx.empty_filter`) rather than defaulting to "select all".

use cad_db::DrawingDatabase;
use cad_domain::{
    CadError, CadResult, Diagnostic, DocumentId, EntityId, InstancePath, SelectionRef, SpaceId,
};

use crate::property_value::property_value;
use crate::select_filter::type_matches;
use crate::{
    Application, Command, CommandOutcome, CommandPayload, SelectionSet, SelectionToolPayload,
    SessionState,
};

/// One value of one `SSX` criterion.
#[derive(Debug, Clone, PartialEq)]
pub enum SsxCriterion {
    /// `SSX BLOCK <name>`: an INSERT's block table name.
    Block(String),
    /// `SSX COLOR <n>`: the canonical colour projection (`ByLayer`/`ByBlock`,
    /// `aci:<n>`, `#RRGGBB`); a bare ACI number is accepted as `aci:<n>`.
    Color(String),
    /// `SSX ENTITY <type>`: the stored type key or a friendly geometry alias.
    Entity(String),
    /// `SSX FLAG`: the DXF entity-flags / attributes-follow toggle. Recorded but
    /// not modelled by `cad-db`; applying it is a named refusal.
    Flag,
    /// `SSX LAYER <name>`: the stored layer name.
    Layer(String),
    /// `SSX LTYPE <name>`: the canonical linetype name.
    LType(String),
    /// `SSX STYLE <name>`: the resolved text style name.
    Style(String),
    /// `SSX THICKNESS <v>`: recorded but not modelled; applying is a named
    /// refusal.
    Thickness(f64),
    /// `SSX VECTOR <x> <y> <z>`: recorded but not modelled; applying is a named
    /// refusal.
    Vector(cad_domain::Point3),
}

/// The criterion a `Clear` removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsxCriterionKind {
    Block,
    Color,
    Entity,
    Flag,
    Layer,
    LType,
    Style,
    Thickness,
    Vector,
}

impl SsxCriterion {
    /// The criterion kind this value names.
    pub fn kind(&self) -> SsxCriterionKind {
        match self {
            SsxCriterion::Block(_) => SsxCriterionKind::Block,
            SsxCriterion::Color(_) => SsxCriterionKind::Color,
            SsxCriterion::Entity(_) => SsxCriterionKind::Entity,
            SsxCriterion::Flag => SsxCriterionKind::Flag,
            SsxCriterion::Layer(_) => SsxCriterionKind::Layer,
            SsxCriterion::LType(_) => SsxCriterionKind::LType,
            SsxCriterion::Style(_) => SsxCriterionKind::Style,
            SsxCriterion::Thickness(_) => SsxCriterionKind::Thickness,
            SsxCriterion::Vector(_) => SsxCriterionKind::Vector,
        }
    }
}

/// One `SSX` edit-and-apply request.
///
/// Every variant applies the filter after the edit, because the Express Tool
/// returns a selection set on each completion. `Pick` is also the bare-`SSX`
/// behaviour (payload `None`).
#[derive(Debug, Clone, PartialEq)]
pub enum SsxRequest {
    /// Add or overwrite one criterion, then apply.
    Set(SsxCriterion),
    /// Remove one criterion (a no-op when it was not set), then apply.
    Clear(SsxCriterionKind),
    /// Seed every supportable criterion from the current selection's first
    /// object (the Express Tool template-object path), then apply.
    Pick,
}

/// The session-held `SSX` filter. One `Option` per criterion.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SsxFilter {
    pub block: Option<String>,
    pub color: Option<String>,
    pub entity: Option<String>,
    pub flag: bool,
    pub layer: Option<String>,
    pub ltype: Option<String>,
    pub style: Option<String>,
    pub thickness: Option<f64>,
    pub vector: Option<cad_domain::Point3>,
}

impl SsxFilter {
    /// Set/overwrite one criterion.
    pub fn set(&mut self, criterion: SsxCriterion) {
        match criterion {
            SsxCriterion::Block(value) => self.block = Some(value),
            SsxCriterion::Color(value) => self.color = Some(value),
            SsxCriterion::Entity(value) => self.entity = Some(value),
            SsxCriterion::Flag => self.flag = true,
            SsxCriterion::Layer(value) => self.layer = Some(value),
            SsxCriterion::LType(value) => self.ltype = Some(value),
            SsxCriterion::Style(value) => self.style = Some(value),
            SsxCriterion::Thickness(value) => self.thickness = Some(value),
            SsxCriterion::Vector(value) => self.vector = Some(value),
        }
    }

    /// Remove one criterion; returns whether it was actually set.
    pub fn clear(&mut self, kind: SsxCriterionKind) -> bool {
        match kind {
            SsxCriterionKind::Block => self.block.take().is_some(),
            SsxCriterionKind::Color => self.color.take().is_some(),
            SsxCriterionKind::Entity => self.entity.take().is_some(),
            SsxCriterionKind::Flag => std::mem::replace(&mut self.flag, false),
            SsxCriterionKind::Layer => self.layer.take().is_some(),
            SsxCriterionKind::LType => self.ltype.take().is_some(),
            SsxCriterionKind::Style => self.style.take().is_some(),
            SsxCriterionKind::Thickness => self.thickness.take().is_some(),
            SsxCriterionKind::Vector => self.vector.take().is_some(),
        }
    }

    /// Whether no criterion is set.
    pub fn is_empty(&self) -> bool {
        self.block.is_none()
            && self.color.is_none()
            && self.entity.is_none()
            && !self.flag
            && self.layer.is_none()
            && self.ltype.is_none()
            && self.style.is_none()
            && self.thickness.is_none()
            && self.vector.is_none()
    }

    /// The stable echo of the active criteria (fixed criterion order).
    fn echo(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(value) = &self.block {
            parts.push(format!("block={value}"));
        }
        if let Some(value) = &self.color {
            parts.push(format!("color={value}"));
        }
        if let Some(value) = &self.entity {
            parts.push(format!("entity={value}"));
        }
        if self.flag {
            parts.push("flag".into());
        }
        if let Some(value) = &self.layer {
            parts.push(format!("layer={value}"));
        }
        if let Some(value) = &self.ltype {
            parts.push(format!("ltype={value}"));
        }
        if let Some(value) = &self.style {
            parts.push(format!("style={value}"));
        }
        if let Some(value) = &self.thickness {
            parts.push(format!("thickness={value}"));
        }
        if let Some(value) = &self.vector {
            parts.push(format!("vector={},{},{}", value.x, value.y, value.z));
        }
        if parts.is_empty() {
            "SSX filter: (empty)".to_string()
        } else {
            format!("SSX filter: {}", parts.join(" "))
        }
    }

    /// The unmodelled criteria, in the fixed refusal order.
    fn unmodelled_refusal(&self) -> Option<CadError> {
        if self.flag {
            return Some(CadError::Unsupported(
                "ssx.flag_not_modelled: the DXF entity-flags / attributes-follow bit has no \
                 cad-db field, so the SSX Flag criterion cannot be matched"
                    .into(),
            ));
        }
        if self.thickness.is_some() {
            return Some(CadError::Unsupported(
                "ssx.thickness_not_modelled: entities in this build have no thickness field, so \
                 the SSX Thickness criterion cannot be matched"
                    .into(),
            ));
        }
        if self.vector.is_some() {
            return Some(CadError::Unsupported(
                "ssx.vector_not_modelled: entities in this build have no extrusion vector field, \
                 so the SSX Vector criterion cannot be matched"
                    .into(),
            ));
        }
        None
    }
}

impl Application {
    /// `SSX`: edit the session filter and replace the selection with the
    /// current-space match set.
    ///
    /// Read-only with respect to the drawing: no transaction, revision bump,
    /// history entry or `ChangeSet`. Only `SessionState::ssx_filter` and
    /// `SessionState::selection` change. See the module docs for the criteria and
    /// the named refusals.
    pub(crate) fn select_matching_properties(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = ssx_request(command)?;
        let document = self.ssx_document(command)?;
        let database = &document.drawing;
        let mut diagnostics = Vec::new();
        match request {
            SsxRequest::Set(criterion) => {
                session.ssx_filter.set(criterion);
            }
            SsxRequest::Clear(kind) => {
                let existed = session.ssx_filter.clear(kind);
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "ssx.cleared".into(),
                    message: if existed {
                        format!("SSX: removed the {kind:?} criterion")
                    } else {
                        format!("SSX: the {kind:?} criterion was not set")
                    },
                });
            }
            SsxRequest::Pick => {
                session.ssx_filter = seed_filter(database, session, command.document)?;
            }
        }
        if let Some(refusal) = session.ssx_filter.unmodelled_refusal() {
            return Err(refusal);
        }
        if session.ssx_filter.is_empty() {
            return Err(CadError::InvalidInput(
                "ssx.empty_filter: at least one SSX criterion is required; an empty filter is \
                 refused instead of selecting every object"
                    .into(),
            ));
        }
        let entities = current_space_entities(database, &session.active_space)?;
        let mut matched = SelectionSet::new();
        let mut count = 0usize;
        for entity in entities {
            if filter_matches(database, entity, &session.ssx_filter) {
                matched.insert(model_ref(command.document, entity.id));
                count += 1;
            }
        }
        session.selection = matched;
        session.begin_selecting();
        diagnostics.push(Diagnostic {
            object: None,
            code: "ssx.result".into(),
            message: format!(
                "SSX: replaced the selection with {count} matching object(s) in {}",
                space_label(&session.active_space)
            ),
        });
        diagnostics.push(Diagnostic {
            object: None,
            code: "ssx.criteria".into(),
            message: session.ssx_filter.echo(),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    fn ssx_document(&self, command: &Command) -> CadResult<&crate::Document> {
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

/// Read the typed request; `None` is the bare-`SSX` template seed + apply.
fn ssx_request(command: &Command) -> CadResult<SsxRequest> {
    match &command.payload {
        CommandPayload::None => Ok(SsxRequest::Pick),
        CommandPayload::SelectionTool(payload) => match payload.as_ref() {
            SelectionToolPayload::Ssx(request) => Ok(request.clone()),
            _ => Err(CadError::InvalidInput(
                "ssx.invalid_payload: SSX accepts only its own criteria payload".into(),
            )),
        },
        _ => Err(CadError::InvalidInput(
            "ssx.invalid_payload: SSX takes no payload or its typed criteria payload".into(),
        )),
    }
}

/// The top-level entities of the active space.
fn current_space_entities<'a>(
    database: &'a DrawingDatabase,
    space: &SpaceId,
) -> CadResult<Vec<&'a cad_db::DbEntity>> {
    match space {
        SpaceId::Model => Ok(database.model_space()),
        SpaceId::Paper(layout) => Ok(database.layout_entities(*layout)),
        SpaceId::Block(_) => Err(CadError::Unsupported(
            "ssx.block_space: SSX runs in model space or a paper layout, not inside a block \
             definition"
                .into(),
        )),
    }
}

/// Seed the filter from the first selected object (the Express Tool template).
fn seed_filter(
    database: &DrawingDatabase,
    session: &SessionState,
    document: DocumentId,
) -> CadResult<SsxFilter> {
    let Some(reference) = session.selection.refs().first() else {
        return Err(CadError::InvalidInput(
            "ssx.selection_required: select at least one object first".into(),
        ));
    };
    if reference.document != document {
        return Err(CadError::InvalidInput(
            "ssx.template_not_addressable: the template object belongs to another document".into(),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(CadError::Unsupported(
            "ssx.template_not_addressable: the first selected object is a block-instance member \
             or sub-element and has no single entity row to read"
                .into(),
        ));
    }
    let entity = database.entity(reference.entity).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "ssx.template_missing: entity {} no longer exists",
            reference.entity.0
        ))
    })?;
    if entity.space != session.active_space {
        return Err(CadError::InvalidInput(
            "ssx.template_space: the template object is not in the current space".into(),
        ));
    }
    let Some(layer) = database.layer(entity.layer).map(|layer| layer.name.clone()) else {
        return Err(CadError::InvalidInput(
            "ssx.template_unresolved: the template object's layer is not in the layer table".into(),
        ));
    };
    Ok(SsxFilter {
        block: block_name(database, entity),
        color: property_value(database, entity, "color"),
        entity: Some(entity.object.type_key.clone()),
        flag: false,
        layer: Some(layer),
        ltype: property_value(database, entity, "linetype"),
        style: style_name(database, entity),
        thickness: None,
        vector: None,
    })
}

/// Whether every set criterion matches one entity.
fn filter_matches(
    database: &DrawingDatabase,
    entity: &cad_db::DbEntity,
    filter: &SsxFilter,
) -> bool {
    if let Some(wanted) = &filter.block {
        if block_name(database, entity).as_deref() != Some(wanted.as_str()) {
            return false;
        }
    }
    if let Some(wanted) = &filter.color {
        if !color_matches(database, entity, wanted) {
            return false;
        }
    }
    if let Some(wanted) = &filter.entity {
        if !type_matches(entity, wanted) {
            return false;
        }
    }
    if let Some(wanted) = &filter.layer {
        let Some(actual) = database.layer(entity.layer) else {
            return false;
        };
        if !actual.name.eq_ignore_ascii_case(wanted) {
            return false;
        }
    }
    if let Some(wanted) = &filter.ltype {
        let Some(actual) = property_value(database, entity, "linetype") else {
            return false;
        };
        if !actual.eq_ignore_ascii_case(wanted) {
            return false;
        }
    }
    if let Some(wanted) = &filter.style {
        let Some(actual) = style_name(database, entity) else {
            return false;
        };
        if !actual.eq_ignore_ascii_case(wanted) {
            return false;
        }
    }
    // `flag`, `thickness` and `vector` are refused before matching.
    true
}

/// The canonical colour comparison; a bare ACI number (`1`) means `aci:1`.
fn color_matches(database: &DrawingDatabase, entity: &cad_db::DbEntity, wanted: &str) -> bool {
    let Some(actual) = property_value(database, entity, "color") else {
        return false;
    };
    let wanted = wanted.trim();
    if actual.eq_ignore_ascii_case(wanted) {
        return true;
    }
    let digits = wanted.strip_prefix('$').unwrap_or(wanted);
    let digits = digits
        .strip_prefix("aci:")
        .or_else(|| digits.strip_prefix("ACI:"))
        .unwrap_or(digits);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && actual.eq_ignore_ascii_case(&format!("aci:{digits}"))
}

/// An INSERT's block table name (the `Block` criterion).
fn block_name(database: &DrawingDatabase, entity: &cad_db::DbEntity) -> Option<String> {
    match &entity.geometry {
        cad_domain::SemanticGeometry::Insert { block, .. } => {
            database.block_name(*block).map(str::to_string)
        }
        _ => None,
    }
}

/// The style name an entity's annotation records, for `SSX STYLE`.
///
/// For TEXT/MULTILEADER/DIMENSION this is the resolved **text** style. A TABLE
/// does not store a text style on the entity: its cells carry per-cell styles
/// and the entity carries a **table** style name, so that name is returned
/// deliberately (matching `SSX STYLE <table style>`). The two name spaces are
/// not interchangeable; changing this selection behavior would be a separate
/// documented decision.
fn style_name<'a>(database: &'a DrawingDatabase, entity: &'a cad_db::DbEntity) -> Option<String> {
    match &entity.geometry {
        cad_domain::SemanticGeometry::Text { style, .. } => {
            database.style(*style).map(|style| style.name.clone())
        }
        cad_domain::SemanticGeometry::MultiLeader(leader) => {
            Some(leader.style().text_style_name.clone())
        }
        cad_domain::SemanticGeometry::Dimension(dimension) => {
            Some(dimension.style().text_style_name.clone())
        }
        // A table's annotation style name is the table style name it stores.
        cad_domain::SemanticGeometry::Table(table) => {
            (!table.style_name().is_empty()).then(|| table.style_name().to_string())
        }
        _ => None,
    }
}

fn model_ref(document: DocumentId, entity: EntityId) -> SelectionRef {
    SelectionRef {
        document,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn space_label(space: &SpaceId) -> &'static str {
    match space {
        SpaceId::Model => "model space",
        SpaceId::Paper(_) => "the active paper layout",
        SpaceId::Block(_) => "block space",
    }
}
