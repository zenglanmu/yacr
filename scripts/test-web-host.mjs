// Pure host-module contracts; the real wasm/GPU path is check-web-ui.mjs.
// Run: node --test scripts/test-web-host.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createFileHost } from "../apps/app-web/web/host/files.js";
import { createI18n } from "../apps/app-web/web/host/i18n.js";
import { startStatePolling } from "../apps/app-web/web/host/renderer.js";
import { isHandoffError } from "../apps/app-web/web/host/runtime.js";
import {
  chooseBackend,
  withDeadline,
  wireRecoveryBackend,
} from "../apps/app-web/web/host/startup.js";

function replaceGlobal(t, key, value) {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
  Object.defineProperty(globalThis, key, {
    configurable: true,
    writable: true,
    value,
  });
  t.after(() => {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else delete globalThis[key];
  });
}

function documentStub() {
  const nodes = new Map();
  for (const id of [
    "host-state",
    "language",
    "language-label",
    "file-input",
    "annotation-input",
  ]) {
    nodes.set(id, {
      textContent: "",
      value: "",
      files: [],
      listeners: {},
      addEventListener(name, callback) {
        this.listeners[name] = callback;
      },
    });
  }
  return {
    nodes,
    documentElement: {},
    title: "",
    hidden: false,
    listeners: {},
    getElementById: (id) => nodes.get(id),
    querySelector: () => null,
    addEventListener(name, callback) {
      this.listeners[name] = callback;
    },
  };
}

test("i18n uses shared catalogs, preserves raw status and tolerates read-only storage", async (t) => {
  const document = documentStub();
  const tags = [];
  replaceGlobal(t, "document", document);
  replaceGlobal(t, "localStorage", {
    getItem: () => "zh-Hans",
    setItem: () => {
      throw new Error("read-only storage");
    },
  });
  replaceGlobal(t, "fetch", async (path) => ({
    ok: true,
    json: async () =>
      JSON.parse(
        readFileSync(
          new URL(`../crates/cad-ui-slint/${path}`, import.meta.url),
        ),
      ),
  }));
  const host = createI18n(() => ({
    web_set_locale: (tag) => {
      tags.push(tag);
      return tag;
    },
  }));
  await host.initialize();
  assert.equal(host.currentLocale(), "zh-CN");
  assert.equal(host.t("missing.key"), "⟦missing.key⟧");
  host.setStateKey("host.exported_bytes", { bytes: 42 });
  const before = document.nodes.get("host-state").textContent;
  assert.equal(await host.setLocale("en-US"), "en");
  assert.notEqual(document.nodes.get("host-state").textContent, before);
  assert.match(document.nodes.get("host-state").textContent, /42/);
  assert.equal(document.documentElement.lang, "en");
  assert.deepEqual(tags, ["en"]);
  host.setStateText("raw Rust diagnostic");
  await host.setLocale("zh-CN");
  assert.equal(
    document.nodes.get("host-state").textContent,
    "raw Rust diagnostic",
  );
});

test("annotation export confirms the exact revision after download, never after failure", (t) => {
  const events = [];
  let failClick = false;
  replaceGlobal(t, "document", {
    createElement: () => ({
      click() {
        events.push("download");
        if (failClick) throw new Error("download failed");
      },
    }),
  });
  t.mock.method(URL, "createObjectURL", () => "blob:test");
  t.mock.method(URL, "revokeObjectURL", () => events.push("revoke"));
  const host = createFileHost(
    {
      annotation_export_json: () => ({ json: "{}", revision: 7 }),
      annotation_confirm_export: (revision) =>
        events.push(["confirm", revision]),
    },
    { setStateKey: (key) => events.push(key) },
  );
  host.exportAnnotations();
  assert.deepEqual(events, [
    "download",
    "revoke",
    ["confirm", 7],
    "host.exported_bytes",
  ]);
  events.length = 0;
  failClick = true;
  assert.throws(() => host.exportAnnotations(), /download failed/);
  assert.deepEqual(events, ["download"]);
});

test("file pickers cancel without replacing a drawing and reset after reads", async (t) => {
  const document = documentStub();
  const opened = [];
  const states = [];
  let decision = null;
  replaceGlobal(t, "document", document);
  replaceGlobal(t, "window", { prompt: () => decision });
  const host = createFileHost(
    {
      open_requires_decision: () => true,
      open_document_bytes_decided: (...args) => {
        opened.push(args);
        return "opened";
      },
      annotation_import_json: (text) => {
        assert.equal(text, "{}");
        return 0;
      },
    },
    {
      t: (key) => key,
      setStateKey: (key) => states.push(key),
      setStateText: (text) => states.push(text),
    },
  );
  host.wireFilePickers();
  const picker = document.nodes.get("file-input");
  picker.files = [
    {
      name: "drawing.dwg",
      arrayBuffer: async () => new Uint8Array([1, 2]).buffer,
    },
  ];
  await picker.listeners.change();
  assert.equal(opened.length, 0);
  assert.equal(states.at(-1), "host.cancelled_open");
  assert.equal(picker.value, "");
  decision = "discard";
  await picker.listeners.change();
  assert.equal(opened[0][0], "drawing.dwg");
  assert.deepEqual([...opened[0][1]], [1, 2]);
  assert.equal(opened[0][2], "discard");
  const annotations = document.nodes.get("annotation-input");
  annotations.files = [{ text: async () => "{}" }];
  await annotations.listeners.change();
  assert.equal(states.at(-1), "host.imported_count");
  assert.equal(annotations.value, "");
});

test("renderer polling backs off, pauses while hidden and announces readiness only once", (t) => {
  const document = documentStub();
  const states = [];
  let scheduled;
  let ready = false;
  replaceGlobal(t, "document", document);
  replaceGlobal(t, "window", {});
  replaceGlobal(t, "setTimeout", (callback, delay) => {
    scheduled = { callback, delay };
    return 1;
  });
  replaceGlobal(t, "clearTimeout", () => {
    scheduled = null;
  });
  startStatePolling(
    {
      renderer_state_report: () =>
        ready ? "adapter=Some(WebGl2)" : "adapter=None",
    },
    (...args) => states.push(args),
  );
  assert.equal(scheduled.delay, 400);
  scheduled.callback();
  assert.equal(scheduled.delay, 600);
  ready = true;
  scheduled.callback();
  assert.equal(scheduled.delay, 2000);
  scheduled.callback();
  assert.deepEqual(states, [["host.renderer_ready", { backend: "WebGl2" }]]);
  document.hidden = true;
  document.listeners.visibilitychange();
  assert.equal(scheduled, null);
  document.hidden = false;
  document.listeners.visibilitychange();
  assert.equal(scheduled.delay, 400);
});

test("only the documented winit handoff is a non-failure", () => {
  assert.equal(
    isHandoffError(new Error("Using exceptions for control flow")),
    true,
  );
  assert.equal(isHandoffError("Using exceptions for control flow"), true);
  assert.equal(isHandoffError(new Error("wasm initialization failed")), false);
  assert.equal(isHandoffError(null), false);
});

test("a stalled WebGPU device probe falls back to WebGL2 within a bounded time", async (t) => {
  replaceGlobal(t, "location", {
    href: "https://example.test/?backend=webgpu",
  });
  replaceGlobal(t, "window", {});
  replaceGlobal(t, "navigator", {
    gpu: { requestAdapter: () => new Promise(() => {}) },
  });
  assert.equal(await chooseBackend({ probeTimeout: 5 }), "webgl2");
  assert.match(window.yacrBackendProbe.reason, /timeout/);
  assert.equal(window.yacrBackendProbe.requested, "webgpu");
});

test("a WebGPU adapter without a usable device is not reported as ready", async (t) => {
  replaceGlobal(t, "location", { href: "https://example.test/?backend=auto" });
  replaceGlobal(t, "window", {});
  replaceGlobal(t, "navigator", {
    gpu: {
      requestAdapter: async () => ({
        requestDevice: async () => {
          throw new Error("device rejected");
        },
      }),
    },
  });
  assert.equal(await chooseBackend(), "webgl2");
  assert.match(window.yacrBackendProbe.reason, /device rejected/);
});

test("startup deadlines reject instead of displaying an endless spinner", async () => {
  await assert.rejects(
    withDeadline(new Promise(() => {}), 5, "renderer"),
    /renderer: timeout/,
  );
});

test("the recovery-backend button cannot reload unsaved annotations", (t) => {
  const document = documentStub();
  const retry = {
    listeners: {},
    hidden: true,
    addEventListener(name, callback) {
      this.listeners[name] = callback;
    },
  };
  document.nodes.set("retry-renderer", retry);
  const destinations = [];
  const messages = [];
  let dirty = true;
  replaceGlobal(t, "document", document);
  replaceGlobal(t, "location", {
    href: "https://example.test/?backend=webgpu",
    assign: (url) => destinations.push(url),
  });
  const show = wireRecoveryBackend(
    () => ({ open_requires_decision: () => dirty }),
    (key) => messages.push(key),
  );
  show();
  assert.equal(retry.hidden, false);
  retry.listeners.click();
  assert.equal(destinations.length, 0);
  assert.deepEqual(messages, ["host.retry_unsaved"]);
  dirty = false;
  retry.listeners.click();
  assert.equal(new URL(destinations[0]).searchParams.get("backend"), "webgl2");
});
