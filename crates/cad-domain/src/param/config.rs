//! Named parameter configurations: sparse overrides and suppression (ADR-0043
//! P1).
//!
//! A configuration is a **named overlay** on the document's base parameter
//! table, never a second authority:
//!
//! * `overrides` maps a base `seq` to a replacement expression (the row's name,
//!   kind and unit stay the base row's);
//! * `suppressed` removes base sequences from the effective table;
//! * `parent` names another configuration whose overrides/suppression are
//!   applied first (child wins).
//!
//! Resolving the active configuration produces a plain [`ParamTable`], so the
//! evaluator, the slot bindings and every caller keep using the same pure
//! functions. The **base** table remains the authoritative stored data; an
//! overlay is derived and is never written back.
//!
//! # Suppression semantics (declared, not inferred)
//!
//! A suppressed parameter is absent from the effective table. Therefore:
//!
//! * another parameter that references it makes the configuration invalid
//!   (`param.unknown_reference`) — a configuration cannot silently change an
//!   expression's meaning;
//! * an **element binding** that references it is not a configuration error: the
//!   element is blocked (`param.suppressed`) and keeps its last-good geometry,
//!   exactly like a geometry-construction failure (D6 second tier).
//!
//! This mirrors SolidWorks' "suppressed feature produces no new geometry"
//! without pretending the binding never existed.

use std::collections::{BTreeMap, BTreeSet};

use super::{evaluate_table, Expr, ParamError, ParamRow, ParamSeq, ParamTable};

/// Two configurations form a parent cycle.
pub const PARAM_CONFIG_CYCLE: &str = "param.config_cycle";
/// A configuration or parent name does not exist.
pub const PARAM_UNKNOWN_CONFIG: &str = "param.unknown_config";
/// The active configuration overrides or suppresses the parameter a value edit
/// targets; editing the base row would not change the derived value.
pub const PARAM_CONFIG_OVERRIDE: &str = "param.config_override";
/// An element binding references a suppressed parameter; the element keeps its
/// last-good geometry (second tier, like `param.blocked`).
pub const PARAM_SUPPRESSED: &str = "param.suppressed";

/// One named configuration overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamConfig {
    pub name: String,
    pub parent: Option<String>,
    pub overrides: BTreeMap<ParamSeq, Expr>,
    pub suppressed: BTreeSet<ParamSeq>,
}

impl ParamConfig {
    pub fn new(name: impl Into<String>) -> Result<Self, ParamError> {
        let name = name.into();
        validate_name(&name)?;
        Ok(Self {
            name,
            parent: None,
            overrides: BTreeMap::new(),
            suppressed: BTreeSet::new(),
        })
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Result<Self, ParamError> {
        let parent = parent.into();
        validate_name(&parent)?;
        self.parent = Some(parent);
        Ok(self)
    }

    pub fn set_override(&mut self, seq: ParamSeq, expr: Expr) -> Result<(), ParamError> {
        expr.validate_shape()?;
        self.overrides.insert(seq, expr);
        Ok(())
    }

    pub fn suppress(&mut self, seq: ParamSeq) {
        self.suppressed.insert(seq);
    }

    /// Structural validation against the base table (parent existence is a
    /// table-level check; this checks names and expression shapes).
    pub fn validate(&self, base: &ParamTable) -> Result<(), ParamError> {
        validate_name(&self.name)?;
        if let Some(parent) = &self.parent {
            validate_name(parent)?;
        }
        for (seq, expr) in &self.overrides {
            expr.validate_shape()?;
            if base.get(*seq).is_none() {
                return Err(ParamError::new(
                    super::PARAM_UNKNOWN_REFERENCE,
                    format!(
                        "configuration {:?} overrides parameter sequence {}, which does not exist",
                        self.name, seq.0
                    ),
                ));
            }
        }
        for seq in &self.suppressed {
            if base.get(*seq).is_none() {
                return Err(ParamError::new(
                    super::PARAM_UNKNOWN_REFERENCE,
                    format!(
                        "configuration {:?} suppresses parameter sequence {}, which does not exist",
                        self.name, seq.0
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), ParamError> {
    if name.trim().is_empty() {
        return Err(super::ParamError::new(
            super::PARAM_INVALID_VALUE,
            "a configuration name must not be empty",
        ));
    }
    if name.len() > super::PARAM_MAX_NAME_BYTES {
        return Err(super::ParamError::new(
            super::PARAM_INVALID_VALUE,
            format!(
                "configuration name is {} bytes, over the {}-byte budget",
                name.len(),
                super::PARAM_MAX_NAME_BYTES
            ),
        ));
    }
    Ok(())
}

/// Every configuration of one document, unique by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamConfigTable {
    rows: BTreeMap<String, ParamConfig>,
}

impl ParamConfigTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, config: ParamConfig) -> Result<(), ParamError> {
        if self.rows.contains_key(&config.name) {
            return Err(ParamError::new(
                super::PARAM_DUPLICATE_NAME,
                format!("configuration {:?} already exists", config.name),
            ));
        }
        validate_name(&config.name)?;
        self.rows.insert(config.name.clone(), config);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ParamConfig> {
        self.rows.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<ParamConfig> {
        self.rows.remove(name)
    }

    pub fn rows(&self) -> impl Iterator<Item = &ParamConfig> {
        self.rows.values()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.rows.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Validate every row against the base table and every parent link.
    pub fn validate(&self, base: &ParamTable) -> Result<(), ParamError> {
        for config in self.rows.values() {
            config.validate(base)?;
            if let Some(parent) = &config.parent {
                if !self.rows.contains_key(parent) {
                    return Err(ParamError::new(
                        PARAM_UNKNOWN_CONFIG,
                        format!(
                            "configuration {:?} names unknown parent {parent:?}",
                            config.name
                        ),
                    ));
                }
            }
        }
        // Cycle detection is part of validation, not only of resolution: an
        // unused cycle must still be refused when the table is written.
        for name in self.rows.keys() {
            self.chain(name)?;
        }
        Ok(())
    }

    /// The parent-first chain ending at `name`; a cycle is refused.
    fn chain(&self, name: &str) -> Result<Vec<&ParamConfig>, ParamError> {
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = Some(name);
        while let Some(current) = cursor {
            if !seen.insert(current.to_string()) {
                return Err(ParamError::new(
                    PARAM_CONFIG_CYCLE,
                    format!("configuration {current:?} is part of a parent cycle"),
                ));
            }
            let config = self.rows.get(current).ok_or_else(|| {
                ParamError::new(
                    PARAM_UNKNOWN_CONFIG,
                    format!("configuration {current:?} does not exist"),
                )
            })?;
            chain.push(config);
            cursor = config.parent.as_deref();
        }
        chain.reverse();
        Ok(chain)
    }
}

/// How the active configuration relates to the base table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigStatus {
    pub name: Option<String>,
    /// Base sequences that keep their base expression.
    pub inherited: BTreeSet<ParamSeq>,
    /// Base sequences whose expression the active chain replaces.
    pub overridden: BTreeSet<ParamSeq>,
    /// Base sequences the active chain removes.
    pub suppressed: BTreeSet<ParamSeq>,
}

/// Resolve the effective parameter table for one active configuration.
///
/// `active == None` returns a clone of the base table and an empty status (no
/// configuration is applied). A missing name, a parent cycle, an invalid
/// override or a suppression that breaks another expression is refused.
pub fn resolve_active(
    base: &ParamTable,
    configs: &ParamConfigTable,
    active: Option<&str>,
) -> Result<(ParamTable, ConfigStatus), ParamError> {
    let Some(active) = active else {
        return Ok((base.clone(), ConfigStatus::default()));
    };
    let chain = configs.chain(active)?;
    let mut effective = base.clone();
    let mut status = ConfigStatus {
        name: Some(active.to_string()),
        inherited: base.seqs().collect(),
        ..ConfigStatus::default()
    };
    for config in &chain {
        for (seq, expr) in &config.overrides {
            let row = base.get(*seq).ok_or_else(|| {
                ParamError::new(
                    super::PARAM_UNKNOWN_REFERENCE,
                    format!(
                        "configuration {:?} overrides missing parameter {}",
                        config.name, seq.0
                    ),
                )
            })?;
            let replacement = ParamRow {
                seq: row.seq,
                name: row.name.clone(),
                kind: row.kind,
                unit: row.unit,
                expr: expr.clone(),
            };
            effective.insert(replacement)?;
            status.overridden.insert(*seq);
            // A child override **re-enables** a sequence its parent suppressed:
            // the effective table has the row, so the status must not still call
            // it suppressed (the host list and the value-edit gate both read it).
            status.suppressed.remove(seq);
            status.inherited.remove(seq);
        }
        for seq in &config.suppressed {
            effective.remove(*seq);
            status.overridden.remove(seq);
            status.inherited.remove(seq);
            status.suppressed.insert(*seq);
        }
    }
    // The effective table must still be a DAG with resolvable references and
    // matching kinds; this is the configuration's static gate.
    evaluate_table(&effective)?;
    Ok((effective, status))
}

#[cfg(test)]
mod tests;
