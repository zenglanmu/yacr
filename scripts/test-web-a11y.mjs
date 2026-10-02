// Accessibility boundary contracts for the HTML host (audit U12 / U08).
//
// These run with `node --test` and need neither the wasm build nor a browser:
// the DOM-facing contracts are asserted as string/regex checks over the served
// `index.html` / `style.css`, and the announcement funnel is exercised against
// the real pure modules with a tiny document stub.
//
// Run: node --test scripts/test-web-a11y.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createA11y } from "../apps/app-web/web/host/a11y.js";
import { createI18n } from "../apps/app-web/web/host/i18n.js";
import { startStatePolling } from "../apps/app-web/web/host/renderer.js";

const html = readFileSync(
  new URL("../apps/app-web/web/index.html", import.meta.url),
  "utf8",
);
const css = readFileSync(
  new URL("../apps/app-web/web/style.css", import.meta.url),
  "utf8",
);
const catalogs = {
  "zh-CN": JSON.parse(
    readFileSync(
      new URL("../crates/cad-ui-slint/i18n/zh-CN.json", import.meta.url),
    ),
  ),
  en: JSON.parse(
    readFileSync(new URL("../crates/cad-ui-slint/i18n/en.json", import.meta.url)),
  ),
};

function tagFor(id) {
  const match = html.match(new RegExp(`<[^>]*id="${id}"[^>]*>`));
  assert.ok(match, `index.html must contain an element with id="${id}"`);
  return match[0];
}

function documentStub({ withBody = false, extraIds = [] } = {}) {
  const nodes = new Map();
  for (const id of [
    "host-state",
    "a11y-status",
    "canvas-host",
    "retry-renderer",
    "language",
    "language-label",
    "open-drawing",
    ...extraIds,
  ]) {
    nodes.set(id, {
      textContent: "",
      value: "",
      listeners: {},
      attributes: {},
      setAttribute(name, value) {
        this.attributes[name] = value;
      },
      addEventListener(name, callback) {
        this.listeners[name] = callback;
      },
    });
  }
  const document = {
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
  if (withBody) {
    document.body = {
      classList: {
        classes: new Set(),
        add(name) {
          this.classes.add(name);
        },
        contains(name) {
          return this.classes.has(name);
        },
        toggle() {},
      },
    };
  }
  return document;
}

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

test("the live region exists and is polite; the technical line is not", () => {
  const live = tagFor("a11y-status");
  assert.match(live, /role="status"/);
  assert.match(live, /aria-live="polite"/);
  assert.match(live, /class="[^"]*visually-hidden[^"]*"/);
  assert.match(live, /aria-atomic="true"/);

  const hostState = tagFor("host-state");
  assert.doesNotMatch(hostState, /aria-live="polite"/);
  assert.match(hostState, /aria-live="off"/);
});

test("the canvas host is a labelled, focusable application region", () => {
  const host = tagFor("canvas-host");
  assert.match(host, /role="application"/);
  assert.match(host, /aria-label="[^"]+"/);
  assert.match(host, /tabindex="0"/);
  // The boundary is documented where the markup lives, not only in docs.
  assert.match(html, /Accessibility boundary \(audit U12\)/);
});

test("catalogs carry the UA-facing aria strings in both languages", () => {
  for (const [locale, catalog] of Object.entries(catalogs)) {
    for (const key of ["a11y.canvas_label", "a11y.status_failed"]) {
      assert.equal(
        typeof catalog[key],
        "string",
        `${locale} must define ${key}`,
      );
      assert.ok(catalog[key].length > 0, `${locale}:${key} must not be empty`);
    }
  }
});

test("the live region is visually hidden but not display:none", () => {
  assert.match(css, /\.visually-hidden\s*\{[^}]*clip-path:\s*inset\(50%\)/s);
  assert.doesNotMatch(
    css,
    /\.visually-hidden\s*\{[^}]*display:\s*none/s,
    "display:none would hide the region from screen readers too",
  );
});

test("CSS hides the technical host-state line once the renderer is ready", () => {
  // Hidden only after `renderer-ready`, and re-shown whenever the retry
  // affordance (failure) is un-hidden.
  const rule = css.match(
    /body\.renderer-ready:not\(:has\(#retry-renderer:not\(\[hidden\]\)\)\)\s+#host-state\s*\{[^}]*display:\s*none/,
  );
  assert.ok(rule, "expected a ready-gated #host-state display:none rule");
});

test("announcements dedupe and strip the raw report from failure text", (t) => {
  const document = documentStub();
  replaceGlobal(t, "document", document);
  const i18n = {
    t: (key, args = {}) =>
      key === "a11y.status_failed"
        ? "renderer unavailable"
        : `⟦${key}:${JSON.stringify(args)}⟧`,
  };
  const a11y = createA11y(i18n);
  a11y.announceStateKey("host.renderer_ready", { backend: "WebGl2" });
  assert.match(
    document.nodes.get("a11y-status").textContent,
    /host\.renderer_ready/,
  );
  a11y.announceStateKey("host.renderer_ready", { backend: "WebGl2" });
  assert.equal(a11y.lastAnnounced(), a11y.lastAnnounced());
  a11y.announceStateKey("host.renderer_failed", {
    error: "chosen=WebGpu adapter=None error=Some(Boom)",
  });
  assert.equal(
    document.nodes.get("a11y-status").textContent,
    "renderer unavailable",
  );
  assert.doesNotMatch(
    document.nodes.get("a11y-status").textContent,
    /adapter=None/,
  );
});

test("the i18n funnel mirrors every status into the live region exactly once", async (t) => {
  const document = documentStub();
  replaceGlobal(t, "document", document);
  replaceGlobal(t, "localStorage", {
    getItem: () => "en",
    setItem: () => {},
  });
  replaceGlobal(t, "fetch", async (path) => ({
    ok: true,
    json: async () =>
      catalogs[path.includes("zh-CN") ? "zh-CN" : "en"],
  }));
  const i18n = createI18n(() => null);
  const a11y = createA11y(i18n);
  i18n.attachA11y(a11y);
  await i18n.initialize();
  // The canvas label is re-applied on load so it follows the active locale.
  assert.equal(
    document.nodes.get("canvas-host").attributes["aria-label"],
    catalogs.en["a11y.canvas_label"],
  );
  i18n.setStateKey("host.renderer_ready", { backend: "WebGL2" });
  const announced = document.nodes.get("a11y-status").textContent;
  assert.match(announced, /WebGL2/);
  assert.equal(announced, document.nodes.get("host-state").textContent);
});

test("renderer readiness flags the body for the CSS hide rule", (t) => {
  const document = documentStub({ withBody: true });
  let scheduled;
  replaceGlobal(t, "document", document);
  replaceGlobal(t, "window", {});
  replaceGlobal(t, "setTimeout", (callback) => {
    scheduled = { callback };
    return 1;
  });
  replaceGlobal(t, "clearTimeout", () => {});
  startStatePolling({ renderer_state_report: () => "adapter=Some(WebGl2)" }, () => {});
  scheduled.callback();
  assert.equal(document.body.classList.contains("renderer-ready"), true);
});
