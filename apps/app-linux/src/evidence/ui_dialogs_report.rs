//! Reads and log formats for [`super::ui_dialogs`] (`YACR_PROBE_UI_DIALOGS`).
//!
//! Split out so the driver, its step bodies and its tests all stay under the
//! 1000-line code-file gate (`scripts/check-file-size.py`, no whitelist). This
//! module owns:
//!
//! * [`Evidence`] — the driver's accumulated **real** reads (a `cad_status`
//!   block, a DOM read-back or a host acknowledgement; never a synthesised
//!   value) and the setters the steps use;
//! * [`summary_line`] — the one final line, including
//!   `ui_dialogs_trigger=probe_driver_not_a_real_click` and only the parts that
//!   really were verified;
//! * the per-read line bodies ([`selection_detail`], [`command_log_line`],
//!   [`xid_detail`]), so the exact text an external grep relies on is defined in
//!   one place and asserted by tests.

use std::sync::{Arc, Mutex, MutexGuard};

use cad_ui_core::session::{CadCommandLogEntry, CadCommandLogKind, CadSelection, CadStatus};

/// The row keys the selection line must carry so the log alone proves the
/// palette's grouped rows exist (the full row set is larger: identity, geometry
/// and display groups).
pub(super) const SELECTED_ROW_KEYS: [&str; 7] = [
    "id",
    "type",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "transparency",
];

/// The driver's accumulated reads. Every field is a real `cad_status`/DOM read
/// or a real host acknowledgement; nothing is synthesised.
#[derive(Default)]
pub(super) struct Evidence {
    palette_present: Option<bool>,
    palette_canvas: Option<String>,
    main_xid: Option<String>,
    document_label: Option<String>,
    selection_count: Option<u64>,
    selection_rows_complete: bool,
    command_log_count: Option<usize>,
    dist_command_logged: Option<bool>,
    dist_value: Option<String>,
    options_popup: Option<String>,
    popup_xid: Option<String>,
    popup_handle: Option<&'static str>,
    close_ack: Option<bool>,
    popup_after_close: Option<String>,
    failed: Vec<String>,
}

impl Evidence {
    /// The DOM read-back of the palette toggle.
    pub(super) fn set_palette(&mut self, present: bool, canvas: Option<String>) {
        self.palette_present = Some(present);
        self.palette_canvas = canvas;
        if !present {
            self.fail("properties", "the palette is not mounted after Ctrl+1");
        }
    }

    pub(super) fn set_main_xid(&mut self, xid: String) {
        self.main_xid = Some(xid);
    }

    pub(super) fn set_document(&mut self, label: Option<String>) {
        self.document_label = label;
    }

    /// The real `cad_status.selection` summary.
    pub(super) fn set_selection(&mut self, count: u64, rows_complete: bool) {
        self.selection_count = Some(count);
        self.selection_rows_complete = rows_complete;
        if count != 1 {
            self.fail("selection", &format!("expected count=1, read {count}"));
        } else if !rows_complete {
            self.fail("selection", "the seven logged row keys are not all present");
        }
    }

    /// The real `cad_status.commandLog` reads.
    pub(super) fn set_command_log(
        &mut self,
        entries: usize,
        dist_command: bool,
        dist_value: Option<String>,
    ) {
        self.command_log_count = Some(entries);
        self.dist_command_logged = Some(dist_command);
        self.dist_value = dist_value;
        if !dist_command {
            self.fail("commandlog", "no admitted DIST entry in commandLog");
        }
    }

    /// The real `cad_status.popup` block plus the in-DOM overlay identity.
    ///
    /// Since WP3 the popup is an overlay in the single UI tree, so `handle` is
    /// `dom_overlay` and `xid` is always `None`. This only **stores** the read:
    /// whether a still-closed popup is a failure depends on where in the timeline
    /// it was read (the driver retries once), so the step body judges it with
    /// [`Self::fail_popup_never_opened`] instead.
    pub(super) fn set_popup(
        &mut self,
        popup: String,
        handle: Option<&'static str>,
        xid: Option<String>,
    ) {
        self.options_popup = Some(popup);
        self.popup_handle = handle;
        self.popup_xid = xid;
    }

    /// The **last** popup read still found no dialog, so the run cannot pass.
    pub(super) fn fail_popup_never_opened(&mut self, popup: &str) {
        self.fail(
            "options_popup",
            &format!("the Options dialog never opened (last read {popup})"),
        );
    }

    /// Whether the Options dialog is really open according to the last read.
    pub(super) fn popup_opened(&self) -> bool {
        self.options_popup
            .as_deref()
            .is_some_and(|popup| popup != "-")
    }

    pub(super) fn set_close_ack(&mut self) {
        self.close_ack = Some(true);
    }

    /// The real `cad_status.popup` read after the close.
    pub(super) fn set_popup_after_close(&mut self, detail: String) {
        if detail != "-" {
            self.fail("popup_after_close", &format!("expected -, read {detail}"));
        }
        self.popup_after_close = Some(detail);
    }

    /// Record a step that could not do what it says, so the summary cannot read
    /// as a pass.
    pub(super) fn fail(&mut self, step: &str, detail: &str) {
        self.failed.push(format!("{step}={}", flat(detail)));
    }

    /// The single summary line body (the caller prepends the `[probe] ui-dialogs:
    /// t=<n>s summary ` prefix).
    pub(super) fn summary(&self, hold: u64) -> String {
        let mut verified: Vec<String> = Vec::new();
        if self.palette_present == Some(true) {
            verified.push("properties_palette_mounted".into());
        }
        if self.selection_rows_complete && self.selection_count == Some(1) {
            verified.push("selection_rows".into());
        }
        if self.dist_command_logged == Some(true) {
            verified.push("commandline_submitted".into());
        }
        if self.dist_value.is_some() {
            verified.push("commandline_dist_result".into());
        }
        if self.popup_opened() {
            verified.push("options_dialog_open".into());
        }
        if self.popup_xid.is_some() {
            verified.push("options_dialog_xid".into());
        }
        if self.popup_after_close.as_deref() == Some("-") {
            verified.push("options_dialog_closed".into());
        }
        format!(
            "ui_dialogs_trigger=probe_driver_not_a_real_click ui_dialogs_synthetic=1 \
             document={} properties_palette={} palette_canvas={} main_xid={} selection_count={} \
             commandlog_entries={} commandlog_dist_command={} commandlog_dist_value={} \
             options_popup={} popup_xid={} popup_native_handle={} popup_close_ack={} \
             popup_after_close={} ui_dialogs_verified={} ui_dialogs_hold_s={hold} \
             ui_dialogs_failed={}",
            text(&self.document_label),
            flag(self.palette_present),
            text(&self.palette_canvas),
            text(&self.main_xid),
            self.selection_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "-".to_string()),
            self.command_log_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "-".to_string()),
            flag(self.dist_command_logged),
            text(&self.dist_value),
            text(&self.options_popup),
            text(&self.popup_xid),
            self.popup_handle.unwrap_or("-"),
            flag(self.close_ack),
            text(&self.popup_after_close),
            joined_or_dash(&verified),
            joined_or_dash(&self.failed),
        )
    }
}

pub(super) fn lock_evidence(evidence: &Arc<Mutex<Evidence>>) -> MutexGuard<'_, Evidence> {
    evidence
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Decode a JSON-quoted `eval_with_callback` payload into the raw string.
pub(super) fn decode_payload(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
    }
    trimmed.to_string()
}

/// Collapse whitespace so one log line stays a tokenizable field list.
pub(super) fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("_")
}

fn text(value: &Option<String>) -> String {
    value
        .as_deref()
        .map(flat)
        .unwrap_or_else(|| "-".to_string())
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    }
}

fn joined_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(",")
    }
}

/// `label xid=0x…` — the XID half of the main-window line, same shape as the
/// popup evidence line so one script can parse both.
pub(super) fn xid_detail(label: &str, xid: u64) -> String {
    format!("{label} xid=0x{xid:x}")
}

/// The `cad_status.selection` facts the log must carry: the count, the row
/// groups that really exist, and one `key=value` pair per key the palette shows.
pub(super) fn selection_detail(selection: &CadSelection) -> String {
    let mut groups: Vec<String> = Vec::new();
    for row in &selection.rows {
        if !groups.iter().any(|group| group == &row.group) {
            groups.push(row.group.clone());
        }
    }
    let rows: Vec<String> = SELECTED_ROW_KEYS
        .iter()
        .map(
            |key| match selection.rows.iter().find(|row| row.key == *key) {
                Some(row) => format!(
                    "{key}={}",
                    row.value
                        .as_deref()
                        .map(flat)
                        .unwrap_or_else(|| "mixed".into())
                ),
                None => format!("{key}=-"),
            },
        )
        .collect();
    format!(
        "selection count={} groups={} rows={}",
        selection.count,
        joined_or_dash(&groups),
        rows.join(";")
    )
}

/// Whether every key the palette groups under identity/display was really read.
pub(super) fn selection_rows_complete(selection: &CadSelection) -> bool {
    SELECTED_ROW_KEYS
        .iter()
        .all(|key| selection.rows.iter().any(|row| row.key == *key))
}

/// `cad_status.popup` as `kind:label:modal`, or `-` when it is `null`. Reuses the
/// popup driver's formatter so both drivers report the block identically.
pub(super) fn popup_detail(status: &CadStatus) -> String {
    super::popup::describe_status_popup(status)
}

/// The four-decimal display value of a measurement, matching the chrome's own
/// `command-log.mjs` decimal count.
pub(super) fn measurement_value(value: f64) -> String {
    format!("{value:.4}")
}

/// One `commandLog` entry as a single whitespace-free line.
pub(super) fn command_log_line(entry: &CadCommandLogEntry) -> String {
    let kind = match entry.kind {
        CadCommandLogKind::Command => "command",
        CadCommandLogKind::Result => "result",
        CadCommandLogKind::Error => "error",
    };
    let (algorithm, value, unit, precision, points) = match entry.measurement.as_ref() {
        Some(measurement) => (
            flat(&measurement.algorithm),
            measurement_value(measurement.value),
            measurement
                .unit
                .as_deref()
                .map(flat)
                .unwrap_or_else(|| "-".to_string()),
            measurement
                .precision
                .as_deref()
                .map(flat)
                .unwrap_or_else(|| "-".to_string()),
            measurement
                .points
                .iter()
                .map(|point| format!("{:.3},{:.3},{:.3}", point[0], point[1], point[2]))
                .collect::<Vec<_>>()
                .join(";"),
        ),
        None => (
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        ),
    };
    let point = match entry.point.as_ref() {
        Some(point) => format!(
            "{:.3},{:.3},{:.3}@{}#{}",
            point.position[0],
            point.position[1],
            point.position[2],
            flat(point.space),
            point.snap.map(flat).unwrap_or_else(|| "-".to_string()),
        ),
        None => "-".to_string(),
    };
    format!(
        "commandlog seq={} kind={} command={} code={} algorithm={} value={} unit={} precision={} \
         points={} point={}",
        entry.sequence,
        kind,
        entry
            .command
            .as_deref()
            .map(flat)
            .unwrap_or_else(|| "-".to_string()),
        entry
            .code
            .as_deref()
            .map(flat)
            .unwrap_or_else(|| "-".to_string()),
        algorithm,
        value,
        unit,
        precision,
        points,
        point,
    )
}
