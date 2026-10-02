// Visibility-aware renderer diagnostics with a bounded polling backoff (B29).
import { pollAsyncOpen } from "./async-open.js";

export function startStatePolling(
  wasmModule,
  setStateKey,
  { onReady = () => {}, onFailure = () => {} } = {},
) {
  let pollTimer = null;
  let pollDelay = 400;
  let rendererReady = false;
  const started = performance.now();

  function stopPolling() {
    if (pollTimer !== null) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
  }

  function schedulePolling() {
    stopPolling();
    if (document.hidden) return;
    pollTimer = setTimeout(poll, pollDelay);
  }

  function poll() {
    pollTimer = null;
    if (document.hidden) return;
    try {
      const report = wasmModule.renderer_state_report();
      if (wasmModule.shell_geometry) {
        const geometry = wasmModule.shell_geometry();
        document.body.classList.toggle("mobile-tools", !!geometry[4] && !!geometry[5]);
      }
      window.yacrState = report;
      if (/error=Some\(/.test(report)) {
        setStateKey("host.renderer_failed", { error: report });
        onFailure();
        return;
      }
      const ready = /adapter=Some\((\w+)\)/.exec(report);
      if (ready) {
        // Heartbeats must not overwrite a later open/export/import message.
        if (!rendererReady) {
          rendererReady = true;
          setStateKey("host.renderer_ready", { backend: ready[1] });
          // Hide the technical `#host-state` line so it cannot permanently cover
          // the Slint status once the session is usable (audit U08/U12). A later
          // failure re-shows it through the retry affordance.
          if (document.body && document.body.classList) {
            document.body.classList.add("renderer-ready");
          }
          onReady();
        }
        pollDelay = 2000;
      } else {
        pollDelay = Math.min(Math.round(pollDelay * 1.5), 2000);
      }
    } catch (error) {
      if (!rendererReady) setStateKey("host.poll_not_ready");
      pollDelay = Math.min(Math.round(pollDelay * 1.5), 2000);
    }
    // Asynchronous open (F01): poll, publish and push the Slint progress panel
    // through one wasm export. Runs after the renderer branch so a running job
    // tightens the heartbeat instead of being overwritten by the 2s backoff. A
    // host without the worker reports `running:false` and this is a cheap
    // no-op that still refreshes an explicit failed/cancelled terminal.
    try {
      const asyncState = pollAsyncOpen(wasmModule);
      if (asyncState && asyncState.running) {
        pollDelay = 250;
      }
    } catch (error) {
      // A transient wasm borrow error must not stop renderer polling.
    }
    if (!rendererReady && performance.now() - started > 30000) {
      setStateKey("host.renderer_failed", { error: "initialization timeout" });
      onFailure();
      return;
    }
    schedulePolling();
  }

  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      stopPolling();
    } else {
      pollDelay = 400;
      schedulePolling();
    }
  });
  schedulePolling();
}
