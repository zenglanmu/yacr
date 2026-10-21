//! The text-edit popup phase's recorded reads and its single-line summary
//! (`YACR_PROBE_POPUP_EVIDENCE` phase 2, split out of
//! [`super::popup_textedit`] at the 1000-line code-file gate).
//!
//! Everything here is formatting over values the phase actually read from the
//! real host/`cad_status`: nothing is synthesized, and `-` always means "absent /
//! never read". The pass flag is derived from those reads and is `0` whenever a
//! step is missing, wrong, skipped or failed.

use std::sync::{Arc, Mutex, MutexGuard};

use cad_ui_core::session::CadStatus;

use super::popup_textedit::{APPLY_ACK, EXPECTED_REOPEN_LINES, EXPECTED_STORED, PATCH_LOG};

/// The driver's accumulated reads; all values are real host/`cad_status` reads.
#[derive(Default)]
pub(super) struct TextEditEvidence {
    pub(super) skipped: Option<String>,
    pub(super) selection_text: Option<String>,
    pub(super) selection_multiline: Option<bool>,
    pub(super) rows_before: Option<usize>,
    pub(super) row_keys_before: Option<String>,
    pub(super) entities_before: Option<u64>,
    pub(super) payload_len: Option<usize>,
    pub(super) payload_multiline: Option<bool>,
    pub(super) payload_fingerprint: Option<String>,
    pub(super) open_ack: Option<String>,
    pub(super) status_open: Option<String>,
    pub(super) xid: Option<String>,
    pub(super) handle: Option<&'static str>,
    pub(super) apply_ack: Option<String>,
    pub(super) text_after: Option<String>,
    pub(super) text_ok: Option<bool>,
    pub(super) multiline_after: Option<bool>,
    pub(super) entities_after: Option<u64>,
    pub(super) rows_after: Option<usize>,
    pub(super) row_keys_after: Option<String>,
    pub(super) tool_after: Option<String>,
    pub(super) error_after: Option<String>,
    pub(super) close_ack: Option<String>,
    pub(super) status_after: Option<String>,
    pub(super) closed_confirmed: bool,
    pub(super) gate_released: Option<bool>,
    pub(super) reopen_ack: Option<String>,
    pub(super) reopen_status_open: Option<String>,
    pub(super) reopen_payload_len: Option<usize>,
    pub(super) reopen_has_newline: Option<bool>,
    pub(super) reopen_lines: Option<usize>,
    pub(super) reopen_xid: Option<String>,
    pub(super) reopen_handle: Option<&'static str>,
    pub(super) reopen_close_ack: Option<String>,
    pub(super) reopen_status_after: Option<String>,
    pub(super) reopen_closed_confirmed: bool,
    pub(super) failed: Vec<String>,
    pub(super) aborted: Option<String>,
}

impl TextEditEvidence {
    /// Whether a later step must not run: an earlier step failed (`aborted`) or
    /// the precondition found nothing to prove (`skipped`).
    pub(super) fn is_aborted(&self) -> bool {
        self.aborted.is_some() || self.skipped.is_some()
    }

    /// Log a failed step at once and remember it; the first failure is the abort
    /// point every later step checks.
    pub(super) fn fail(&mut self, step: &str, detail: &str) {
        eprintln!("[probe] popup textedit evidence: {step} FAILED: {detail}");
        self.failed.push(format!("{step}={}", flat(detail)));
        if self.aborted.is_none() {
            self.aborted = Some(step.to_string());
        }
    }

    /// A later step that did not run because an earlier one failed or the
    /// precondition skipped the phase.
    pub(super) fn skip(&mut self, step: &str) {
        let at = self
            .aborted
            .clone()
            .or_else(|| self.skipped.clone())
            .unwrap_or_else(|| "-".to_string());
        eprintln!("[probe] popup textedit evidence: {step} skipped (halted at {at})");
    }

    /// Both entity counts were read and are equal.
    pub(super) fn entities_same(&self) -> Option<bool> {
        match (self.entities_before, self.entities_after) {
            (Some(before), Some(after)) => Some(before == after),
            _ => None,
        }
    }
}

/// Build the single summary line from the recorded reads.
///
/// `-` means "absent / never read"; `popup_textedit_pass` is derived from the
/// reads and is `0` whenever any step is missing, wrong, skipped or failed.
pub(super) fn summary_line(evidence: &TextEditEvidence, hold: u64, reopen_hold: u64) -> String {
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(flat)
            .unwrap_or_else(|| "-".to_string())
    };
    let number = |value: Option<usize>| match value {
        Some(value) => value.to_string(),
        None => "-".to_string(),
    };
    let count = |value: Option<u64>| match value {
        Some(value) => value.to_string(),
        None => "-".to_string(),
    };

    let opened = u8::from(
        evidence.open_ack.is_some()
            && evidence
                .status_open
                .as_deref()
                .is_some_and(|value| value != "-")
            && evidence.handle.is_some(),
    );
    let applied = u8::from(evidence.apply_ack.as_deref() == Some(APPLY_ACK));
    let closed = u8::from(
        evidence.close_ack.is_some()
            && evidence.status_after.as_deref() == Some("-")
            && evidence.closed_confirmed,
    );
    let text_ok = u8::from(evidence.text_ok == Some(true));
    let multiline = u8::from(evidence.multiline_after == Some(true));
    let entities_same_token = match evidence.entities_same() {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    };
    let entities_same = u8::from(entities_same_token == "1");
    let reopen_opened = u8::from(
        evidence.reopen_ack.is_some()
            && evidence
                .reopen_status_open
                .as_deref()
                .is_some_and(|value| value != "-")
            && evidence.reopen_handle.is_some(),
    );
    let reopen_closed = u8::from(
        evidence.reopen_close_ack.is_some()
            && evidence.reopen_status_after.as_deref() == Some("-")
            && evidence.reopen_closed_confirmed,
    );
    let reopen_payload_ok = evidence.reopen_payload_len.is_some_and(|len| len > 0)
        && evidence.reopen_has_newline == Some(true)
        && evidence.reopen_lines == Some(EXPECTED_REOPEN_LINES);
    let pass = u8::from(
        evidence.aborted.is_none()
            && evidence.skipped.is_none()
            && opened == 1
            && applied == 1
            && text_ok == 1
            && multiline == 1
            && entities_same == 1
            && evidence.tool_after.as_deref() == Some("-")
            && evidence.error_after.as_deref() == Some("-")
            && closed == 1
            && evidence.gate_released == Some(true)
            && reopen_opened == 1
            && reopen_payload_ok
            && reopen_closed == 1,
    );
    let failed = if evidence.failed.is_empty() {
        "-".to_string()
    } else {
        evidence.failed.join(",")
    };
    let aborted = evidence
        .aborted
        .as_deref()
        .map(flat)
        .unwrap_or_else(|| "-".to_string());

    format!(
        "popup_textedit_synthetic=1 popup_textedit_skipped={} popup_textedit_pass={pass} \
         te_pass={pass} te_skipped={} te_selection_text={} te_selection_multiline={} \
         te_selection_rows_before={} te_row_keys_before={} te_entities_before={} \
         te_entities_after={} te_entities_same={entities_same_token} te_opened={opened} \
         te_ack={} te_popup_open={} \
         te_xid={} te_native_handle={} te_payload_len={} te_multiline={} \
         te_payload_fingerprint={} te_payload_source=host_derivation_before_open \
         te_patch={PATCH_LOG} te_admission={} te_applied={applied} te_text={} \
         te_expected={EXPECTED_STORED} te_text_ok={text_ok} te_multiline_after={multiline} \
         te_selection_rows_after={} te_row_keys_after={} te_tool={} te_error={} te_closed={closed} \
         te_status_after={} te_gate_released={} te_reopen_opened={reopen_opened} \
         te_reopen_payload_len={} te_reopen_has_newline={} te_reopen_lines={} \
         te_reopen_xid={} te_reopen_closed={reopen_closed} te_reopen_hold_s={reopen_hold} \
         te_hold_s={hold} te_step_failed={failed} \
         te_aborted_at={aborted} \
         te_display_channel=none_cad_status_exposes_no_entity_display_or_root_count \
         te_trigger=probe_driver_not_a_real_click \
         te_apply_channel=admit_selected_text_then_owner_tick_drain_the_cad_set_selected_text_path",
        string(&evidence.skipped),
        string(&evidence.skipped),
        text(&evidence.selection_text),
        flag(evidence.selection_multiline),
        number(evidence.rows_before),
        text(&evidence.row_keys_before),
        count(evidence.entities_before),
        count(evidence.entities_after),
        text(&evidence.open_ack),
        text(&evidence.status_open),
        text(&evidence.xid),
        evidence.handle.unwrap_or("-"),
        number(evidence.payload_len),
        flag(evidence.payload_multiline),
        text(&evidence.payload_fingerprint),
        text(&evidence.apply_ack),
        text(&evidence.text_after),
        number(evidence.rows_after),
        text(&evidence.row_keys_after),
        text(&evidence.tool_after),
        text(&evidence.error_after),
        text(&evidence.status_after),
        flag(evidence.gate_released),
        number(evidence.reopen_payload_len),
        flag(evidence.reopen_has_newline),
        number(evidence.reopen_lines),
        text(&evidence.reopen_xid),
    )
}

/// `1`/`0`/`-` for an optional boolean.
fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    }
}

/// `1`/`0` for a known boolean.
pub(super) fn bool_token(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

fn string(value: &Option<String>) -> String {
    value
        .as_deref()
        .map(flat)
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn optional_count(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// A short, whitespace-free fingerprint: the first 8 and last 8 characters, or
/// just the first 8 for a short string. The whole string is never logged.
pub(super) fn fingerprint(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return "-".to_string();
    }
    let head: String = chars.iter().take(8).collect();
    if chars.len() <= 16 {
        return flat(&head);
    }
    let tail: String = chars[chars.len() - 8..].iter().collect();
    flat(&format!("{head}..{tail}"))
}

/// The selection's stable machine row keys, comma-joined (`-` when absent).
pub(super) fn row_keys(status: &CadStatus) -> String {
    if status.selection.rows.is_empty() {
        return "-".to_string();
    }
    status
        .selection
        .rows
        .iter()
        .map(|row| row.key.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// Collapse whitespace so the summary stays one whitespace-free field list.
pub(super) fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("_")
}

/// `flat`, capped at 80 characters with an explicit `...` marker.
///
/// Only display/logging uses this: the equality check in the verify step always
/// compares the full strings, so a capped log can never hide a mismatch.
pub(super) fn short_flat(value: &str) -> String {
    let flat = flat(value);
    let chars: Vec<char> = flat.chars().collect();
    if chars.len() <= 80 {
        flat
    } else {
        format!("{}...", chars[..80].iter().collect::<String>())
    }
}

pub(super) fn lock_evidence(
    evidence: &Arc<Mutex<TextEditEvidence>>,
) -> MutexGuard<'_, TextEditEvidence> {
    evidence
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::super::popup_textedit::PATCH;
    use super::*;

    /// Every read the pass criterion needs, as it looks after a clean run.
    fn passing() -> TextEditEvidence {
        TextEditEvidence {
            skipped: None,
            selection_text: Some("MLEADER_ANNOTATION".into()),
            selection_multiline: Some(true),
            rows_before: Some(4),
            row_keys_before: Some("id,type,layer,handle".into()),
            entities_before: Some(42),
            payload_len: Some(20),
            payload_multiline: Some(true),
            payload_fingerprint: Some("MLEADER_..TATION".into()),
            open_ack: Some("accepted=true_existing=false".into()),
            status_open: Some("textEdit:popup-1:true".into()),
            xid: Some("0x2a".into()),
            handle: Some("x11_xid"),
            apply_ack: Some(APPLY_ACK.into()),
            text_after: Some(EXPECTED_STORED.into()),
            text_ok: Some(true),
            multiline_after: Some(true),
            entities_after: Some(42),
            rows_after: Some(4),
            row_keys_after: Some("id,type,layer,handle".into()),
            tool_after: Some("-".into()),
            error_after: Some("-".into()),
            close_ack: Some("accepted=true".into()),
            status_after: Some("-".into()),
            closed_confirmed: true,
            gate_released: Some(true),
            reopen_ack: Some("accepted=true_existing=false".into()),
            reopen_status_open: Some("textEdit:popup-1:true".into()),
            reopen_payload_len: Some(13),
            reopen_has_newline: Some(true),
            reopen_lines: Some(2),
            reopen_xid: Some("0x2b".into()),
            reopen_handle: Some("x11_xid"),
            reopen_close_ack: Some("accepted=true".into()),
            reopen_status_after: Some("-".into()),
            reopen_closed_confirmed: true,
            failed: Vec::new(),
            aborted: None,
        }
    }

    #[test]
    fn summary_is_one_line_with_the_required_fields_on_a_clean_run() {
        let line = summary_line(&passing(), 30, 20);
        assert!(!line.contains('\n'), "one summary line: {line}");
        for expected in [
            "popup_textedit_skipped=-",
            "popup_textedit_pass=1",
            "te_pass=1",
            "te_opened=1",
            "te_multiline=1",
            "te_payload_len=20",
            "te_selection_rows_before=4",
            "te_selection_rows_after=4",
            "te_display_channel=none_cad_status_exposes_no_entity_display_or_root_count",
            "te_payload_fingerprint=MLEADER_..TATION",
            "te_applied=1",
            "te_text_ok=1",
            "te_expected=EVIDENCE\\PEDIT",
            "te_multiline_after=1",
            "te_entities_same=1",
            "te_tool=-",
            "te_error=-",
            "te_closed=1",
            "te_gate_released=1",
            "te_reopen_opened=1",
            "te_reopen_payload_len=13",
            "te_reopen_has_newline=1",
            "te_reopen_lines=2",
            "te_reopen_xid=0x2b",
            "te_reopen_closed=1",
            "te_reopen_hold_s=20",
            "te_patch=EVIDENCE\\nEDIT",
            "te_hold_s=30",
            "te_step_failed=-",
        ] {
            assert!(line.contains(expected), "missing {expected}: {line}");
        }
    }

    #[test]
    fn a_skipped_phase_never_claims_a_pass() {
        let mut skipped = passing();
        skipped.skipped = Some("no_text_selection".into());
        skipped.text_ok = None;
        let line = summary_line(&skipped, 30, 20);
        assert!(
            line.contains("popup_textedit_skipped=no_text_selection"),
            "{line}"
        );
        assert!(line.contains("popup_textedit_pass=0"), "{line}");
        assert!(line.contains("te_pass=0"), "{line}");
    }

    #[test]
    fn any_missing_or_wrong_read_clears_the_pass_flag() {
        let mut wrong_text = passing();
        wrong_text.text_ok = Some(false);
        wrong_text.text_after = Some("EVIDENCE_EDIT".into());
        assert!(summary_line(&wrong_text, 30, 20).contains("popup_textedit_pass=0"));

        let mut count_changed = passing();
        count_changed.entities_after = Some(43);
        let line = summary_line(&count_changed, 30, 20);
        assert!(line.contains("te_entities_same=0"), "{line}");
        assert!(line.contains("popup_textedit_pass=0"), "{line}");

        let mut tool_active = passing();
        tool_active.tool_after = Some("TEXT".into());
        assert!(summary_line(&tool_active, 30, 20).contains("popup_textedit_pass=0"));

        let mut error_set = passing();
        error_set.error_after = Some("textedit.target_not_text".into());
        assert!(summary_line(&error_set, 30, 20).contains("popup_textedit_pass=0"));

        let mut still_open = passing();
        still_open.status_after = Some("textEdit:popup-1:true".into());
        assert!(summary_line(&still_open, 30, 20).contains("popup_textedit_pass=0"));

        let mut gate_refused = passing();
        gate_refused.gate_released = Some(false);
        assert!(summary_line(&gate_refused, 30, 20).contains("popup_textedit_pass=0"));

        let mut aborted = passing();
        aborted.aborted = Some("verify_text".into());
        assert!(summary_line(&aborted, 30, 20).contains("popup_textedit_pass=0"));

        let mut unread_entities = passing();
        unread_entities.entities_before = None;
        let line = summary_line(&unread_entities, 30, 20);
        assert!(line.contains("te_entities_same=-"), "{line}");
        assert!(line.contains("popup_textedit_pass=0"), "{line}");
    }

    #[test]
    fn a_missing_or_wrong_reopen_clears_the_pass_flag() {
        // The reopen is part of the pass: no reopen reads at all cannot pass.
        let mut never_reopened = passing();
        never_reopened.reopen_ack = None;
        never_reopened.reopen_status_open = None;
        never_reopened.reopen_xid = None;
        never_reopened.reopen_handle = None;
        never_reopened.reopen_payload_len = None;
        never_reopened.reopen_has_newline = None;
        never_reopened.reopen_lines = None;
        never_reopened.reopen_close_ack = None;
        never_reopened.reopen_status_after = None;
        never_reopened.reopen_closed_confirmed = false;
        let line = summary_line(&never_reopened, 30, 20);
        assert!(line.contains("te_reopen_opened=0"), "{line}");
        assert!(line.contains("te_reopen_payload_len=-"), "{line}");
        assert!(line.contains("te_reopen_has_newline=-"), "{line}");
        assert!(line.contains("te_reopen_lines=-"), "{line}");
        assert!(line.contains("te_reopen_xid=-"), "{line}");
        assert!(line.contains("te_reopen_closed=0"), "{line}");
        assert!(line.contains("popup_textedit_pass=0"), "{line}");
        assert!(line.contains("te_pass=0"), "{line}");

        // One line instead of two (the `\P` round trip did not happen).
        let mut one_line = passing();
        one_line.reopen_has_newline = Some(false);
        one_line.reopen_lines = Some(1);
        let line = summary_line(&one_line, 30, 20);
        assert!(line.contains("te_reopen_has_newline=0"), "{line}");
        assert!(line.contains("te_reopen_lines=1"), "{line}");
        assert!(line.contains("popup_textedit_pass=0"), "{line}");

        // The reopened dialog never closed.
        let mut still_open = passing();
        still_open.reopen_status_after = Some("textEdit:popup-1:true".into());
        still_open.reopen_closed_confirmed = false;
        let line = summary_line(&still_open, 30, 20);
        assert!(line.contains("te_reopen_closed=0"), "{line}");
        assert!(line.contains("popup_textedit_pass=0"), "{line}");

        // A failed reopen step aborts the phase like any other step.
        let mut aborted = passing();
        aborted.aborted = Some("reopen_lines".into());
        let line = summary_line(&aborted, 30, 20);
        assert!(line.contains("te_aborted_at=reopen_lines"), "{line}");
        assert!(line.contains("popup_textedit_pass=0"), "{line}");
    }

    #[test]
    fn fingerprint_is_short_and_never_echoes_the_whole_long_string() {
        assert_eq!(fingerprint(""), "-");
        assert_eq!(fingerprint("abc"), "abc");
        assert_eq!(fingerprint("line one\nline two"), "line_one..line_two");
        let long = "0123456789abcdefghijklmnopqrstuvwxyz";
        let value = fingerprint(long);
        assert_eq!(value, "01234567..stuvwxyz");
        assert!(value.len() < long.len());
    }

    #[test]
    fn patch_is_the_documented_newline_string_and_its_expected_stored_form() {
        assert_eq!(PATCH, "EVIDENCE\nEDIT");
        assert_eq!(EXPECTED_STORED, "EVIDENCE\\PEDIT");
        assert!(PATCH.contains('\n'));
        assert!(!EXPECTED_STORED.contains('\n'));
        assert_eq!(PATCH_LOG, "EVIDENCE\\nEDIT");
        assert!(
            !PATCH_LOG.contains('\n'),
            "the summary patch token is one line"
        );
        // The expected stored form is exactly the policy's mapping.
        assert_eq!(PATCH.replace('\n', "\\P"), EXPECTED_STORED);
    }

    #[test]
    fn log_text_is_capped_but_the_pass_check_uses_the_full_string() {
        let long = "x".repeat(200);
        let capped = short_flat(&long);
        assert!(capped.ends_with("..."), "{capped}");
        assert_eq!(capped.chars().count(), 83);
        assert_eq!(short_flat("a\nb"), "a_b");
        // The full string is what `te_text_ok` compares, never the capped log.
        assert_ne!(capped, long);
    }

    #[test]
    fn flag_and_row_keys_are_whitespace_free() {
        assert_eq!(flag(Some(true)), "1");
        assert_eq!(flag(Some(false)), "0");
        assert_eq!(flag(None), "-");
        assert_eq!(bool_token(true), "1");
        assert_eq!(bool_token(false), "0");
        let mut status = cad_ui_core::unsupported_status(None);
        assert_eq!(row_keys(&status), "-");
        status
            .selection
            .rows
            .push(cad_ui_core::session::CadSelectionRow {
                key: "id".into(),
                value: Some("1".into()),
                group: "entity".into(),
                mixed: false,
                editable: false,
                kind: "text".into(),
                options: Vec::new(),
            });
        assert_eq!(row_keys(&status), "id");
    }
}
