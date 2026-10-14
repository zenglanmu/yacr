// Real browser proof that stale/multitouch capture cannot be confirmed later.
import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = playwright.chromium || playwright.default.chromium;
const url = process.argv[2] || "http://127.0.0.1:8099/";
const output = process.argv[3] || "/tmp/opencode/yacr-drawing-safety";
mkdirSync(output, { recursive: true });
// Real-GPU by default (headful, like check-web-files.mjs). SwiftShader software
// rendering is an explicit opt-in: set YACR_SOFTWARE_GPU=1, and the run is then
// labelled SOFTWARE everywhere and must never be presented as real-GPU evidence.
const softwareGpu = process.env.YACR_SOFTWARE_GPU === "1";
const launchOptions = softwareGpu
  ? {
      headless: true,
      args: [
        "--no-sandbox",
        "--enable-unsafe-swiftshader",
        "--use-gl=angle",
        "--use-angle=swiftshader",
      ],
    }
  : { headless: false, args: ["--no-sandbox"] };
if (softwareGpu) {
  console.error(
    "YACR_SOFTWARE_GPU=1: SOFTWARE SwiftShader run - not real-GPU evidence.",
  );
}
const server = await chromium.launchServer(launchOptions);
try {
  const browser = await chromium.connect(server.wsEndpoint());
  const page = await browser.newPage({
    viewport: { width: 1280, height: 800 },
    hasTouch: true,
  });
  page.setDefaultTimeout(20000);
  const cdp = await page.context().newCDPSession(page);
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.goto(url);
  await page.waitForFunction(
    () => window.yacr?.renderer_state_report().includes("adapter=Some"),
    null,
    { timeout: 60000 },
  );
  // Real-GPU evidence: the unmasked GL renderer must not be a software backend.
  const glRenderer = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2") || canvas.getContext("webgl");
    if (!gl) return null;
    const ext = gl.getExtension("WEBGL_debug_renderer_info");
    return ext
      ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
      : gl.getParameter(gl.RENDERER);
  });
  const softwareRenderer = /swiftshader|llvmpipe|software|basic render/i.test(
    glRenderer || "",
  );
  if (softwareRenderer && !softwareGpu) {
    throw new Error(
      `real GPU required, but the browser reported a software renderer: ${glRenderer}. ` +
        "Set YACR_SOFTWARE_GPU=1 to opt into SwiftShader (the run is then labelled SOFTWARE).",
    );
  }
  const rendererInfo = {
    glRenderer,
    software: softwareRenderer,
    mode: softwareGpu ? "software" : "real-gpu",
  };
  const height = () => page.evaluate(() => window.yacr.shell_geometry()[3]);
  const idleHeight = await height();
  const count = async () =>
    Number(
      (await page.evaluate(() => window.yacr.renderer_state_report())).match(
        /entities=(\d+)/,
      )[1],
    );
  const initialCount = await count();
  const command = async (text) => {
    const bounds = await page.locator("#canvas").boundingBox();
    await page.mouse.click(
      bounds.x + bounds.width * 0.6,
      bounds.y + bounds.height - 28,
    );
    await page.keyboard.type(text);
    await page.keyboard.press("Enter");
    await page.waitForTimeout(350);
  };
  const capture = async () => {
    await command("LINE");
    await page.mouse.click(500, 350);
    await page.mouse.click(700, 430);
    assert.ok(
      (await height()) < idleHeight,
      "two points leave a confirmable capture",
    );
    assert.equal(await count(), initialCount);
  };
  const touch = (type, points) =>
    cdp.send("Input.dispatchTouchEvent", {
      type,
      touchPoints: points.map(([x, y], id) => ({
        x,
        y,
        id,
        radiusX: 3,
        radiusY: 3,
      })),
    });
  await capture();
  await touch("touchStart", [[600, 400]]);
  await touch("touchStart", [
    [600, 400],
    [650, 400],
  ]);
  await page.waitForTimeout(300);
  assert.equal(
    await height(),
    idleHeight,
    "second finger cancels before any movement",
  );
  await touch("touchEnd", []);
  await command("CONFIRM");
  assert.equal(await count(), initialCount);

  await capture();
  await touch("touchStart", [[600, 400]]);
  await touch("touchCancel", []);
  await command("CONFIRM");
  assert.equal(await height(), idleHeight);
  assert.equal(await count(), initialCount);

  await capture();
  await page
    .locator("#file-input")
    .setInputFiles("fixtures/dwg/synthetic-four-lines.dwg");
  await page.waitForFunction(() =>
    window.yacr.renderer_state_report().includes("entities=4"),
  );
  await page.waitForTimeout(300);
  assert.equal(
    await height(),
    idleHeight,
    "successful document replacement clears old capture",
  );
  await command("CONFIRM");
  assert.equal(
    await count(),
    4,
    "old points must not write into the newly opened drawing",
  );
  assert.deepEqual(errors, []);
  const result = {
    passed: true,
    secondFinger: true,
    touchCancel: true,
    documentReplacement: true,
    errors,
    renderer: rendererInfo,
  };
  writeFileSync(`${output}/report.json`, JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} finally {
  server.process().kill("SIGKILL");
}
