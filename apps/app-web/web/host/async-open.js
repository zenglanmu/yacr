// Asynchronous / cancellable open panel heartbeat (F01).
//
// The Rust host does the real work: `async_open_poll_json` polls the core
// manager (worker-capable hosts) or re-reads the synchronous fallback's real
// terminal snapshot, publishes an opened drawing at most once, installs the
// document and pushes the Slint progress panel. This module only:
//
//   * parses the stable panel JSON defensively, and
//   * triggers the poll from the single renderer heartbeat.
//
// The pure parser is unit-tested with Node (`scripts/test-web-host.mjs`); it
// never fabricates a state the host did not report.

/// Parse the host's panel JSON. Returns `null` for a missing/invalid payload so
/// a transient wasm error cannot be mistaken for a real progress state.
export function parseAsyncPoll(raw) {
  if (typeof raw !== "string" || raw.length === 0) return null;
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    return null;
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
  const terminal = parsed.terminal;
  if (
    terminal !== "none" &&
    terminal !== "opened" &&
    terminal !== "cancelled" &&
    terminal !== "failed"
  ) {
    return null;
  }
  return parsed;
}

/// Poll the host and record the result for diagnostics/tests.
///
/// Returns the parsed state, or `null` when the host has not started or the
/// export is absent (a build that predates the panel). Setting
/// `window.yacrAsyncOpen` mirrors the other `window.yacr*` diagnostics without
/// changing the user-facing Slint panel, which the Rust host already pushed.
export function pollAsyncOpen(wasmModule) {
  if (!wasmModule || typeof wasmModule.async_open_poll_json !== "function") {
    return null;
  }
  const state = parseAsyncPoll(wasmModule.async_open_poll_json());
  if (state !== null && typeof window !== "undefined" && window) {
    window.yacrAsyncOpen = state;
  }
  return state;
}
