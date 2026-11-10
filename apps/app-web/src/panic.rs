//! **Wasm panic reporting**: turn a Rust panic into a structured JSON report the
//! DOM can show.
//!
//! A panic inside the `dom-canvas` wasm session (an import that unwraps, an
//! allocation failure in a `Vec::with_capacity`, a debug assertion in a dependency)
//! traps the module: the JS call rejects with a `RuntimeError: unreachable` whose
//! message names no Rust location and carries no Rust message. The React chrome can
//! only show what reaches it, so this module installs a `std::panic::set_hook` hook
//! that **reports the panic before the trap happens** (the panic still traps — the
//! hook does not resume, abort early, or pretend the failure was handled).
//!
//! # The payload contract
//!
//! [`panic_report_json`] is the single wire shape every sink below carries. It is a
//! plain formatter (no wasm type in its signature) so `cargo test -p app-web`
//! asserts it on the host, not only in a browser:
//!
//! ```json
//! {"message": "…", "location": "src/live.rs:123:7", "stack": "Error: …\n    at …"}
//! ```
//!
//! `location` and `stack` are **omitted** when unknown — never an empty string that
//! would render as a blank dialog field. `message` is always present; a panic with
//! no payload string reports `"panic without a message"` rather than an empty line.
//!
//! # Delivery order (never panicking in the hook itself)
//!
//! 1. `globalThis.__yacr_report_panic(json)` when the chrome installed a callable
//!    bridge (the richest sink: it can route into React state immediately);
//! 2. else `globalThis.__YACR_PANIC_LOG__` (an `Array` the bootstrap pre-creates) —
//!    the JSON is parsed back into an **object** so the chrome drains an array of
//!    reports, not of strings. A panic before the chrome mounts lands here;
//! 3. always `console.error("yacr: wasm panic: …")`, so DevTools shows the same
//!    report even when no bridge and no log exist.
//!
//! A bridge that throws falls through to (2) and (3): one broken sink must not lose
//! the report. Nothing here creates a second failure mode of its own.

use serde_json::{Map, Value};

/// The message reported for a panic whose payload is not a string.
///
/// `panic_any(42)` and a payload type that is neither `&str` nor `String` are real
/// panics with no printable message; this names that honestly instead of reporting
/// an empty message that reads like a formatting bug.
pub const NO_MESSAGE: &str = "panic without a message";

/// Format the panic report JSON: `{message, location?, stack?}`.
///
/// Target-independent on purpose (no `js_sys`/`web_sys` type in the signature), so
/// the exact payload the JS bridge and the React store read is unit-tested by
/// `cargo test -p app-web` on the host. Absent facts are **omitted** rather than
/// serialized as `null` or `""`.
pub fn panic_report_json(message: &str, location: Option<&str>, stack: Option<&str>) -> String {
    let mut report = Map::new();
    report.insert("message".to_string(), Value::String(message.to_string()));
    if let Some(location) = location {
        report.insert("location".to_string(), Value::String(location.to_string()));
    }
    if let Some(stack) = stack {
        report.insert("stack".to_string(), Value::String(stack.to_string()));
    }
    Value::Object(report).to_string()
}

/// Install the wasm panic hook. Called once from the module's start function
/// (`yacr_wasm_start` in `lib.rs`), before any session export can run.
///
/// The hook reports and then lets the panic continue: the module still traps
/// exactly as it did before this hook existed. Reporting is best-effort by
/// construction — every JS call is fallible and every failure falls through to the
/// next sink — but the panic itself is never swallowed.
#[cfg(target_arch = "wasm32")]
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = panic_message(info.payload());
        let location = info.location().map(format_location);
        let stack = error_stack(&message);
        let report = panic_report_json(&message, location.as_deref(), stack.as_deref());
        report_panic(&report);
    }));
}

/// The readable message of a panic payload: `&str`, then `String`, else
/// [`NO_MESSAGE`].
#[cfg(target_arch = "wasm32")]
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        NO_MESSAGE.to_string()
    }
}

/// `file:line:col` — the same spelling rustc uses in its own diagnostics.
#[cfg(target_arch = "wasm32")]
fn format_location(location: &std::panic::Location<'_>) -> String {
    format!(
        "{}:{}:{}",
        location.file(),
        location.line(),
        location.column()
    )
}

/// A JS stack captured at the panic point.
///
/// `js-sys` 0.3.106 has no `Error::stack()` accessor (it ships only the
/// non-standard `stackTraceLimit` getter/setter), so the standard `stack` property
/// is read through `Reflect`. A browser without that property (or a non-string
/// value) is "no stack", not an error.
#[cfg(target_arch = "wasm32")]
fn error_stack(message: &str) -> Option<String> {
    use wasm_bindgen::JsValue;
    let error = js_sys::Error::new(message);
    js_sys::Reflect::get(&error, &JsValue::from_str("stack"))
        .ok()
        .and_then(|stack| stack.as_string())
        .filter(|stack| !stack.is_empty())
}

/// Deliver one report to the DOM, in the documented order (see the module docs).
///
/// Never panics: a missing global, a non-callable bridge, a bridge that throws and
/// a non-array log are all "skip this sink and try the next one".
#[cfg(target_arch = "wasm32")]
fn report_panic(report: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let global: JsValue = js_sys::global().into();
    let bridge = js_sys::Reflect::get(&global, &JsValue::from_str("__yacr_report_panic"))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok());
    if let Some(bridge) = bridge.as_ref() {
        // A throwing bridge must not hide the panic: fall through to the log and the
        // console line instead of losing the report.
        if bridge
            .call1(&JsValue::NULL, &JsValue::from_str(report))
            .is_ok()
        {
            log_to_console(report);
            return;
        }
    }
    push_to_log(&global, report);
    log_to_console(report);
}

/// Append the report to `globalThis.__YACR_PANIC_LOG__` as an **object**.
///
/// The bootstrap creates that array before the module is instantiated, so a panic
/// during module init is retained until the React chrome drains it. The JSON is
/// parsed back (`js_sys::JSON::parse`) so the store receives the same structured
/// object a `yacr:panic` listener gets; if parsing fails the raw string is pushed —
/// still a report, never dropped.
#[cfg(target_arch = "wasm32")]
fn push_to_log(global: &wasm_bindgen::JsValue, report: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let Ok(log) = js_sys::Reflect::get(global, &JsValue::from_str("__YACR_PANIC_LOG__")) else {
        return;
    };
    if !js_sys::Array::is_array(&log) {
        return;
    }
    let log: js_sys::Array = log.unchecked_into();
    let value = js_sys::JSON::parse(report).unwrap_or_else(|_| JsValue::from_str(report));
    log.push(&value);
}

/// The unconditional sink: one prefixed console line DevTools shows even when no
/// bridge and no log exist.
#[cfg(target_arch = "wasm32")]
fn log_to_console(report: &str) {
    web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&format!(
        "yacr: wasm panic: {report}"
    )));
}

#[cfg(test)]
mod tests {
    use super::panic_report_json;

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("panic report must be valid JSON")
    }

    #[test]
    fn message_only_omits_location_and_stack() {
        let report = parse(&panic_report_json("boom", None, None));
        assert_eq!(report, serde_json::json!({ "message": "boom" }));
        assert!(report.get("location").is_none());
        assert!(report.get("stack").is_none());
    }

    #[test]
    fn message_and_location_omit_only_stack() {
        let report = parse(&panic_report_json(
            "boom",
            Some("apps/app-web/src/live.rs:42:9"),
            None,
        ));
        assert_eq!(
            report,
            serde_json::json!({
                "message": "boom",
                "location": "apps/app-web/src/live.rs:42:9",
            })
        );
        assert!(report.get("stack").is_none());
    }

    #[test]
    fn message_location_and_stack_are_all_carried() {
        let stack = "Error: boom\n    at open_bytes (app_web.js:1:1)";
        let report = parse(&panic_report_json(
            "boom",
            Some("src/page.rs:7:3"),
            Some(stack),
        ));
        assert_eq!(
            report,
            serde_json::json!({
                "message": "boom",
                "location": "src/page.rs:7:3",
                "stack": stack,
            })
        );
    }

    #[test]
    fn quotes_and_newlines_are_escaped_not_dropped() {
        let message = "he said \"stop\"\nand then\\";
        let json = panic_report_json(message, None, Some("a\nb"));
        // A single line of JSON: the newline is escaped, not emitted raw.
        assert!(!json.contains('\n'), "{json}");
        let report = parse(&json);
        assert_eq!(report["message"], message);
        assert_eq!(report["stack"], "a\nb");
    }
}
