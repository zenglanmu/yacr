// Real-browser end-to-end for the web file features (P3): New (blank drawing),
// Save-as-DXF download and Plot SVG/PDF download.
//
// Usage:
//   PLAYWRIGHT_MODULE=/path/to/playwright-core/index.js \
//     node scripts/check-web-files.mjs [url] [output] [fixture]
//
// With no `url` the script serves the built `web-dist` (DIST env or
// /tmp/opencode/yacr-web-p3) with scripts/serve-web.py on an ephemeral port.
//
// The browser is launched **headful with the real GPU** (no `--use-angle=swiftshader`,
// no `--enable-unsafe-swiftshader`); the GL renderer string is captured as
// evidence. SwiftShader software rendering is an explicit opt-in
// (`YACR_SOFTWARE_GPU=1`), and such a run is labelled SOFTWARE. A headful browser
// that cannot start is reported honestly as a blocker (exit code 2), not silently
// downgraded to software.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const playwrightModule = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = playwrightModule.chromium ?? playwrightModule.default?.chromium;
if (!chromium) {
  console.error("could not resolve chromium (set PLAYWRIGHT_MODULE)");
  process.exit(2);
}

const fixturePath =
  process.argv[4] ||
  process.env.WEB_FIXTURE ||
  fileURLToPath(new URL("../fixtures/plot/a4-layout.dwg", import.meta.url));
const output =
  process.argv[3] || process.env.WEB_OUTPUT || "/tmp/opencode/yacr-web-p3-e2e";
mkdirSync(output, { recursive: true });

const softwareGpu = process.env.YACR_SOFTWARE_GPU === "1";
if (softwareGpu) {
  console.error(
    "YACR_SOFTWARE_GPU=1: SOFTWARE SwiftShader run - not real-GPU evidence.",
  );
}
const report = {
  fixture: fixturePath,
  headful: !softwareGpu,
  softwareRenderer: false,
  mode: softwareGpu ? "software" : "real-gpu",
  errors: [],
  steps: [],
  status: "failed",
};
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const contains = (haystack, needle) =>
  Buffer.from(haystack).includes(Buffer.from(needle));

let server = null;
let url = process.argv[2] || process.env.WEB_URL;

async function waitForServer(target, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(target);
      if (response.ok) return;
    } catch (_) {
      /* not up yet */
    }
    await delay(200);
  }
  throw new Error(`server did not come up at ${target}`);
}

async function startServer() {
  const dist = process.env.DIST || "/tmp/opencode/yacr-web-p3";
  const port = Number(process.env.WEB_PORT || 8123);
  const script = fileURLToPath(new URL("./serve-web.py", import.meta.url));
  server = spawn("python3", [script, "--directory", dist, "--port", String(port)], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout.on("data", (data) => process.stderr.write(`[server] ${data}`));
  server.stderr.on("data", (data) => process.stderr.write(`[server] ${data}`));
  url = `http://127.0.0.1:${port}/`;
  await waitForServer(url);
}

let browser = null;
try {
  if (!url) await startServer();

  try {
    browser = await chromium.launch({
      headless: softwareGpu,
      // Real-GPU by default; the SwiftShader args only appear when the caller
      // explicitly opts into a labelled SOFTWARE run via YACR_SOFTWARE_GPU=1.
      args: softwareGpu
        ? [
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--enable-unsafe-swiftshader",
            "--use-gl=angle",
            "--use-angle=swiftshader",
          ]
        : ["--no-sandbox", "--disable-dev-shm-usage"],
    });
  } catch (error) {
    report.blocker = `headful chromium could not start: ${error}`;
    console.error(report.blocker);
    writeFileSync(`${output}/report.json`, JSON.stringify(report, null, 2));
    process.exit(2);
  }

  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    acceptDownloads: true,
  });
  const page = await context.newPage();
  page.setDefaultTimeout(60000);
  const isHandoff = (text) =>
    typeof text === "string" && text.includes("Using exceptions for control flow");
  page.on("pageerror", (error) => {
    if (!isHandoff(String(error))) {
      report.errors.push(String(error));
    }
  });
  // Console errors are unexpected too (mirrors check-web-ui.mjs): a page error
  // alone would miss a failed fetch/asset or an app console.error.
  const consoleErrors = [];
  report.consoleErrors = consoleErrors;
  page.on("console", (message) => {
    const line = `${message.type()}: ${message.text()}`;
    if (message.type() === "error" && !isHandoff(line)) consoleErrors.push(line);
  });

  await page.goto(url, { waitUntil: "load" });

  // Real-GPU evidence: the unmasked GL renderer must not be a software backend.
  report.glRenderer = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2") || canvas.getContext("webgl");
    if (!gl) return null;
    const ext = gl.getExtension("WEBGL_debug_renderer_info");
    return ext
      ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
      : gl.getParameter(gl.RENDERER);
  });
  report.softwareRenderer = /swiftshader|llvmpipe|software|basic render/i.test(
    report.glRenderer || "",
  );

  const state = () => page.evaluate(() => window.yacr.renderer_state_report());
  const entities = async () => Number((await state()).match(/entities=(\d+)/)[1]);

  await page.waitForFunction(
    () => {
      const s = window.yacr?.renderer_state_report();
      return s?.includes("adapter=Some") && /cad_frames=(\d+)/.test(s);
    },
    null,
    { timeout: 60000 },
  );
  await page.waitForFunction(
    () => Number(window.yacr.renderer_state_report().match(/cad_frames=(\d+)/)[1]) >= 1,
    null,
    { timeout: 60000 },
  );
  report.renderer = await state();

  const bounds = await page.locator("#canvas").boundingBox();
  // The command bar sits just below the CAD rect; `shell_geometry` returns the
  // CAD rect `[x, y, width, height, ...]` in canvas-local logical pixels.
  const geometry = await page.evaluate(() =>
    Array.from(window.yacr.shell_geometry()),
  );
  const commandBarX = bounds.x + geometry[0] + geometry[2] / 2;
  const commandBarY = bounds.y + geometry[1] + geometry[3] + 20;
  const focusCommandBar = async () => {
    await page.mouse.click(commandBarX, commandBarY);
    await page.waitForTimeout(80);
  };
  const command = async (text) => {
    await focusCommandBar();
    await page.keyboard.type(text);
    await page.waitForTimeout(150);
    await page.keyboard.press("Enter");
    await page.waitForTimeout(400);
  };
  const commandDownload = async (text) => {
    await focusCommandBar();
    await page.keyboard.type(text);
    await page.waitForTimeout(150);
    const [download] = await Promise.all([
      page.waitForEvent("download", { timeout: 30000 }),
      page.keyboard.press("Enter"),
    ]);
    return download;
  };
  const waitEntities = (count) =>
    page.waitForFunction(
      (expected) =>
        Number(window.yacr.renderer_state_report().match(/entities=(\d+)/)[1]) ===
        expected,
      count,
      { timeout: 30000 },
    );

  // The plot fixture carries a paper-space layout but no model-space bounds, so
  // the axes/grid overlay reports an explicit diagnostic (`error=Some(...)`);
  // the entity count is the load evidence, not `error=None`.
  const upload = async (expected) => {
    await page.locator("#file-input").setInputFiles(fixturePath);
    await waitEntities(expected);
    // The entity count is published before the GPU upload; wait for a real frame.
    await page.waitForTimeout(1000);
  };

  // 1. Upload a committed fixture and prove it loads (the demo starts at 6).
  const initialEntities = await entities();
  await page.locator("#file-input").setInputFiles(fixturePath);
  await page.waitForFunction(
    (initial) => {
      const n = Number(window.yacr.renderer_state_report().match(/entities=(\d+)/)[1]);
      return n > 0 && n !== initial;
    },
    initialEntities,
    { timeout: 60000 },
  );
  const loaded = await entities();
  report.steps.push({ step: "upload", entities: loaded });
  assert.ok(loaded > 0 && loaded !== initialEntities, "the fixture must load");

  // 2. New replaces the document with a blank drawing.
  await command("NEW");
  await waitEntities(0);
  report.steps.push({ step: "new", entities: await entities() });
  assert.equal(await entities(), 0, "NEW must install a blank document");
  await page.screenshot({ path: `${output}/02-new-blank.png` });

  // 3. Re-upload so Save/Plot operate on real geometry + a paper layout.
  await upload(loaded);
  const reloaded = await entities();
  report.steps.push({ step: "reupload", entities: reloaded });
  assert.equal(reloaded, loaded, "the fixture must reload after NEW");
  await page.screenshot({ path: `${output}/03-loaded.png` });

  // 4. Save As DXF: a real download whose bytes are an ASCII DXF document.
  const dxfDownload = await commandDownload("SAVE");
  const dxfName = dxfDownload.suggestedFilename();
  const dxfBytes = readFileSync(await dxfDownload.path());
  writeFileSync(`${output}/${dxfName}`, dxfBytes);
  report.steps.push({
    step: "save",
    filename: dxfName,
    bytes: dxfBytes.length,
    sha256: sha256(dxfBytes),
  });
  assert.match(dxfName, /\.dxf$/i, "SAVE must download a .dxf file");
  assert.ok(contains(dxfBytes, "SECTION"), "the DXF must contain a SECTION marker");
  assert.ok(contains(dxfBytes, "EOF"), "the DXF must contain an EOF marker");

  // 5. Plot SVG: the default vector format.
  const svgDownload = await commandDownload("PLOT");
  const svgName = svgDownload.suggestedFilename();
  const svgBytes = readFileSync(await svgDownload.path());
  writeFileSync(`${output}/${svgName}`, svgBytes);
  report.steps.push({
    step: "plot-svg",
    filename: svgName,
    bytes: svgBytes.length,
    sha256: sha256(svgBytes),
  });
  assert.match(svgName, /\.svg$/i, "PLOT must default to a .svg download");
  assert.ok(contains(svgBytes, "<svg"), "the SVG must contain the <svg> root");
  assert.ok(
    contains(svgBytes, "<polyline") ||
      contains(svgBytes, "<polygon") ||
      contains(svgBytes, "<path"),
    "the SVG must contain stroke geometry, not just a prolog",
  );

  // 6. Switch the plot format to PDF through the config API, then plot again.
  const configResult = await page.evaluate(() =>
    window.yacr.updateConfig({ ui: { export: { plotFormat: "pdf" } } }),
  );
  report.plotFormat = configResult;
  assert.equal(configResult.ok, true, "the plotFormat patch must be accepted");
  assert.equal(
    configResult.config.ui.export.plotFormat,
    "pdf",
    "the effective config must carry the pdf plot format",
  );
  const pdfDownload = await commandDownload("PLOT");
  const pdfName = pdfDownload.suggestedFilename();
  const pdfBytes = readFileSync(await pdfDownload.path());
  writeFileSync(`${output}/${pdfName}`, pdfBytes);
  report.steps.push({
    step: "plot-pdf",
    filename: pdfName,
    bytes: pdfBytes.length,
    sha256: sha256(pdfBytes),
  });
  assert.match(pdfName, /\.pdf$/i, "PLOT with plotFormat=pdf must download a .pdf");
  assert.ok(pdfBytes.subarray(0, 5).equals(Buffer.from("%PDF-")), "PDF must start with %PDF-");
  assert.ok(
    contains(pdfBytes, "stream") && contains(pdfBytes, "xref"),
    "the PDF must contain a content stream and an xref table",
  );
  assert.ok(
    contains(pdfBytes, " l\n") || contains(pdfBytes, " re\n"),
    "the PDF must contain path operators (l/re), not just an empty page",
  );

  report.finalState = await state();
  assert.match(report.finalState, /adapter=Some/);
  assert.deepEqual(report.errors, [], "no unexpected page errors");
  assert.deepEqual(report.consoleErrors, [], "no unexpected console errors");
  if (report.softwareRenderer && !softwareGpu) {
    throw new Error(
      `real GPU required, but renderer is software: ${report.glRenderer}. ` +
        "Set YACR_SOFTWARE_GPU=1 to opt into SwiftShader (the run is then labelled SOFTWARE).",
    );
  }
  report.status = "passed";
  await context.close();
} catch (error) {
  report.failure = String(error?.stack || error);
  process.exitCode = 1;
} finally {
  writeFileSync(`${output}/report.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  if (browser) await browser.close().catch(() => {});
  if (server) server.kill("SIGKILL");
}
