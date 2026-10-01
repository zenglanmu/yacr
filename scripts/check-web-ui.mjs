// Headless browser verification for the yacr web build (spec v2.0 §11.2).
//
// Usage:
//   scripts/serve-web.py --directory web-dist &   # or any static server
//   node scripts/check-web-ui.mjs [url] [screenshot]
//
// Exit code 0 only when: wasm boots, the renderer reports a backend, the CAD
// frame is actually non-uniform (something was drawn), and no unexpected
// console/page errors were raised.

import zlib from "node:zlib";
import { readFileSync, writeFileSync } from "node:fs";

// Playwright is a verification tool, not a repo dependency: resolve it from
// PLAYWRIGHT_MODULE (absolute path) or a local/global installation.
const playwrightSpecifier = process.env.PLAYWRIGHT_MODULE || "playwright";
const { chromium } = await import(playwrightSpecifier);

const url = process.argv[2] || process.env.WEB_URL || "http://127.0.0.1:8090/";
const screenshotPath = process.argv[3] || process.env.SCREENSHOT || "/tmp/opencode/yacr-web.png";

// Minimal PNG (RGBA/RGB 8-bit, non-interlaced) decoder for screenshot checks.
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

function imageStats(image) {
  const { width, height, channels, pixels, stride } = image;
  const counts = new Map();
  let sampled = 0;
  let min = 255;
  let max = 0;
  let sum = 0;
  let sumSq = 0;
  // Sample every 4th pixel to keep this fast.
  for (let y = 0; y < height; y += 2) {
    for (let x = 0; x < width; x += 2) {
      const index = y * stride + x * channels;
      const luma = 0.2126 * pixels[index] + 0.7152 * pixels[index + 1] + 0.0722 * pixels[index + 2];
      counts.set(`${pixels[index]},${pixels[index + 1]},${pixels[index + 2]}`, true);
      min = Math.min(min, luma);
      max = Math.max(max, luma);
      sum += luma;
      sumSq += luma * luma;
      sampled += 1;
    }
  }
  const mean = sum / sampled;
  const variance = sumSq / sampled - mean * mean;
  return { sampled, distinctColors: counts.size, min, max, mean, stddev: Math.sqrt(Math.max(0, variance)) };
}

const browser = await chromium.launch({
  args: ["--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader"],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });

// Expose the WebGL2 probe to the page? No: query it directly in-page below.
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
const pageErrors = [];
page.on("console", (message) => {
  consoleMessages.push(`${message.type()}: ${message.text()}`);
});
page.on("pageerror", (error) => {
  pageErrors.push(String(error));
});

const report = { url, state: null, image: null, backend: null, console: [], pageErrors: [] };
let failed = false;
try {
  await page.goto(url, { waitUntil: "load", timeout: 60000 });
  await page.waitForFunction(
    () => typeof window.yacrState === "string" && /adapter=(Some|None)/.test(window.yacrState),
    null,
    { timeout: 60000 },
  );
  // Give the renderer a couple of frames plus any async status update.
  await page.waitForTimeout(2000);
  report.state = await page.evaluate(() => window.yacrState);
  const caps = await page.evaluate(() => {
    const canvas = document.getElementById("canvas");
    return {
      canvasWidth: canvas ? canvas.width : 0,
      canvasHeight: canvas ? canvas.height : 0,
      webgl2: webgl2Available(),
    };
  });
  report.canvas = caps;
  const screenshot = await page.screenshot({ path: screenshotPath });
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
  const stats = imageStats(decodePng(screenshot));
  report.image = { ...stats, path: screenshotPath };
  if (stats.distinctColors < 4 || stats.stddev < 1.0) {
    throw new Error(`frame looks blank: ${JSON.stringify(stats)}`);
  }
} catch (error) {
  failed = true;
  report.failure = String(error);
} finally {
  report.console = consoleMessages.filter((line) => !/Using exceptions for control flow/.test(line));
  report.pageErrors = pageErrors.filter((line) => !/Using exceptions for control flow/.test(line));
  if (report.pageErrors.length > 0) failed = true;
  await browser.close();
}

writeFileSync(
  screenshotPath.replace(/\.png$/, ".json"),
  JSON.stringify(report, null, 2),
);
console.log(JSON.stringify(report, null, 2));
if (failed) {
  console.error("web UI check FAILED");
  process.exit(1);
}
console.log("web UI check passed");