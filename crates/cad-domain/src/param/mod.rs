//! Document parameters, bindings and pure evaluation (ADR-0043).
//!
//! A *parameter* is a named, typed expression owned by the document
//! (`tables/parameters.json`). An *element* binds some of its type's closed
//! **slots** to expressions (`"params"` block on the element row). Nothing here
//! stores a current value: values are derived by [`evaluate_table`] every time
//! they are needed, exactly like `DimensionGeometry` synthesises its display
//! (ADR-0017) instead of caching it (D3/D7).
//!
//! This module is deliberately pure: no database, no GPU, no clock, no I/O. It
//! owns three things and nothing else:
//!
//! * the expression grammar and its **dimension** rules (length/number/angle,
//!   canonical `mm`/`rad`, no silent conversion);
//! * the closed slot registry ([`param_slots`]) — a slot is a field the
//!   evaluator knows how to rewrite, never an arbitrary user key (D3);
//! * evaluation: dependency-ordered values, cycle detection and the budgets
//!   from D5.
//!
//! Failure markers (`param.*`) are stable strings, like `ymodel.*`: callers
//! match on them, not on [`ParamError`] variants. Every rejection is explicit —
//! there is no "evaluate what we can" mode (D5/D6).
//!
//! Persistence lives in `cad-db` (the table row codec and the element block);
//! this module never names a JSON field beyond its own grammar.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::EntityId;

mod config;
mod expr;
mod slots;

pub use config::{
    resolve_active, ConfigStatus, ParamConfig, ParamConfigTable, PARAM_CONFIG_CYCLE,
    PARAM_CONFIG_OVERRIDE, PARAM_SUPPRESSED, PARAM_UNKNOWN_CONFIG,
};
use expr::unknown_reference;
pub use expr::{dimension, resolve, BinaryOp, Dimension, Expr, ParamConst, UnaryOp};
pub use slots::{apply_slot, geometry_type_key, param_slot, param_slots, ParamSlot};

// ---------------------------------------------------------------------------
// Stable markers (ADR-0043 D5/D6/D15).
// ---------------------------------------------------------------------------

/// Two rows (or a rename target) share one name.
pub const PARAM_DUPLICATE_NAME: &str = "param.duplicate_name";
/// A `ref` names a sequence that does not exist.
pub const PARAM_UNKNOWN_REFERENCE: &str = "param.unknown_reference";
/// The reference graph is not a DAG, or a self-reference was written.
pub const PARAM_CYCLE: &str = "param.cycle";
/// A length/number/angle was used in a position of the wrong dimension.
pub const PARAM_UNIT_MISMATCH: &str = "param.unit_mismatch";
/// A binding names a slot its type does not register.
pub const PARAM_UNKNOWN_SLOT: &str = "param.unknown_slot";
/// The element's type has no evaluator at all (P0 refuses instead of guessing).
pub const PARAM_NO_EVALUATOR: &str = "param.no_evaluator";
/// A table/expression/closure exceeded its declared budget; the whole commit is
/// refused, never partially evaluated.
pub const PARAM_BUDGET_EXCEEDED: &str = "param.budget_exceeded";
/// A row, expression or computed value is structurally invalid (empty name,
/// zero sequence, non-finite literal, division by zero, non-finite result).
pub const PARAM_INVALID_VALUE: &str = "param.invalid_value";
/// A parameter is legal but the derived geometry cannot be constructed
/// (degenerate radius/height). The parameters commit and the element keeps its
/// last-good geometry (D6 second tier).
pub const PARAM_GEOMETRY_FAILED: &str = "param.geometry_failed";
/// The write side refuses a mixed multi-selection (D9). Defined here so the
/// vocabulary has one home; produced by `cad-app`.
pub const PARAM_MIXED: &str = "param.mixed";
/// A downstream element whose input failed; it keeps last-good geometry (D6).
pub const PARAM_BLOCKED: &str = "param.blocked";
/// An opened document's stored geometry disagreed with the pure evaluation and
/// was recomputed (D7). A diagnostic, never a refusal: the document still opens.
pub const PARAM_RECOMPUTED: &str = "param.recomputed";
/// A delete/rename target is still referenced where the command cannot rewrite
/// it (e.g. deleting a parameter a binding or another expression still uses).
pub const PARAM_REFERENCED: &str = "param.referenced";
/// A property write named a parameter that does not exist in this document.
pub const PARAM_UNKNOWN_NAME: &str = "param.unknown_name";

// ---------------------------------------------------------------------------
// Budgets (ADR-0043 D5).
// ---------------------------------------------------------------------------

/// Maximum parameter rows a document may hold.
pub const PARAM_MAX_ROWS: usize = 4096;
/// Maximum nesting depth of one expression tree.
pub const PARAM_MAX_EXPR_DEPTH: usize = 32;
/// Maximum node count of one expression tree.
pub const PARAM_MAX_EXPR_NODES: usize = 256;
/// Maximum length of one chain of parameter references.
pub const PARAM_MAX_REF_CHAIN: usize = 256;
/// Maximum number of slots one element may bind.
pub const PARAM_MAX_BINDINGS: usize = 64;
/// Maximum number of elements one commit may re-evaluate.
pub const PARAM_MAX_AFFECTED: usize = 10_000;
/// Maximum length of a parameter name, in bytes.
pub const PARAM_MAX_NAME_BYTES: usize = 128;

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// A rejection carrying one stable `param.*` marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamError {
    code: &'static str,
    detail: String,
}

impl ParamError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// The stable marker, e.g. [`PARAM_CYCLE`].
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Human-readable context. Not part of the contract.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for ParamError {}

fn invalid(detail: impl Into<String>) -> ParamError {
    ParamError::new(PARAM_INVALID_VALUE, detail)
}

// ---------------------------------------------------------------------------
// Kind, unit, sequence.
// ---------------------------------------------------------------------------

/// The quantity class of a parameter or slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParamKind {
    /// Canonical unit: millimetres.
    Length,
    /// Dimensionless.
    Number,
    /// Canonical unit: radians.
    Angle,
}

impl ParamKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Number => "number",
            Self::Angle => "angle",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "length" => Some(Self::Length),
            "number" => Some(Self::Number),
            "angle" => Some(Self::Angle),
            _ => None,
        }
    }

    /// The canonical unit token, or `None` for a dimensionless kind.
    pub fn canonical_unit(self) -> Option<ParamUnit> {
        match self {
            Self::Length => Some(ParamUnit::Millimetre),
            Self::Number => None,
            Self::Angle => Some(ParamUnit::Radian),
        }
    }
}

/// The only units this format accepts. Values are canonical; no conversion is
/// ever performed (ADR-0043 §2 and D14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamUnit {
    Millimetre,
    Radian,
}

impl ParamUnit {
    pub fn token(self) -> &'static str {
        match self {
            Self::Millimetre => "mm",
            Self::Radian => "rad",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "mm" => Some(Self::Millimetre),
            "rad" => Some(Self::Radian),
            _ => None,
        }
    }
}

/// A parameter's document-local sequence. Not a `PersistentId` (D4/D12): the
/// table owns identity, and renames never change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamSeq(pub u64);

// ---------------------------------------------------------------------------
// Table.
// ---------------------------------------------------------------------------

/// One document parameter row.
///
/// `unit` is optional on the wire. When present it must be the kind's canonical
/// unit; the reader refuses anything else instead of converting.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamRow {
    pub seq: ParamSeq,
    pub name: String,
    pub kind: ParamKind,
    pub unit: Option<ParamUnit>,
    pub expr: Expr,
}

impl ParamRow {
    pub fn new(
        seq: ParamSeq,
        name: impl Into<String>,
        kind: ParamKind,
        expr: Expr,
    ) -> Result<Self, ParamError> {
        let row = Self {
            seq,
            name: name.into(),
            kind,
            unit: None,
            expr,
        };
        row.validate()?;
        Ok(row)
    }

    pub fn with_unit(mut self, unit: ParamUnit) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Structural validation, independent of the rest of the table.
    pub fn validate(&self) -> Result<(), ParamError> {
        if self.seq.0 == 0 {
            return Err(invalid("a parameter sequence must be at least 1"));
        }
        if self.name.trim().is_empty() {
            return Err(invalid(format!(
                "parameter {} has an empty name",
                self.seq.0
            )));
        }
        if self.name.len() > PARAM_MAX_NAME_BYTES {
            return Err(invalid(format!(
                "parameter {:?} is {} bytes, over the {PARAM_MAX_NAME_BYTES}-byte budget",
                self.name,
                self.name.len()
            )));
        }
        if let Some(unit) = self.unit {
            if Some(unit) != self.kind.canonical_unit() {
                return Err(ParamError::new(
                    PARAM_UNIT_MISMATCH,
                    format!(
                        "parameter {:?} is {} but declares unit {:?}; only {:?} is accepted",
                        self.name,
                        self.kind.token(),
                        unit.token(),
                        self.kind.canonical_unit().map(ParamUnit::token)
                    ),
                ));
            }
        }
        self.expr.validate_shape()
    }
}

/// The document's parameter rows, unique by sequence **and** by name.
///
/// Names are compared exactly (no case folding, no trimming): a rename is an
/// explicit command and two spellings are two names. The index is derived from
/// the rows, so it cannot drift.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamTable {
    rows: BTreeMap<ParamSeq, ParamRow>,
    names: BTreeMap<String, ParamSeq>,
}

impl ParamTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_rows(rows: impl IntoIterator<Item = ParamRow>) -> Result<Self, ParamError> {
        let mut table = Self::new();
        for row in rows {
            table.insert(row)?;
        }
        Ok(table)
    }

    /// Insert or replace the row at its sequence.
    ///
    /// Replacing the row at `seq` keeps its identity; the name may change as
    /// long as it is not already taken by another sequence.
    pub fn insert(&mut self, row: ParamRow) -> Result<(), ParamError> {
        row.validate()?;
        if let Some(previous) = self.rows.get(&row.seq) {
            if previous.name != row.name {
                if let Some(owner) = self.names.get(&row.name) {
                    return Err(ParamError::new(
                        PARAM_DUPLICATE_NAME,
                        format!(
                            "parameter name {:?} already belongs to sequence {} (rename of {})",
                            row.name, owner.0, row.seq.0
                        ),
                    ));
                }
                self.names.remove(&previous.name);
            }
        } else {
            if self.rows.len() >= PARAM_MAX_ROWS {
                return Err(ParamError::new(
                    PARAM_BUDGET_EXCEEDED,
                    format!("a document may hold at most {PARAM_MAX_ROWS} parameters"),
                ));
            }
            if let Some(owner) = self.names.get(&row.name) {
                return Err(ParamError::new(
                    PARAM_DUPLICATE_NAME,
                    format!(
                        "parameter name {:?} already belongs to sequence {}",
                        row.name, owner.0
                    ),
                ));
            }
        }
        self.names.insert(row.name.clone(), row.seq);
        self.rows.insert(row.seq, row);
        Ok(())
    }

    pub fn remove(&mut self, seq: ParamSeq) -> Option<ParamRow> {
        let row = self.rows.remove(&seq)?;
        self.names.remove(&row.name);
        Some(row)
    }

    pub fn get(&self, seq: ParamSeq) -> Option<&ParamRow> {
        self.rows.get(&seq)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&ParamRow> {
        self.names.get(name).and_then(|seq| self.rows.get(seq))
    }

    pub fn rows(&self) -> impl Iterator<Item = &ParamRow> {
        self.rows.values()
    }

    pub fn seqs(&self) -> impl Iterator<Item = ParamSeq> + '_ {
        self.rows.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Values.
// ---------------------------------------------------------------------------

/// One evaluated parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamValue {
    pub value: f64,
    pub kind: ParamKind,
}

/// Every parameter of one document, evaluated once.
///
/// Never persisted (D3): a value is a function of the rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamValues {
    entries: BTreeMap<ParamSeq, ParamValue>,
}

impl ParamValues {
    pub fn get(&self, seq: ParamSeq) -> Option<ParamValue> {
        self.entries.get(&seq).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ParamSeq, ParamValue)> + '_ {
        self.entries.iter().map(|(seq, value)| (*seq, *value))
    }
}

// ---------------------------------------------------------------------------
// Bindings.
// ---------------------------------------------------------------------------

/// The slot → expression map attached to one element row (D3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamBindings {
    entries: BTreeMap<String, Expr>,
}

impl ParamBindings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_entries(
        entries: impl IntoIterator<Item = (String, Expr)>,
    ) -> Result<Self, ParamError> {
        let mut bindings = Self::new();
        for (slot, expr) in entries {
            bindings.insert(slot, expr)?;
        }
        Ok(bindings)
    }

    pub fn insert(&mut self, slot: impl Into<String>, expr: Expr) -> Result<(), ParamError> {
        let slot = slot.into();
        if slot.trim().is_empty() {
            return Err(invalid("a parameter binding has an empty slot name"));
        }
        if slot.len() > PARAM_MAX_NAME_BYTES {
            return Err(invalid(format!(
                "binding slot {slot:?} is {} bytes, over the {PARAM_MAX_NAME_BYTES}-byte budget",
                slot.len()
            )));
        }
        expr.validate_shape()?;
        if !self.entries.contains_key(&slot) && self.entries.len() >= PARAM_MAX_BINDINGS {
            return Err(ParamError::new(
                PARAM_BUDGET_EXCEEDED,
                format!("an element may bind at most {PARAM_MAX_BINDINGS} slots"),
            ));
        }
        self.entries.insert(slot, expr);
        Ok(())
    }

    pub fn remove(&mut self, slot: &str) -> Option<Expr> {
        self.entries.remove(slot)
    }

    pub fn get(&self, slot: &str) -> Option<&Expr> {
        self.entries.get(slot)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &Expr)> {
        self.entries
            .iter()
            .map(|(slot, expr)| (slot.as_str(), expr))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolve every bound slot against evaluated parameters.
    ///
    /// Unknown slots and kind mismatches are static failures (D6 first tier):
    /// the caller must refuse the whole commit instead of committing a partial
    /// binding.
    pub fn resolve(
        &self,
        type_key: &str,
        values: &ParamValues,
    ) -> Result<BTreeMap<String, f64>, ParamError> {
        let mut resolved = BTreeMap::new();
        for (slot, expr) in self.entries() {
            let spec = param_slot(type_key, slot).ok_or_else(|| {
                ParamError::new(
                    PARAM_UNKNOWN_SLOT,
                    format!("{type_key} does not register the slot {slot:?}"),
                )
            })?;
            let value = resolve(expr, spec.kind, values)?;
            resolved.insert(slot.to_string(), value);
        }
        Ok(resolved)
    }
}

// Table evaluation.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Grey,
    Black,
}

/// Evaluate every parameter in dependency order (D5).
///
/// * A reference cycle is [`PARAM_CYCLE`] — including a row referencing itself.
/// * A dangling reference is [`PARAM_UNKNOWN_REFERENCE`].
/// * A reference chain deeper than [`PARAM_MAX_REF_CHAIN`] is a budget refusal
///   rather than a stack overflow.
///
/// The result is deterministic: the traversal starts from ascending sequences
/// and each row's expression is a pure function of the values it references.
pub fn evaluate_table(table: &ParamTable) -> Result<ParamValues, ParamError> {
    let mut values = ParamValues::default();
    let mut state: BTreeMap<ParamSeq, Visit> = BTreeMap::new();
    for seq in table.seqs() {
        visit(seq, table, &mut values, &mut state, 0)?;
    }
    Ok(values)
}

fn visit(
    seq: ParamSeq,
    table: &ParamTable,
    values: &mut ParamValues,
    state: &mut BTreeMap<ParamSeq, Visit>,
    depth: usize,
) -> Result<ParamValue, ParamError> {
    if let Some(Visit::Black) = state.get(&seq) {
        return values.get(seq).ok_or_else(|| unknown_reference(seq));
    }
    if let Some(Visit::Grey) = state.get(&seq) {
        return Err(ParamError::new(
            PARAM_CYCLE,
            format!("parameter sequence {} is part of a reference cycle", seq.0),
        ));
    }
    if depth > PARAM_MAX_REF_CHAIN {
        return Err(ParamError::new(
            PARAM_BUDGET_EXCEEDED,
            format!(
                "the reference chain exceeded {PARAM_MAX_REF_CHAIN} links at sequence {}",
                seq.0
            ),
        ));
    }
    let row = table
        .get(seq)
        .ok_or_else(|| unknown_reference(seq))?
        .clone();
    state.insert(seq, Visit::Grey);
    // Evaluate references first so `values` is complete for `resolve`.
    let mut refs = BTreeSet::new();
    row.expr.references(&mut refs);
    for referenced in refs {
        visit(referenced, table, values, state, depth + 1)?;
    }
    let value = resolve(&row.expr, row.kind, values)?;
    let evaluated = ParamValue {
        value,
        kind: row.kind,
    };
    values.entries.insert(seq, evaluated);
    state.insert(seq, Visit::Black);
    Ok(evaluated)
}

/// The parameters transitively affected by `changed` (including `changed`).
///
/// Used to bound a commit to the reference closure instead of re-evaluating the
/// document. Deterministic (ascending sequences).
pub fn affected_params(table: &ParamTable, changed: &BTreeSet<ParamSeq>) -> BTreeSet<ParamSeq> {
    let mut reverse: BTreeMap<ParamSeq, BTreeSet<ParamSeq>> = BTreeMap::new();
    for row in table.rows() {
        let mut refs = BTreeSet::new();
        row.expr.references(&mut refs);
        for referenced in refs {
            reverse.entry(referenced).or_default().insert(row.seq);
        }
    }
    let mut affected = changed.clone();
    let mut frontier: Vec<ParamSeq> = changed.iter().copied().collect();
    while let Some(seq) = frontier.pop() {
        if let Some(dependents) = reverse.get(&seq) {
            for dependent in dependents {
                if affected.insert(*dependent) {
                    frontier.push(*dependent);
                }
            }
        }
    }
    affected
}

/// The elements whose bindings reference any affected parameter.
///
/// `directly_changed` names elements whose binding block itself changed; they
/// are always included. Exceeding [`PARAM_MAX_AFFECTED`] is a refusal, never a
/// partial evaluation.
pub fn affected_entities(
    bindings: &BTreeMap<EntityId, ParamBindings>,
    affected: &BTreeSet<ParamSeq>,
    directly_changed: &BTreeSet<EntityId>,
) -> Result<BTreeSet<EntityId>, ParamError> {
    let mut entities: BTreeSet<EntityId> = directly_changed.clone();
    for (id, binding) in bindings {
        if entities.contains(id) {
            continue;
        }
        let mut refs = BTreeSet::new();
        for (_, expr) in binding.entries() {
            expr.references(&mut refs);
        }
        if refs.iter().any(|seq| affected.contains(seq)) {
            entities.insert(*id);
        }
        if entities.len() > PARAM_MAX_AFFECTED {
            return Err(ParamError::new(
                PARAM_BUDGET_EXCEEDED,
                format!("the parameter closure exceeds {PARAM_MAX_AFFECTED} elements"),
            ));
        }
    }
    Ok(entities)
}

#[cfg(test)]
mod tests;
