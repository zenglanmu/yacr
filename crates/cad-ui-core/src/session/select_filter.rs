//! The typed `QSELECT` / `FILTER` / `GETSEL` / `-FIND` / `FIND` command lines
//! (2026-11-07).
//!
//! These are argument forms of canonical selectors: the session parses the
//! property/value (or needle/replacement) and dispatches the typed
//! [`cad_app::CommandPayload`] through the ordinary host funnel. The application
//! layer is still the authority — it re-parses the filter word, refuses unknown
//! properties and applies the matching — so this module never invents a match.
//!
//! `QSELECT`/`GETSEL` replace the selection; `FILTER` unions into it. `-FIND`
//! reports only; `FIND <needle>` reports and `FIND <needle> => <text>` replaces.
//! A bare selector keeps the name-only funnel and is refused by the application
//! with its own needs-argument reason, never defaulted.

use cad_app::interaction::SelectionMode;
use cad_app::{CommandId, CommandPayload};

use super::{
    commands::{
        invalid_filter_argument_reason, invalid_find_argument_reason,
        invalid_getsel_argument_reason, parse_find_argument, parse_property_filter,
        FILTER_SELECTOR, FIND_REPORT_SELECTOR, FIND_SELECTOR, GETSEL_SELECTOR, QSELECT_SELECTOR,
    },
    CadSession,
};

impl CadSession {
    /// Apply one typed filter line (`QSELECT`/`FILTER`/`GETSEL`).
    pub(super) fn select_filter_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let Some((property, value)) = parse_property_filter(argument) else {
            return self.refuse_argument(invalid_filter_argument_reason(text, selector));
        };
        if selector == GETSEL_SELECTOR && !matches!(property.as_str(), "TYPE" | "LAYER") {
            return self.refuse_argument(invalid_getsel_argument_reason(text));
        }
        let (id, mode) = match selector {
            QSELECT_SELECTOR => (CommandId::QSelect, SelectionMode::Replace),
            FILTER_SELECTOR => (CommandId::FilterSelect, SelectionMode::Add),
            GETSEL_SELECTOR => (CommandId::GetSelection, SelectionMode::Replace),
            _ => return self.refuse_argument(invalid_filter_argument_reason(text, selector)),
        };
        self.core.record_command(selector);
        self.dispatch_payload(
            id,
            CommandPayload::SelectFilter {
                property,
                value,
                mode,
            },
        )
    }

    /// Apply one typed `-FIND` / `FIND` line.
    pub(super) fn find_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let Some((needle, replacement)) = parse_find_argument(argument) else {
            return self.refuse_argument(invalid_find_argument_reason(text, selector));
        };
        let (id, replacement) = if selector == FIND_REPORT_SELECTOR {
            if replacement.is_some() {
                return self.refuse_argument(invalid_find_argument_reason(text, selector));
            }
            (CommandId::FindReport, None)
        } else if selector == FIND_SELECTOR {
            (CommandId::FindReplace, replacement)
        } else {
            return self.refuse_argument(invalid_find_argument_reason(text, selector));
        };
        self.core.record_command(selector);
        self.dispatch_payload(
            id,
            CommandPayload::TextFind {
                needle,
                replacement,
            },
        )
    }
}
