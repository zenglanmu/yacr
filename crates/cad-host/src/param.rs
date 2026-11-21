//! Document-level parameter list host read (ADR-0043 P1).
//!
//! The base table is the authority (`tables/parameters.json`); a configuration is
//! a runtime overlay. This module projects both into one read-only list:
//!
//! * every base row, in ascending sequence order;
//! * its status under the active configuration — `inherited`, `overridden` or
//!   `suppressed`;
//! * the effective value where one exists, and the base value for a suppressed
//!   row (or `0.0` if the base table cannot evaluate).
//!
//! No document is an explicit empty answer (`active: None`, `rows: []`,
//! `total: 0`), never an error: the caller asked a real question and "nothing is
//! open" is its real answer.
//!
//! The wire mapping lives in `cad-ui-core`; this module carries host values only
//! (the same split as [`crate::BimPage`] and the BIM query response).

use cad_domain::param;
use cad_domain::CadResult;

use crate::HostCore;

/// One parameter row of the document-level list.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamListRow {
    /// The base table sequence (identity across renames).
    pub seq: u64,
    /// The base row's name.
    pub name: String,
    /// `length` | `number` | `angle`.
    pub kind: String,
    /// The canonical or declared unit token (`mm`, `rad`), or `None`.
    pub unit: Option<String>,
    /// The effective value; for a suppressed row, the base value (or `0.0`).
    pub value: f64,
    /// `inherited` | `overridden` | `suppressed`.
    pub status: String,
}

/// The whole base table plus the active configuration's status.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamList {
    /// The configuration currently applied, or `None` for the base table.
    pub active: Option<String>,
    /// Base rows in ascending sequence order.
    pub rows: Vec<ParamListRow>,
    /// `rows.len()`.
    pub total: usize,
}

impl HostCore {
    /// The document parameter list under the active configuration.
    ///
    /// Runtime state only: the selection is not persisted and a reopened document
    /// starts at the base table. A malformed table or active configuration is the
    /// backend's own `param.*` error and travels out verbatim.
    pub fn param_list(&self) -> CadResult<ParamList> {
        let controller = self.controller.borrow();
        let Some(database) = controller.drawing() else {
            return Ok(ParamList::default());
        };
        let status = database.param_config_status()?;
        let effective = database.param_values()?;
        // A suppressed row is absent from `effective`. Its base value is the
        // honest fallback; only evaluate the base table when a suppressed row
        // actually needs it.
        let base = if status.suppressed.is_empty() {
            None
        } else {
            param::evaluate_table(database.parameters()).ok()
        };
        let mut rows = Vec::with_capacity(
            status.inherited.len() + status.overridden.len() + status.suppressed.len(),
        );
        for row in database.parameters().rows() {
            let row_status = if status.suppressed.contains(&row.seq) {
                "suppressed"
            } else if status.overridden.contains(&row.seq) {
                "overridden"
            } else {
                "inherited"
            };
            let value = effective
                .get(row.seq)
                .map(|value| value.value)
                .or_else(|| {
                    base.as_ref()
                        .and_then(|values| values.get(row.seq))
                        .map(|value| value.value)
                })
                .unwrap_or(0.0);
            let unit = row
                .unit
                .or_else(|| row.kind.canonical_unit())
                .map(|unit| unit.token().to_string());
            rows.push(ParamListRow {
                seq: row.seq.0,
                name: row.name.clone(),
                kind: row.kind.token().to_string(),
                unit,
                value,
                status: row_status.to_string(),
            });
        }
        Ok(ParamList {
            active: status.name,
            total: rows.len(),
            rows,
        })
    }
}
