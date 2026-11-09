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
use cad_app::{CommandId, CommandPayload, SelectionToolPayload};

use super::{
    commands::{
        invalid_audit_argument_reason, invalid_filter_argument_reason,
        invalid_filter_logic_argument_reason, invalid_find_argument_reason,
        invalid_getsel_argument_reason, invalid_qselect_argument_reason,
        invalid_ssx_argument_reason, looks_like_filter_expression, looks_like_structured_query,
        parse_audit_argument, parse_filter_request, parse_find_argument, parse_property_filter,
        parse_qselect_request, parse_ssx_argument, AUDIT_SELECTOR, FILTER_SELECTOR,
        FIND_REPORT_SELECTOR, FIND_SELECTOR, GETSEL_SELECTOR, QSELECT_SELECTOR, SSX_SELECTOR,
    },
    CadSession,
};

impl CadSession {
    /// Apply one typed filter line (`QSELECT`/`FILTER`/`GETSEL`).
    ///
    /// The structured forms are tried first (they carry an operator/logic
    /// grammar the legacy `PROPERTY VALUE` line never has); a line without an
    /// operator falls through to the legacy one-shot payload, so both contracts
    /// stay reachable.
    pub(super) fn select_filter_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        if selector == QSELECT_SELECTOR {
            if let Some(request) = parse_qselect_request(argument) {
                self.core.record_command(selector);
                return self.dispatch_payload(
                    CommandId::QSelect,
                    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::QSelect(
                        Box::new(request),
                    ))),
                );
            }
            // A property/operator shape that parsed as a property but not as a
            // complete request is a malformed structured line, not a legacy one.
            if looks_like_structured_query(argument) {
                return self.refuse_argument(invalid_qselect_argument_reason(text));
            }
        }
        if selector == FILTER_SELECTOR {
            if let Some(request) = parse_filter_request(argument) {
                self.core.record_command(selector);
                return self.dispatch_payload(
                    CommandId::FilterSelect,
                    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Filter(
                        Box::new(request),
                    ))),
                );
            }
            if looks_like_filter_expression(argument) {
                return self.refuse_argument(invalid_filter_logic_argument_reason(text));
            }
        }
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

    /// Apply one typed `SSX <criterion> ...` line.
    ///
    /// The line edits the application's session filter (one value per
    /// criterion) and applies it; the application owns the unknown-criterion,
    /// empty-filter and unmodelled-criterion refusals.
    pub(super) fn ssx_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(request) = parse_ssx_argument(argument) else {
            return self.refuse_argument(invalid_ssx_argument_reason(text));
        };
        self.core.record_command(SSX_SELECTOR);
        self.dispatch_payload(
            CommandId::SelectMatchingProperties,
            CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Ssx(request))),
        )
    }

    /// Apply one typed `AUDIT [YES|NO|LOG]` line.
    pub(super) fn audit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(request) = parse_audit_argument(argument) else {
            return self.refuse_argument(invalid_audit_argument_reason(text));
        };
        self.core.record_command(AUDIT_SELECTOR);
        self.dispatch_payload(
            CommandId::AuditDrawing,
            CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Audit(request))),
        )
    }
}
