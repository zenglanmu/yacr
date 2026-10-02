// Visibility-aware renderer diagnostics with a bounded polling backoff (B29).
export function startStatePolling(wasmModule, setStateKey) {
  let pollTimer = null;
  let pollDelay = 400;
  let rendererReady = false;

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
      window.yacrState = report;
      const ready = /adapter=Some\((\w+)\)/.exec(report);
      if (ready) {
        // Heartbeats must not overwrite a later open/export/import message.
        if (!rendererReady) {
          rendererReady = true;
          setStateKey("host.renderer_ready", { backend: ready[1] });
        }
        pollDelay = 2000;
      } else {
        pollDelay = Math.min(Math.round(pollDelay * 1.5), 2000);
      }
    } catch (error) {
      if (!rendererReady) setStateKey("host.poll_not_ready");
      pollDelay = Math.min(Math.round(pollDelay * 1.5), 2000);
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
