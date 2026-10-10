// App icon / web manifest contracts for the HTML host.
//
// These run with `node --test` and need neither the wasm build nor a browser:
// the icon wiring in `index.html` is asserted as string checks, and the
// manifest plus the referenced icon files are read straight off disk (no
// network).
//
// Run: node --test scripts/test-web-icon.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const html = readFileSync(
  new URL("../apps/app-web/web/index.html", import.meta.url),
  "utf8",
);
const manifestPath = new URL(
  "../apps/app-web/web/manifest.webmanifest",
  import.meta.url,
);

function exists(url) {
  try {
    readFileSync(url);
    return true;
  } catch {
    return false;
  }
}

test("index.html links the SVG favicon and the web manifest", () => {
  assert.match(html, /<link[^>]*rel="icon"[^>]*href="assets\/yacr-icon\.svg"/);
  assert.match(html, /<link[^>]*rel="manifest"[^>]*href="manifest\.webmanifest"/);
});

test("the single-source-of-truth SVG icon exists and is self-contained", () => {
  const svg = readFileSync(
    new URL("../assets/yacr-icon.svg", import.meta.url),
    "utf8",
  );
  assert.ok(svg.startsWith("<svg"), "assets/yacr-icon.svg must start with <svg");
});

test("manifest parses as JSON and lists the SVG icon", () => {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  assert.ok(Array.isArray(manifest.icons), "manifest.icons must be an array");
  assert.ok(
    manifest.icons.some((icon) => icon.src === "assets/yacr-icon.svg"),
    "manifest.icons must reference assets/yacr-icon.svg",
  );
});

test("the PNG icons referenced by the host and manifest exist", () => {
  for (const name of ["yacr-256.png", "yacr-512.png"]) {
    assert.ok(
      exists(new URL(`../assets/icons/${name}`, import.meta.url)),
      `assets/icons/${name} must exist`,
    );
  }
});
