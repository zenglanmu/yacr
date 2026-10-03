// ViewerConfig host bridge: set/update/query, allowed-path persistence and a
// DOM change event. The config protocol is data-only; no script is stored.
export const CONFIG_STORAGE_KEY = "yacr.cad.config";
export const CONFIG_EVENT_NAME = "yacr-config-changed";

// Config never carries user-facing text we render here; these are internal
// error strings surfaced in the console/status line (not localized catalog UI).
function wasmCall(wasm, name, ...args) {
  const fn = wasm?.[name];
  if (typeof fn !== "function") {
    throw new Error(`wasm export ${name} is unavailable`);
  }
  return fn(...args);
}

/// Normalize a wasm result object into `{ ok, config?, revision?, path?, reason? }`.
export function parseConfigResult(result) {
  if (!result || typeof result !== "object") {
    return { ok: false, path: "config", reason: "no result from wasm" };
  }
  if (result.ok) {
    return { ok: true, config: result.config, revision: result.revision };
  }
  return {
    ok: false,
    path: typeof result.path === "string" ? result.path : "config",
    reason: typeof result.reason === "string" ? result.reason : "rejected",
  };
}

/// Emit the change event with the revision and effective config.
export function emitConfigChanged(result) {
  const parsed = parseConfigResult(result);
  if (!parsed.ok) return parsed;
  try {
    window.dispatchEvent(
      new CustomEvent(CONFIG_EVENT_NAME, {
        detail: { revision: parsed.revision, config: parsed.config },
      }),
    );
  } catch (_) {
    /* no window (headless): the store change still holds */
  }
  return parsed;
}

export function createConfigHost(getWasmModule) {
  const wasm = () => getWasmModule();
  let last = null;

  function setConfig(config) {
    const text = typeof config === "string" ? config : JSON.stringify(config);
    const result = emitConfigChanged(wasmCall(wasm(), "viewer_set_config_json", text));
    last = result.ok ? result.config : last;
    return result;
  }

  function updateConfig(patch) {
    const text = typeof patch === "string" ? patch : JSON.stringify(patch);
    const result = emitConfigChanged(
      wasmCall(wasm(), "viewer_update_config_json", text),
    );
    last = result.ok ? result.config : last;
    return result;
  }

  /// `persist` mirrors the allowed projection the Rust host already wrote to
  /// localStorage; the JS side only records the effective config for queries.
  function applyUserPreference(patch) {
    const text = typeof patch === "string" ? patch : JSON.stringify(patch);
    const result = emitConfigChanged(
      wasmCall(wasm(), "viewer_apply_user_preference_json", text),
    );
    last = result.ok ? result.config : last;
    return result;
  }

  function clearUserPreference() {
    const result = emitConfigChanged(
      wasmCall(wasm(), "viewer_clear_user_preference"),
    );
    last = result.ok ? result.config : last;
    return result;
  }

  function config() {
    if (last) return last;
    try {
      return JSON.parse(wasmCall(wasm(), "viewer_config_json"));
    } catch (_) {
      return null;
    }
  }

  return { setConfig, updateConfig, applyUserPreference, clearUserPreference, config };
}
