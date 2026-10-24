//! Can this engine run the CAD session **isolated in a Worker**?
//!
//! ADR 0024 stage ⑧ splits parsing/rendering into a Worker. Whether that is
//! possible is a **browser capability**, not a design choice, and the two engines
//! this project targets disagree — measured on 2026-10-24:
//!
//! | engine | `OffscreenCanvas.getContext("webgl2")` **in a Worker** |
//! |---|---|
//! | Chromium 141 | a context (verified with an ANGLE/Intel adapter: clear + `readPixels` correct) |
//! | WebKitGTK 2.52.6 | **`null`** (verified) |
//!
//! So on WebKitGTK worker-side rendering is impossible. This module exists to say
//! exactly that, in a machine-readable form, instead of attempting it and showing
//! a blank canvas: the host reads [`Capability`] and, when isolation is not
//! available, keeps the main-thread carrier **and states the reason in its status
//! line** (`isolation=main-thread (no WebGL2 in worker)`).
//!
//! The split into a pure model ([`Capability`], compiled on every target so its
//! matrix is unit tested by `cargo test -p app-web`) and a wasm-only probe
//! ([`probe`]) mirrors `crate::budget`: the reasoning is testable on the host, the
//! browser fact is only observable in a browser.

/// Where the capability probe ran.
///
/// The distinction matters: the probe creates an `OffscreenCanvas` and asks it for a
/// WebGL2 context, and that question is only meaningful **inside a worker** — the
/// measured WebKitGTK limitation is specifically "`getContext("webgl2")` returns
/// `null` for an `OffscreenCanvas` in a Worker", not a claim about every canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// The page's main thread (a `Window` global).
    Window,
    /// A Worker global (`DedicatedWorkerGlobalScope`), where the isolated CAD
    /// session would run.
    Worker,
}

impl Context {
    /// Every context, in declaration order (used by the exactness tests).
    pub const ALL: [Self; 2] = [Self::Window, Self::Worker];

    /// The stable lowercase token reported in the capability JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Worker => "worker",
        }
    }
}

impl std::fmt::Display for Context {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The stable `isolation` tokens a status carries.
pub const ISOLATION_WORKER: &str = "worker";
/// The main-thread carrier (the verified `dom-canvas` path).
pub const ISOLATION_MAIN_THREAD: &str = "main-thread";

/// Why isolation is **not** available: the measured WebKitGTK 2.52.6 behaviour.
///
/// A host prints this verbatim next to `isolation=main-thread`, so the fallback can
/// never look like an unexplained choice.
pub const REASON_NO_WEBGL2_IN_WORKER: &str = "no WebGL2 in worker";
/// Why isolation is not available from the page: the probe must run in a Worker.
pub const REASON_NOT_A_WORKER: &str = "capability probe ran on the page's main thread";
/// Why isolation **is** available (Chromium 141).
pub const REASON_WORKER_AVAILABLE: &str = "WebGL2 available in a worker";

/// What one JS context can do.
///
/// Only two facts, deliberately: where the probe ran, and whether an
/// `OffscreenCanvas` in that context yields a WebGL2 context. Everything else
/// (whether the API exists, whether a canvas can be transferred) is checked at the
/// call site that needs it, so this report never claims more than it measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    /// The context the probe ran in.
    pub context: Context,
    /// Whether `new OffscreenCanvas(1, 1).getContext("webgl2")` returned a context
    /// in this context. `false` on the main thread: the question is not answered
    /// there.
    pub webgl2_in_worker: bool,
}

impl Capability {
    /// The capability of a context, from its measured WebGL2 answer.
    pub fn new(context: Context, webgl2_in_worker: bool) -> Self {
        // The main thread never answers the worker question: a context created
        // there says nothing about a worker (that is exactly the WebKitGTK case —
        // the page has WebGL2, the worker does not).
        let webgl2_in_worker = webgl2_in_worker && context == Context::Worker;
        Self {
            context,
            webgl2_in_worker,
        }
    }

    /// Whether the session can run isolated in a Worker here.
    pub fn ok(self) -> bool {
        self.context == Context::Worker && self.webgl2_in_worker
    }

    /// The `isolation` token this context can actually deliver.
    ///
    /// `worker` only when the probe ran in a Worker *and* that Worker has WebGL2;
    /// everything else is the main-thread carrier, which is the already-verified
    /// path.
    pub fn isolation(self) -> &'static str {
        if self.ok() {
            ISOLATION_WORKER
        } else {
            ISOLATION_MAIN_THREAD
        }
    }

    /// The stable reason phrase: why isolation is unavailable, or that it is.
    pub fn reason(self) -> &'static str {
        if self.ok() {
            REASON_WORKER_AVAILABLE
        } else if self.context == Context::Worker {
            REASON_NO_WEBGL2_IN_WORKER
        } else {
            REASON_NOT_A_WORKER
        }
    }

    /// The report a host reads: `{ok, isolation, reason, context, webgl2_in_worker}`.
    ///
    /// Hand-built rather than `serde_json` so the model stays dependency-free and
    /// therefore unit tested on the host target (where this crate has no
    /// `serde_json` edge). `reason` is a compile-time constant from this module, but
    /// it is escaped anyway so the JSON can never be broken by a later edit.
    pub fn to_json(self) -> String {
        format!(
            "{{\"ok\":{},\"isolation\":\"{}\",\"reason\":\"{}\",\"context\":\"{}\",\
             \"webgl2_in_worker\":{}}}",
            self.ok(),
            self.isolation(),
            escape_json(self.reason()),
            self.context.as_str(),
            self.webgl2_in_worker
        )
    }
}

/// Escape the two characters that could terminate/fracture a JSON string.
///
/// The inputs are this module's own constants; this is a guard against a future
/// reason gaining a quote, not an encoder for arbitrary text.
fn escape_json(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Probe the **current** JS context, in a browser.
///
/// The WebGL2 answer is obtained exactly the way the limitation was measured: a 1×1
/// `OffscreenCanvas` and `getContext("webgl2")`. On the page's main thread the
/// question is not asked at all (see [`Capability::new`]), so the page cannot
/// mistake "the page has WebGL2" for "a worker has WebGL2".
#[cfg(target_arch = "wasm32")]
pub(crate) fn probe() -> Capability {
    let context = current_context();
    let webgl2 = context == Context::Worker && offscreen_webgl2_available();
    Capability::new(context, webgl2)
}

/// Whether this module is running in a Worker's global scope.
///
/// A `Window` global always exposes `document`; a worker global never does. That is
/// a property of the environment rather than of a type name, so it needs no
/// `instanceof` against a constructor the global scope may not have.
///
/// An unreadable global is treated as **the page**: guessing "worker" there would
/// let a main-thread session try to own a transferred canvas, which is exactly the
/// second-owner situation the single-session rule forbids.
#[cfg(target_arch = "wasm32")]
pub(crate) fn current_context() -> Context {
    let document = js_sys::Reflect::get(
        &js_sys::global(),
        &wasm_bindgen::JsValue::from_str("document"),
    );
    match document {
        Ok(value) if value.is_undefined() || value.is_null() => Context::Worker,
        _ => Context::Window,
    }
}

/// The capability report for the **current** JS context, as JSON
/// (`worker_capability()`).
///
/// Called from the page it answers "the page's main thread" (isolation
/// `main-thread`); called from the worker it answers the measured question and, on
/// WebKitGTK 2.52.6, reports `isolation: "main-thread"` with the reason
/// `no WebGL2 in worker` instead of attempting worker rendering.
#[cfg(target_arch = "wasm32")]
pub(crate) fn report() -> String {
    probe().to_json()
}

/// Whether an `OffscreenCanvas` created in **this** context can yield a WebGL2
/// context.
///
/// This is the measured discriminator, not a feature sniff: WebKitGTK 2.52.6 has
/// the `OffscreenCanvas` API and still returns `null` here inside a Worker.
#[cfg(target_arch = "wasm32")]
fn offscreen_webgl2_available() -> bool {
    let Ok(canvas) = web_sys::OffscreenCanvas::new(1, 1) else {
        return false;
    };
    // `Ok(Some(_))` is the only success: `Ok(None)` is the "not supported" answer
    // (WebKitGTK in a worker) and `Err` is a thrown exception, i.e. also no context.
    matches!(canvas.get_context("webgl2"), Ok(Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_worker_with_webgl2_reports_worker_isolation() {
        // The full matrix. The `window` rows are the load-bearing ones: the page has
        // WebGL2 in both engines, so a probe that answered "yes" on the main thread
        // would wrongly promise isolation on WebKitGTK.
        let cases = [
            (Context::Window, false, false, ISOLATION_MAIN_THREAD),
            (Context::Window, true, false, ISOLATION_MAIN_THREAD),
            (Context::Worker, false, false, ISOLATION_MAIN_THREAD),
            (Context::Worker, true, true, ISOLATION_WORKER),
        ];
        for (context, webgl2, expected_ok, expected_isolation) in cases {
            let capability = Capability::new(context, webgl2);
            assert_eq!(
                capability.ok(),
                expected_ok,
                "{context:?}/{webgl2}: ok classification"
            );
            assert_eq!(
                capability.isolation(),
                expected_isolation,
                "{context:?}/{webgl2}: isolation token"
            );
        }
    }

    #[test]
    fn a_main_thread_probe_never_claims_webgl2_in_a_worker() {
        let page = Capability::new(Context::Window, true);
        assert!(!page.webgl2_in_worker);
        assert_eq!(page.reason(), REASON_NOT_A_WORKER);
    }

    #[test]
    fn webkitgtk_worker_case_is_reported_as_main_thread_with_the_measured_reason() {
        // The exact WebKitGTK 2.52.6 measurement: a worker, and a null context.
        let webkitgtk = Capability::new(Context::Worker, false);
        assert!(!webkitgtk.ok());
        assert_eq!(webkitgtk.isolation(), ISOLATION_MAIN_THREAD);
        assert_eq!(webkitgtk.reason(), REASON_NO_WEBGL2_IN_WORKER);
        let json = webkitgtk.to_json();
        assert!(json.contains("\"isolation\":\"main-thread\""), "{json}");
        assert!(
            json.contains("\"reason\":\"no WebGL2 in worker\""),
            "{json}"
        );
        assert!(json.contains("\"ok\":false"), "{json}");
    }

    #[test]
    fn chromium_worker_case_is_reported_as_worker() {
        let chromium = Capability::new(Context::Worker, true);
        assert!(chromium.ok());
        assert_eq!(chromium.isolation(), ISOLATION_WORKER);
        assert_eq!(chromium.reason(), REASON_WORKER_AVAILABLE);
        let json = chromium.to_json();
        assert!(json.contains("\"isolation\":\"worker\""), "{json}");
        assert!(json.contains("\"ok\":true"), "{json}");
    }

    #[test]
    fn report_is_flat_json_with_the_three_contract_fields() {
        for context in Context::ALL {
            for webgl2 in [false, true] {
                let json = Capability::new(context, webgl2).to_json();
                for field in ["\"ok\":", "\"isolation\":", "\"reason\":"] {
                    assert!(json.contains(field), "{json} must carry {field}");
                }
                // One object, no nesting: the host reads `isolation`/`reason` at the
                // top level and nothing here may require a second parse.
                assert!(json.starts_with('{') && json.ends_with('}'), "{json}");
                assert!(!json.contains('['), "{json} must not nest arrays");
            }
        }
    }

    #[test]
    fn reason_escaping_keeps_the_json_parseable() {
        assert_eq!(escape_json("no \"quotes\""), "no \\\"quotes\\\"");
        assert_eq!(escape_json("back\\slash"), "back\\\\slash");
        assert_eq!(
            escape_json(REASON_NO_WEBGL2_IN_WORKER),
            REASON_NO_WEBGL2_IN_WORKER
        );
    }

    #[test]
    fn context_tokens_are_distinct_and_stable() {
        assert_eq!(Context::Window.as_str(), "window");
        assert_eq!(Context::Worker.as_str(), "worker");
        assert_ne!(Context::Window.as_str(), Context::Worker.as_str());
        for context in Context::ALL {
            assert_eq!(context.to_string(), context.as_str());
        }
    }
}
