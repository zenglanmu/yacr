//! `YACR_PROBE_UI_DIALOGS`: drives the **React chrome** through the same JS the
//! UI itself runs, so an external window grab can photograph the Options dialog,
//! the populated Properties palette and the command-line echo.
//!
//! ## Why this driver exists
//!
//! OS-level input injection (uinput/XTest) does not reach this Wayland/XWayland
//! session, so a real mouse or keyboard cannot be used to produce evidence. The
//! repo's sanctioned path is a synthetic `Webview::eval` script through the real
//! webview — the same technique as [`super::input`] (`SYNTHETIC_INPUT_JS`) and
//! [`super::tool`] (public `cad_status`/`cad_command` calls plus synthetic DOM
//! pointer events).
//!
//! What is different here: the keyboard and the command line are driven through
//! the **handlers the production chrome installs**, not by calling the IPC
//! directly.
//!
//! * `Ctrl+1` is a real `KeyboardEvent('keydown')` on `window`; the app's global
//!   handler (`apps/app-linux/ui/src/App.jsx`, `if (event.ctrlKey && event.key
//!   === '1') { … setPropertiesOpen(…) }`) toggles the docked palette.
//! * `DIST`/`OPTIONS` are typed into `#cad-command` through the
//!   `HTMLInputElement.prototype` value setter plus the `input` event its
//!   `onChange` listens to, then submitted with the `KeyboardEvent('keydown')`
//!   `Enter` that `CommandLine.jsx`'s own `onKeyDown` (`handleCommandKey`) turns
//!   into `shell.submitCommand`. `OPTIONS` is therefore intercepted by the chrome
//!   (`apps/app-linux/ui/src/options.mjs` `isOptionsCommand`) and never reaches
//!   the router — the same path a user's Enter takes.
//! * The two `DIST` picks and the selection click are synthetic `PointerEvent`s
//!   on `.native-input` using [`super::tool::tool_click_world_js`].
//!
//! ## Files
//!
//! * this module: env parsing, the timeline, the scheduler and the eval/read
//!   plumbing;
//! * [`super::ui_dialogs_js`]: the JS blobs and their source assertions;
//! * [`super::ui_dialogs_steps`]: one function per scheduled second;
//! * [`super::ui_dialogs_report`]: the accumulated reads, the log-line bodies and
//!   the summary.
//!
//! ## Timeline (seconds after driver start; `YACR_PROBE_UI_DIALOGS_AT`, default 3)
//!
//! * `at`: read the main window's native handle (X11 XID / Wayland surface), wait
//!   for the chrome to be mounted, then dispatch the `Ctrl+1` keydown.
//! * `at+2`: read `.properties` / `.native-input` back from the DOM and log
//!   whether the palette is really mounted.
//! * `at+5`: synthetic click on the canvas at a **world** point of the opened
//!   drawing (`fixtures/dwg/synthetic-four-lines.dwg`'s LINE at `x = 1`, hit at
//!   its midpoint `(1.0, 0.5)`), gated on the real `cad_status` (a drawing that
//!   is still loading is reported, not clicked into).
//! * `at+7`: read the real `cad_status.selection` and log `count` plus the
//!   `id`/`type`/`layer`/`color`/`linetype`/`lineweight`/`transparency` row
//!   key/value pairs and the row groups, after the click template's own
//!   read-back (`window.__yacr_tool`).
//! * `at+12`: type `DIST` + Enter through the command field.
//! * `at+15` and `at+17`: the two measurement picks (`(0,0)` and `(3,1)`).
//! * `at+20`: read the real `cad_status.commandLog` and log its newest entries
//!   (sequence, kind, command, measurement algorithm/value/unit/precision, the
//!   picked points).
//! * `at+24`: type `OPTIONS` + Enter through the same command field; the chrome
//!   opens the dialog over the real `cad_open_popup` path.
//! * `at+27` (and one retry at `at+30`): log the popup's native handle with
//!   [`super::popup::log_popup_native_handle`], i.e. the existing
//!   `[probe] popup evidence: window xid=0x…` line the external
//!   `apps/app-linux/scripts/x11-window-grab.c <xid> <out.ppm>` greps — plus the
//!   real `cad_status.popup` block.
//! * `at+24+hold`: close the dialog through the same host path `cad_close_popup`
//!   runs ([`crate::host::close_popup_for_probe`]) and log its ack. `hold` is
//!   `YACR_PROBE_UI_DIALOGS_HOLD` seconds (default **30**, floored at
//!   [`MIN_HOLD_SECONDS`]) so the external grab has a window while it is open.
//! * `+2` and `+4`: read `cad_status.popup` back (must be `-`) and print one
//!   summary line.
//!
//! Every line starts with `[probe] ui-dialogs:` so external scripts can grep one
//! prefix. `t=<n>s` is the **scheduled** second of the step, not a measurement.
//!
//! ## Honest limits
//!
//! * The **trigger** of every step is the probe driver's synthetic event, not a
//!   real mouse or keyboard; the summary says
//!   `ui_dialogs_trigger=probe_driver_not_a_real_click` and
//!   `ui_dialogs_synthetic=1`. Everything downstream (palette mount, command
//!   funnel, popup window, popup close) is the production path.
//! * The pick coordinate is chosen for `fixtures/dwg/synthetic-four-lines.dwg`
//!   and the log prints the real `documentLabel` next to it; if a different
//!   drawing is open the click simply lands wherever that world point is, and
//!   the selection read reports what actually got selected (never the guess).
//! * The popup XID line exists only on X11/XWayland. On native Wayland the popup
//!   is a compositor-owned toplevel, so the handle line reports its `wl_surface`
//!   pointer and no X11 grab is claimed.
//! * A screenshot is only the window's own pixels. Nothing here is a
//!   visual-correctness, precision, vendor or cross-platform claim.
//! * The driver leaves the drawing unchanged (a `DIST` measurement creates no
//!   entity) but leaves a **selection** behind (the click-select in step 2), and
//!   the command echo it produced stays in `cad_status.commandLog`.
//!
//! ## Failure behaviour
//!
//! A step that cannot do what it says logs `[probe] ui-dialogs: t=<n>s FAILED:
//! …` at once and records itself in the summary's `ui_dialogs_failed` list. The
//! summary always prints, with only the parts that really happened in
//! `ui_dialogs_verified` — a run that stopped early can never read as a pass.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ui_dialogs_report::Evidence;
use super::{ui_dialogs_js, ui_dialogs_steps as steps};

/// Default timeline start (seconds after driver start). Small, because this
/// driver only needs an already opened drawing; configurable because other
/// drivers may share the evidence session.
const DEFAULT_START_SECONDS: u64 = 3;
/// Default hold (seconds) between the Options dialog opening and its close. The
/// hold is for the **external** grab; it is not an internal sleep to optimise
/// away.
const DEFAULT_HOLD_SECONDS: u64 = 30;
/// Floor for `YACR_PROBE_UI_DIALOGS_HOLD`: the popup handle is read at `at+27`
/// and retried at `at+30` after the open at `at+24`, so a shorter hold could
/// close the dialog before the handle was ever read.
const MIN_HOLD_SECONDS: u64 = 10;
/// How long one `eval_wait` waits for the webview to deliver its callback.
const EVAL_TIMEOUT: Duration = Duration::from_millis(2_000);
/// Chrome-mount wait: 8 attempts × 200 ms = 1.6 s, which is shorter than the
/// 2 s gap to the palette read, so a slow start cannot reorder the timeline.
const MOUNT_ATTEMPTS: u32 = 8;
const MOUNT_INTERVAL: Duration = Duration::from_millis(200);

/// Step labels. Every emitted line is `[probe] ui-dialogs: t=<n>s <label> …`, so
/// these constants are the grep anchors and are asserted in the tests.
pub(super) const L_PROPERTIES_KEY: &str = "properties ctrl+1 keydown";
pub(super) const L_PROPERTIES_READ: &str = "properties";
pub(super) const L_MAIN_HANDLE: &str = "main window handle";
pub(super) const L_SELECT_CLICK: &str = "select click";
pub(super) const L_SELECT_READ: &str = "selection";
pub(super) const L_COMMAND_DIST: &str = "command echo DIST";
pub(super) const L_COMMAND_OPTIONS: &str = "command echo OPTIONS";
pub(super) const L_PICK_A: &str = "pick 1";
pub(super) const L_PICK_B: &str = "pick 2";
pub(super) const L_COMMAND_LOG: &str = "commandlog";
pub(super) const L_POPUP_HANDLE: &str = "options popup handle";
pub(super) const L_POPUP_RETRY: &str = "options popup handle retry";
pub(super) const L_CLOSE: &str = "options close";
pub(super) const L_CLOSE_READ: &str = "options popup after close";
pub(super) const L_SUMMARY: &str = "summary";

/// Whether the UI-dialogs driver is enabled (`YACR_PROBE_UI_DIALOGS`).
///
/// Evidence-only, default OFF, independent of every other probe driver.
pub fn ui_dialogs_enabled() -> bool {
    enabled_value(std::env::var("YACR_PROBE_UI_DIALOGS").ok().as_deref())
}

/// The env-value predicate, split out so it is testable without touching the
/// process environment.
fn enabled_value(raw: Option<&str>) -> bool {
    matches!(raw, Some("1") | Some("true") | Some("yes"))
}

/// The scheduled seconds of one run. Pure so the whole timeline is testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Timeline {
    start: u64,
    hold: u64,
}

impl Timeline {
    pub(super) const fn new(start: u64, hold: u64) -> Self {
        Self { start, hold }
    }

    pub(super) fn properties_key(&self) -> u64 {
        self.start
    }

    pub(super) fn properties_read(&self) -> u64 {
        self.start + 2
    }

    pub(super) fn select_click(&self) -> u64 {
        self.start + 5
    }

    pub(super) fn select_read(&self) -> u64 {
        self.start + 7
    }

    pub(super) fn command_dist(&self) -> u64 {
        self.start + 12
    }

    pub(super) fn pick_a(&self) -> u64 {
        self.start + 15
    }

    pub(super) fn pick_b(&self) -> u64 {
        self.start + 17
    }

    pub(super) fn command_log_read(&self) -> u64 {
        self.start + 20
    }

    pub(super) fn options(&self) -> u64 {
        self.start + 24
    }

    pub(super) fn popup_handle(&self) -> u64 {
        self.start + 27
    }

    pub(super) fn popup_handle_retry(&self) -> u64 {
        self.start + 30
    }

    pub(super) fn close(&self) -> u64 {
        self.start + 24 + self.hold
    }

    pub(super) fn close_read(&self) -> u64 {
        self.close() + 2
    }

    pub(super) fn summary(&self) -> u64 {
        self.close() + 4
    }

    pub(super) fn hold(&self) -> u64 {
        self.hold
    }

    /// One whitespace-free line naming every scheduled second, so the runtime
    /// log states the timeline the run actually used (the env vars override it).
    fn describe(&self) -> String {
        format!(
            "properties_key={}s properties_read={}s select_click={}s select_read={}s \
             command_DIST={}s pick1={}s pick2={}s commandlog={}s options={}s popup_handle={}s \
             popup_handle_retry={}s options_close={}s options_popup_after_close={}s \
             summary={}s hold={}s",
            self.properties_key(),
            self.properties_read(),
            self.select_click(),
            self.select_read(),
            self.command_dist(),
            self.pick_a(),
            self.pick_b(),
            self.command_log_read(),
            self.options(),
            self.popup_handle(),
            self.popup_handle_retry(),
            self.close(),
            self.close_read(),
            self.summary(),
            self.hold,
        )
    }
}

/// Parse `YACR_PROBE_UI_DIALOGS_AT`. An unset/blank value uses the default; an
/// unparseable value is reported (never silently accepted) and falls back to the
/// default. `0` is legal: the first step then fires immediately.
fn parse_start_seconds(raw: Option<&str>) -> (u64, Option<String>) {
    match raw.map(str::trim) {
        None | Some("") => (DEFAULT_START_SECONDS, None),
        Some(text) => match text.parse::<u64>() {
            Ok(seconds) => (seconds, None),
            Err(_) => (
                DEFAULT_START_SECONDS,
                Some(format!(
                    "YACR_PROBE_UI_DIALOGS_AT={text:?} is not an integer number of seconds; \
                     using {DEFAULT_START_SECONDS}"
                )),
            ),
        },
    }
}

/// Parse `YACR_PROBE_UI_DIALOGS_HOLD`, floored at [`MIN_HOLD_SECONDS`] so the
/// popup handle and the external grab always have a window.
fn parse_hold_seconds(raw: Option<&str>) -> (u64, Option<String>) {
    match raw.map(str::trim) {
        None | Some("") => (DEFAULT_HOLD_SECONDS, None),
        Some(text) => match text.parse::<u64>() {
            Ok(seconds) if seconds < MIN_HOLD_SECONDS => (
                MIN_HOLD_SECONDS,
                Some(format!(
                    "hold {seconds}s is below the {MIN_HOLD_SECONDS}s floor (the popup handle \
                     read and the external grab need the dialog open); using {MIN_HOLD_SECONDS}s"
                )),
            ),
            Ok(seconds) => (seconds, None),
            Err(_) => (
                DEFAULT_HOLD_SECONDS,
                Some(format!(
                    "YACR_PROBE_UI_DIALOGS_HOLD={text:?} is not an integer number of seconds; \
                     using {DEFAULT_HOLD_SECONDS}"
                )),
            ),
        },
    }
}

/// Schedule the UI-dialogs driver if enabled. Never panics on spawn failure (a
/// failed spawn is logged; the run continues unchanged).
///
/// The thread only dispatches `Webview::eval` scripts and reads the real
/// `cad_status`/popup state on the main thread. It changes no command, session
/// or render logic.
pub fn maybe_spawn_ui_dialogs(app: &tauri::AppHandle, webview: Option<&tauri::Webview>) {
    if !ui_dialogs_enabled() {
        return;
    }
    let (start_at, start_notice) =
        parse_start_seconds(std::env::var("YACR_PROBE_UI_DIALOGS_AT").ok().as_deref());
    let (hold, hold_notice) =
        parse_hold_seconds(std::env::var("YACR_PROBE_UI_DIALOGS_HOLD").ok().as_deref());
    for notice in start_notice.into_iter().chain(hold_notice) {
        eprintln!("[probe] ui-dialogs: {notice}");
    }
    let Some(webview) = webview else {
        eprintln!(
            "[probe] ui-dialogs: no child webview; the Options/Properties/command-echo pass is \
             skipped (nothing faked)"
        );
        return;
    };
    let timeline = Timeline::new(start_at, hold);
    eprintln!(
        "[probe] ui-dialogs: enabled (YACR_PROBE_UI_DIALOGS); synthetic DOM/keyboard events \
         through the real webview, NOT a real mouse or keyboard; every step line is \
         `[probe] ui-dialogs: t=<n>s …`"
    );
    eprintln!("[probe] ui-dialogs: timeline {}", timeline.describe());
    let app = app.clone();
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-ui-dialogs".into())
        .spawn(move || run(app, webview, timeline))
    {
        eprintln!("[probe] ui-dialogs: failed to spawn: {error}");
    }
}

/// One line with the driver's distinctive prefix and the scheduled second.
pub(super) fn step_line(second: u64, detail: &str) -> String {
    format!("[probe] ui-dialogs: t={second}s {detail}")
}

pub(super) fn log(second: u64, detail: &str) {
    eprintln!("{}", step_line(second, detail));
}

/// Sleep until `second` after `start`, then run `action`. A failure is logged
/// here as `t=<n>s FAILED: …`; a successful step logs its own line.
fn schedule_step(start: &Instant, second: u64, action: impl FnOnce() -> Result<(), String>) {
    let target = Duration::from_secs(second);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    if let Err(error) = action() {
        log(second, &format!("FAILED: {error}"));
    }
}

/// Run one task on the GTK main thread and wait for its result.
///
/// Every CAD-session read and every popup accessor must run there (the session
/// is a main-thread-local value). Called only from the spawned driver thread, so
/// the blocking receive cannot deadlock the thread it is waiting for.
pub(super) fn on_main_thread<T, F>(app: &tauri::AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(task());
    })
    .map_err(|error| format!("ui-dialogs.main_thread_failed: {error}"))?;
    receiver
        .recv()
        .map_err(|_| "ui-dialogs.main_thread_failed: the main thread dropped it".to_string())?
}

/// Evaluate `js` in the real webview and wait for its returned string.
///
/// `eval_with_callback` serializes the script's return value into a JSON string;
/// the wait is bounded by [`EVAL_TIMEOUT`] so a page that never answers cannot
/// hang the evidence run. Blocking here (rather than logging from the callback)
/// keeps the steps in timeline order, which is what the external grabs rely on.
pub(super) fn eval_wait(webview: &tauri::Webview, js: &str) -> Result<String, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    webview
        .eval_with_callback(js, move |payload| {
            let _ = sender.send(payload);
        })
        .map_err(|error| format!("eval failed: {error}"))?;
    let payload = receiver
        .recv_timeout(EVAL_TIMEOUT)
        .map_err(|error| format!("the webview did not answer within {EVAL_TIMEOUT:?}: {error}"))?;
    Ok(super::ui_dialogs_report::decode_payload(&payload))
}

/// Wait (bounded) until the React chrome has rendered its command field, so the
/// `Ctrl+1` keydown cannot be dispatched into a page with no listener yet.
pub(super) fn wait_for_chrome(webview: &tauri::Webview) -> Result<(), String> {
    for attempt in 1..=MOUNT_ATTEMPTS {
        match eval_wait(webview, ui_dialogs_js::MOUNTED_JS) {
            Ok(state) if state == "mounted" => return Ok(()),
            Ok(state) => {
                if attempt == MOUNT_ATTEMPTS {
                    return Err(format!(
                        "the React chrome is not mounted after {MOUNT_ATTEMPTS} attempts \
                         (last={state})"
                    ));
                }
                std::thread::sleep(MOUNT_INTERVAL);
            }
            Err(error) => {
                if attempt == MOUNT_ATTEMPTS {
                    return Err(format!("the webview never answered: {error}"));
                }
                std::thread::sleep(MOUNT_INTERVAL);
            }
        }
    }
    Err("the React chrome never mounted".to_string())
}

/// The whole bounded timeline.
fn run(app: tauri::AppHandle, webview: tauri::Webview, timeline: Timeline) {
    let start = Instant::now();
    let evidence = Arc::new(Mutex::new(Evidence::default()));

    // ---- at: main window handle + Ctrl+1 through the app's global handler ----
    let second = timeline.properties_key();
    {
        let app = app.clone();
        let webview = webview.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::properties_key(&app, &webview, &evidence, second)
        });
    }

    // ---- at+2: read the palette back from the DOM ----
    let second = timeline.properties_read();
    {
        let webview = webview.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::properties_read(&webview, &evidence, second)
        });
    }

    // ---- at+5: one synthetic canvas click on the fixture's LINE at x = 1 ----
    let second = timeline.select_click();
    {
        let app = app.clone();
        let webview = webview.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::select_click(&app, &webview, &evidence, second)
        });
    }

    // ---- at+7: the real cad_status.selection ----
    let second = timeline.select_read();
    {
        let app = app.clone();
        let webview = webview.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::select_read(&app, &webview, &evidence, second)
        });
    }

    // ---- at+12: type DIST + Enter into the real command field ----
    let second = timeline.command_dist();
    {
        let webview = webview.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::type_command(&webview, &evidence, second, "DIST", L_COMMAND_DIST)
        });
    }

    // ---- at+15 / at+17: the two measurement picks ----
    for (index, (world_x, world_y, pointer_label, step)) in steps::PICKS.into_iter().enumerate() {
        let second = if index == 0 {
            timeline.pick_a()
        } else {
            timeline.pick_b()
        };
        let webview = webview.clone();
        schedule_step(&start, second, move || {
            steps::pick(&webview, second, (world_x, world_y), pointer_label, step)
        });
    }

    // ---- at+20: the real cad_status.commandLog ----
    let second = timeline.command_log_read();
    {
        let app = app.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::command_log(&app, &evidence, second)
        });
    }

    // ---- at+24: type OPTIONS + Enter; the chrome intercepts it ----
    let second = timeline.options();
    {
        let webview = webview.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::type_options(&webview, &evidence, second)
        });
    }

    // ---- at+27 (+ one retry at at+30): the popup window's native handle ----
    let second = timeline.popup_handle();
    {
        let app = app.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::popup_handle(&app, &evidence, second, false)
        });
    }
    let second = timeline.popup_handle_retry();
    {
        let app = app.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::popup_handle(&app, &evidence, second, true)
        });
    }

    // ---- at+24+hold: the same host close path cad_close_popup runs ----
    let second = timeline.close();
    {
        let app = app.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::close(&app, &evidence, second)
        });
    }

    // ---- close+2: the real cad_status.popup must be back to null ----
    let second = timeline.close_read();
    {
        let app = app.clone();
        let evidence = evidence.clone();
        schedule_step(&start, second, move || {
            steps::popup_after_close(&app, &evidence, second)
        });
    }

    // ---- close+4: one summary line ----
    let second = timeline.summary();
    let evidence = evidence.clone();
    let hold = timeline.hold();
    schedule_step(&start, second, move || {
        steps::summary(&evidence, second, hold)
    });
}

#[cfg(test)]
mod tests;
