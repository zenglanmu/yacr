// Headless browser verification for the yacr web build (spec v2.0 §11.2).
//
// Usage:
//   scripts/serve-web.py --directory web-dist &
//   PLAYWRIGHT_MODULE=/path/to/playwright-core/index.js \
//     node scripts/check-web-ui.mjs [url] [screenshot]
//
// Exit code 0 only when: wasm boots with a real adapter, the CAD frame is
// non-uniform, a real navigation changes the cropped CAD region, the language
// switch preserves the document and updates the HTML chrome, and no unexpected
// console/page errors were raised.
//
// Playwright is a verification tool, not a repo dependency: resolve it from
// PLAYWRIGHT_MODULE (absolute path to an entry file) or a local installation.

import zlib from "node:zlib";
import { writeFileSync } from "node:fs";

const playwrightSpecifier = process.env.PLAYWRIGHT_MODULE || "playwright";
const playwrightModule = await import(playwrightSpecifier);
const chromium = playwrightModule.chromium ?? playwrightModule.default?.chromium;
if (!chromium) {
  console.error(
    `could not resolve chromium from '${playwrightSpecifier}' (set PLAYWRIGHT_MODULE)`,
  );
  process.exit(2);
}

const url = process.argv[2] || process.env.WEB_URL || "http://127.0.0.1:8090/";
const screenshotPath = process.argv[3] || process.env.SCREENSHOT || "/tmp/opencode/yacr-web.png";

// --- PNG decode (RGBA/RGB 8-bit, non-interlaced) -----------------------------

function decodePng(buffer) {
  if (buffer.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
  let offset = 8;
  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  const idat = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString("ascii", offset + 4, offset + 8);
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
      if (bitDepth !== 8 || (colorType !== 6 && colorType !== 2)) {
        throw new Error(`unsupported PNG format depth=${bitDepth} color=${colorType}`);
      }
    } else if (type === "IDAT") {
      idat.push(data);
    } else if (type === "IEND") {
      break;
    }
    offset += 12 + length;
  }
  const channels = colorType === 6 ? 4 : 3;
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const pixels = Buffer.alloc(height * stride);
  let previous = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const current = Buffer.alloc(stride);
    for (let x = 0; x < stride; x += 1) {
      const left = x >= channels ? current[x - channels] : 0;
      const up = previous[x];
      const upLeft = x >= channels ? previous[x - channels] : 0;
      let value = line[x];
      switch (filter) {
        case 0: break;
        case 1: value = (value + left) & 0xff; break;
        case 2: value = (value + up) & 0xff; break;
        case 3: value = (value + ((left + up) >> 1)) & 0xff; break;
        case 4: {
          const p = left + up - upLeft;
          const pa = Math.abs(p - left);
          const pb = Math.abs(p - up);
          const pc = Math.abs(p - upLeft);
          const predictor = pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft;
          value = (value + predictor) & 0xff;
          break;
        }
        default: throw new Error(`unknown PNG filter ${filter}`);
      }
      current[x] = value;
    }
    current.copy(pixels, y * stride);
    previous = current;
  }
  return { width, height, channels, pixels, stride };
}

// --- CAD-region sampling -----------------------------------------------------

/// Crop rectangle in normalized [0,1] coordinates. The Slint shell draws the
/// top bar, tool panels and status bar outside this band; the demo geometry
/// (room outline + circle) is centred by `image-fit: contain` in the canvas
/// rectangle above the tool panels.
const CAD_REGION = { x0: 0.32, y0: 0.10, x1: 0.68, y1: 0.42 };

function pixelRect(image, region) {
  return {
    x0: Math.floor(region.x0 * image.width),
    y0: Math.floor(region.y0 * image.height),
    x1: Math.ceil(region.x1 * image.width),
    y1: Math.ceil(region.y1 * image.height),
  };
}

/// Sample the CAD region: colour variety (is something drawn?) plus the luma
/// vector used to detect change after navigation.
function sampleRegion(image, region, step = 2) {
  const { x0, y0, x1, y1 } = pixelRect(image, region);
  const { channels, pixels, stride } = image;
  const colors = new Set();
  const luma = [];
  let min = 255;
  let max = 0;
  let sum = 0;
  let sumSq = 0;
  for (let y = y0; y < y1; y += step) {
    for (let x = x0; x < x1; x += step) {
      const index = y * stride + x * channels;
      const r = pixels[index];
      const g = pixels[index + 1];
      const b = pixels[index + 2];
      colors.add((r << 16) | (g << 8) | b);
      const value = 0.2126 * r + 0.7152 * g + 0.0722 * b;
      luma.push(value);
      min = Math.min(min, value);
      max = Math.max(max, value);
      sum += value;
      sumSq += value * value;
    }
  }
  const sampled = luma.length;
  const mean = sum / sampled;
  const variance = sumSq / sampled - mean * mean;
  return {
    sampled,
    distinctColors: colors.size,
    min,
    max,
    mean,
    stddev: Math.sqrt(Math.max(0, variance)),
    luma,
  };
}

/// Fraction of sampled pixels whose luma changed by more than `tolerance`.
function changedFraction(before, after, tolerance = 8) {
  const length = Math.min(before.luma.length, after.luma.length);
  if (length === 0) return 0;
  let changed = 0;
  for (let i = 0; i < length; i += 1) {
    if (Math.abs(before.luma[i] - after.luma[i]) > tolerance) changed += 1;
  }
  return changed / length;
}

// --- run ---------------------------------------------------------------------

const browser = await chromium.launch({
  args: [
    "--no-sandbox",
    "--enable-unsafe-swiftshader",
    "--use-gl=angle",
    "--use-angle=swiftshader",
  ],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });

await page.addInitScript(() => {
  window.webgl2Available = () => {
    const canvas = document.getElementById("canvas") || document.createElement("canvas");
    try {
      return !!canvas.getContext("webgl2");
    } catch (error) {
      return false;
    }
  };
});

const consoleMessages = [];
const consoleErrors = [];
const pageErrors = [];
const isHandoff = (text) =>
  typeof text === "string" && text.includes("Using exceptions for control flow");

page.on("console", (message) => {
  const line = `${message.type()}: ${message.text()}`;
  consoleMessages.push(line);
  if (message.type() === "error" && !isHandoff(line)) consoleErrors.push(line);
});
page.on("pageerror", (error) => {
  const text = String(error);
  if (!isHandoff(text)) pageErrors.push(text);
});

const report = {
  url,
  browser: browser.version(),
  state: null,
  canvas: null,
  cadBefore: null,
  cadAfter: null,
  navigation: null,
  language: null,
  console: [],
  consoleErrors: [],
  pageErrors: [],
};
let failed = false;
const fail = (message) => {
  failed = true;
  report.failure = report.failure ? `${report.failure}; ${message}` : message;
};

try {
  await page.goto(url, { waitUntil: "load", timeout: 60000 });
  // Distinguish a real startup failure from a legitimate winit handoff, and wait
  // until the renderer either adopts an adapter or reports a failure. The
  // initial report is `adapter=None` before the first rendering-setup frame.
  await page.waitForFunction(
    () =>
      (typeof window.yacrStartupError === "string" && window.yacrStartupError.length > 0) ||
      (typeof window.yacrState === "string" &&
        (/adapter=Some\(/.test(window.yacrState) || /error=Some\(/.test(window.yacrState))),
    null,
    { timeout: 60000 },
  );
  const startupError = await page.evaluate(() => window.yacrStartupError || null);
  if (startupError) throw new Error(`startup failed: ${startupError}`);

  // Give the renderer a couple of frames plus the async adapter report.
  await page.waitForTimeout(2000);
  report.state = await page.evaluate(() => window.yacrState);
  const caps = await page.evaluate(() => {
    const canvas = document.getElementById("canvas");
    return {
      canvasWidth: canvas ? canvas.width : 0,
      canvasHeight: canvas ? canvas.height : 0,
      webgl2: window.webgl2Available(),
    };
  });
  report.canvas = caps;
  if (!report.state) throw new Error("no renderer state reported");
  if (/error=Some/.test(report.state)) throw new Error(`renderer error: ${report.state}`);
  const adapter = /adapter=Some\((\w+)\)/.exec(report.state);
  if (!adapter) throw new Error(`no adapter adopted; state: ${report.state}`);
  report.backend = adapter[1];
  if (report.backend === "WebGpu" && /caps=Some\(\(WebGpu/.test(report.state) === false) {
    throw new Error(`WebGPU reported without enhanced capabilities: ${report.state}`);
  }
  if (report.backend === "WebGl2" && !/caps=Some\(\(WebGl2, false,/.test(report.state)) {
    throw new Error(`WebGL2 must report the base tier (no compute): ${report.state}`);
  }

  const first = decodePng(await page.screenshot({ path: screenshotPath }));
  const cadBefore = sampleRegion(first, CAD_REGION);
  report.cadBefore = { ...cadBefore, luma: undefined, region: CAD_REGION };
  // A blank/uniform region has one colour and zero variance. A real line
  // drawing may be only two colours (strokes on the clear colour), so the
  // variety test must not demand a rich palette.
  if (cadBefore.distinctColors < 2 || cadBefore.stddev < 1.0 || cadBefore.max - cadBefore.min < 20) {
    throw new Error(`CAD region looks blank: ${JSON.stringify(report.cadBefore)}`);
  }

  // Real navigation through the scroll input path (zoom), then require the CAD
  // region to actually change. UI chrome is outside the crop and the point is
  // inside the canvas touch area (above the tool panels).
  const cx = first.width * 0.5;
  const cy = first.height * 0.18;
  await page.mouse.move(cx, cy);
  await page.mouse.wheel(0, 240);
  await page.waitForTimeout(800);

  const navShot = screenshotPath.replace(/\.png$/, "-navigation.png");
  const second = decodePng(await page.screenshot({ path: navShot }));
  const cadAfter = sampleRegion(second, CAD_REGION);
  const fraction = changedFraction(cadBefore, cadAfter);
  report.cadAfter = { ...cadAfter, luma: undefined, region: CAD_REGION };
  report.navigation = { screenshot: navShot, changedFraction: fraction };
  // A stale/static frame changes by exactly 0; thin CAD strokes under a zoom
  // move only a small fraction of the sampled pixels, so require a clear but
  // modest change (measured ~0.3% for the demo zoom).
  if (fraction < 0.001) {
    throw new Error(
      `CAD region did not change after navigation (changedFraction=${fraction.toFixed(4)})`,
    );
  }

  // Font orchestration (informational): the demo drawing references no CAD text
  // fonts, so this exercises the real host path and must resolve, not hang.
  report.fonts = await page.evaluate(async () => {
    try {
      return { ok: true, report: await window.yacr.load_fonts() };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  });

  // Language switch (N01): updates HTML chrome and persisted preference without
  // resetting the document/camera/annotations/undo history.
  const beforeSwitch = await page.evaluate(() => ({
    lang: document.documentElement.lang,
    title: document.title,
    entities: (window.yacr.renderer_state_report().match(/entities=(\d+)/) || [])[1],
    marker: (window.__yacrMarker = "keep"),
  }));
  const resolved = await page.evaluate(() => window.yacr.set_locale("en"));
  await page.waitForTimeout(300);
  const afterSwitch = await page.evaluate(() => ({
    lang: document.documentElement.lang,
    title: document.title,
    label: (document.getElementById("language-label") || {}).textContent,
    entities: (window.yacr.renderer_state_report().match(/entities=(\d+)/) || [])[1],
    marker: window.__yacrMarker,
    stored: localStorage.getItem("yacr.cad.locale"),
    state: window.yacrState,
  }));
  report.language = { resolved, before: beforeSwitch, after: afterSwitch };
  if (resolved !== "en") fail(`set_locale returned ${JSON.stringify(resolved)}`);
  if (afterSwitch.lang !== "en") fail("document lang did not switch to en");
  if (beforeSwitch.title === afterSwitch.title) {
    fail("document title did not change after language switch");
  }
  if (afterSwitch.marker !== "keep") fail("language switch reloaded/reset the page");
  if (beforeSwitch.entities !== afterSwitch.entities) {
    fail("language switch changed the document entity count");
  }

  // Persisted preference restores on a fresh start (HTML chrome + host locale).
  await page.reload({ waitUntil: "load", timeout: 60000 });
  await page.waitForFunction(
    () =>
      window.yacr &&
      typeof window.yacr.current_locale === "function" &&
      typeof window.yacrState === "string" &&
      /adapter=/.test(window.yacrState),
    null,
    { timeout: 60000 },
  );
  const restored = await page.evaluate(() => ({
    locale: window.yacr.current_locale(),
    lang: document.documentElement.lang,
    title: document.title,
    stored: localStorage.getItem("yacr.cad.locale"),
  }));
  report.restore = restored;
  if (restored.locale !== "en") fail(`locale not restored after reload: ${restored.locale}`);
  if (restored.lang !== "en") fail(`HTML lang not restored after reload: ${restored.lang}`);
} catch (error) {
  fail(String(error));
} finally {
  report.console = consoleMessages.filter((line) => !isHandoff(line));
  report.consoleErrors = consoleErrors;
  report.pageErrors = pageErrors;
  // Unexpected console/page errors fail the run (audit B29).
  if (report.consoleErrors.length > 0) fail(`unexpected console errors: ${report.consoleErrors.length}`);
  if (report.pageErrors.length > 0) fail(`unexpected page errors: ${report.pageErrors.length}`);
  await browser.close();
}

writeFileSync(screenshotPath.replace(/\.png$/, ".json"), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
if (failed) {
  console.error("web UI check FAILED");
  process.exit(1);
}
console.log("web UI check passed");
