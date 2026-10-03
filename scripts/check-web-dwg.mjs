// Secondary browser integration smoke for an external real DWG, not a golden test.
// PLAYWRIGHT_MODULE=... node scripts/check-web-dwg.mjs URL DWG OUTPUT [entities]
import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { decodePng } from "./web-pixels.mjs";

const module = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = module.chromium || module.default.chromium;
const [url, dwg, output, expectedEntities] = process.argv.slice(2);
const timeout = Number(process.env.WEB_DWG_TIMEOUT || 120000);
assert.ok(url && dwg && output, "usage: URL DWG OUTPUT [entities]");
mkdirSync(output, { recursive: true });
const report = { url, dwg, errors: [], console: [], status: "failed" };
let page;
const server = await chromium.launchServer({
  headless: true,
  args: ["--no-sandbox", "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader"],
});
try {
  const browser = await chromium.connect(server.wsEndpoint());
  report.browser = browser.version();
  page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  page.setDefaultTimeout(timeout);
  page.on("pageerror", (error) => {
    if (!String(error).includes("Using exceptions for control flow")) report.errors.push(String(error));
  });
  page.on("console", (message) => {
    report.console.push(`${message.type()}: ${message.text()}`);
    if (message.type() === "error" && !message.text().includes("Using exceptions for control flow"))
      report.errors.push(message.text());
  });
  await page.goto(url, { waitUntil: "load" });
  await page.waitForFunction(() => {
    const state = window.yacr?.renderer_state_report();
    return state?.includes("adapter=Some") && Number(state.match(/cad_frames=(\d+)/)?.[1]) >= 1;
  });
  const before = await page.evaluate(() => window.yacr.renderer_state_report());
  report.before = before;
  const initialFrames = Number(before.match(/cad_frames=(\d+)/)[1]);
  const initialEntities = Number(before.match(/entities=(\d+)/)[1]);
  await page.locator("#file-input").setInputFiles(dwg);
  await page.waitForFunction((count) => {
    const state = window.yacr.renderer_state_report();
    return Number(state.match(/entities=(\d+)/)?.[1]) !== count && state.includes("error=None");
  }, initialEntities);
  report.state = await page.evaluate(() => window.yacr.renderer_state_report());
  if (expectedEntities) assert.match(report.state, new RegExp(`entities=${Number(expectedEntities)}(?: |,)`));
  // Await the actual font pipeline, including failures/unknown-face plans.
  report.fonts = await page.evaluate(() => window.yacr.load_fonts());
  // Entity count is published before GPU upload/presentation. A stale demo
  // texture is non-uniform too, so never accept it as the imported CAD frame.
  await page.waitForFunction((frames) => {
    const state = window.yacr.renderer_state_report();
    return Number(state.match(/cad_frames=(\d+)/)?.[1]) > frames && state.includes("error=None");
  }, initialFrames);
  report.state = await page.evaluate(() => window.yacr.renderer_state_report());
  report.diagnostics = await page.evaluate(() => JSON.parse(window.yacr.diagnostics_report()));
  const geometry = await page.evaluate(() => Array.from(window.yacr.shell_geometry()));
  report.geometry = geometry;
  const [x, y, width, height] = geometry;
  const bounds = await page.locator("#canvas").boundingBox();
  const png = await page.screenshot({ path: `${output}/browser-cad.png`,
    clip: { x: bounds.x + x, y: bounds.y + y, width, height }, timeout });
  const image = decodePng(png);
  const colors = new Set();
  for (let row = 4; row < image.height - 4; row += 2) {
    for (let col = 4; col < image.width - 4; col += 2) {
      const offset = row * image.stride + col * image.channels;
      colors.add(image.pixels.subarray(offset, offset + 3).toString("hex"));
    }
  }
  report.pixelColors = colors.size;
  assert.ok(colors.size > 1, "CAD viewport is blank/uniform");
  assert.match(report.state, /error=None/);
  assert.deepEqual(report.errors, []);
  report.status = "passed";
} catch (error) {
  report.failure = String(error);
  if (page) {
    try {
      report.stateAtFailure = await page.evaluate(() => window.yacr?.renderer_state_report());
    } catch (stateError) {
      report.stateFailure = String(stateError);
    }
  }
  process.exitCode = 1;
} finally {
  writeFileSync(`${output}/browser.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  await server.close();
}
