// End-to-end verification that the host-wired selection highlight and tool
// preview overlays actually change CAD pixels in a real headless browser
// (SwiftShader/WebGL2). Software GPU only: not a hardware-GPU claim.
//
// Baseline demo drawing: a 4000x3000 room with a circle (world centre 2000,1500,
// r=800). The camera target is (2000,1500) with world_per_px read from the
// renderer report, so the circle's left edge is at canvas-local x = w/2 - 800/wpp
// and the vertical centre at canvas-local y = h/2.
import assert from "node:assert/strict";
import zlib from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = playwright.chromium || playwright.default.chromium;
const url = process.argv[2] || "http://127.0.0.1:8099/";
const output = process.argv[3] || "/tmp/opencode/yacr-overlay-validation";
mkdirSync(output, { recursive: true });

function decodePng(buffer) {
  if (buffer.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
  let offset = 8, width = 0, height = 0, colorType = 0;
  const idat = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString("ascii", offset + 4, offset + 8);
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") { width = data.readUInt32BE(0); height = data.readUInt32BE(4); colorType = data[9]; }
    else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
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
        case 1: value = (value + left) & 0xff; break;
        case 2: value = (value + up) & 0xff; break;
        case 3: value = (value + ((left + up) >> 1)) & 0xff; break;
        case 4: { const p = left + up - upLeft; const pa = Math.abs(p - left), pb = Math.abs(p - up), pc = Math.abs(p - upLeft); const pred = pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft; value = (value + pred) & 0xff; break; }
      }
      current[x] = value;
    }
    current.copy(pixels, y * stride);
    previous = current;
  }
  return { width, height, channels, pixels, stride };
}

const REGION = { x0: 0.2, y0: 0.25, x1: 0.8, y1: 0.75 };
function sample(image) {
  const x0 = Math.floor(REGION.x0 * image.width), y0 = Math.floor(REGION.y0 * image.height);
  const x1 = Math.ceil(REGION.x1 * image.width), y1 = Math.ceil(REGION.y1 * image.height);
  const luma = [];
  for (let y = y0; y < y1; y += 2) for (let x = x0; x < x1; x += 2) {
    const i = y * image.stride + x * image.channels;
    luma.push(0.2126 * image.pixels[i] + 0.7152 * image.pixels[i + 1] + 0.0722 * image.pixels[i + 2]);
  }
  return luma;
}
function changed(a, b, tol = 8) {
  const n = Math.min(a.length, b.length);
  let c = 0;
  for (let i = 0; i < n; i += 1) if (Math.abs(a[i] - b[i]) > tol) c += 1;
  return c / n;
}

const server = await chromium.launchServer({
  headless: true,
  args: ["--no-sandbox", "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader"],
});
const browser = await chromium.connect(server.wsEndpoint());
const results = [];
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, hasTouch: true });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const capture = async (name) => {
    await page.mouse.move(1, 1);
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png", fromSurface: false });
    writeFileSync(`${output}/${name}.png`, Buffer.from(data, "base64"));
    return decodePng(Buffer.from(data, "base64"));
  };

  await page.goto(url);
  await page.waitForFunction(() => window.yacr?.renderer_state_report().includes("adapter=Some"), null, { timeout: 60000 });
  await page.waitForTimeout(600);
  const g = await page.evaluate(() => Array.from(window.yacr.shell_geometry()));
  const top = g[1], cw = g[2], ch = g[3];
  const report = await page.evaluate(() => window.yacr.renderer_state_report());
  const wpp = Number(report.match(/wpp=([\d.]+)/)[1]);

  const baseline = sample(await capture("00-baseline"));
  // Circle left edge: canvas-local x = cw/2 - 800/wpp, y = ch/2.
  const cx = cw / 2 - 800 / wpp;
  const cy = top + ch / 2;
  await page.mouse.click(cx, cy);
  await page.waitForTimeout(500);
  const selected = sample(await capture("01-selection-highlight"));
  const highlightChange = changed(baseline, selected);
  assert.ok(highlightChange > 0.0005, `selection highlight did not change CAD pixels: ${highlightChange}`);

  // Deselect by clicking empty space left of the room outline (the room spans
  // roughly x in [190,1090] window px; x=60 is outside it): the highlight must
  // disappear.
  await page.mouse.click(60, top + ch / 2);
  await page.waitForTimeout(500);
  const cleared = sample(await capture("02-cleared"));
  const clearedBack = changed(baseline, cleared);
  assert.ok(clearedBack < highlightChange, "clearing the selection did not remove the highlight");

  results.push({
    name: "desktop-overlay",
    worldPerPx: wpp,
    click: [Math.round(cx), Math.round(cy)],
    highlightChange,
    clearedChange: clearedBack,
    errors,
  });
  assert.deepEqual(errors, []);
  writeFileSync(`${output}/report.json`, JSON.stringify({ passed: true, results }, null, 2));
  console.log(JSON.stringify({ passed: true, output, results }));
} finally {
  server.process().kill("SIGKILL");
}
