// Real headless Chromium responsive/DWG/startup regression. Playwright is external.
import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
const module = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = module.chromium || module.default.chromium;
const url = process.argv[2] || "http://127.0.0.1:8098/";
const output = process.argv[3] || "/tmp/opencode/yacr-mobile-validation";
mkdirSync(output, { recursive: true });
const server = await chromium.launchServer({
  headless: true,
  args: [
    "--no-sandbox",
    "--enable-unsafe-swiftshader",
    "--use-gl=angle",
    "--use-angle=swiftshader",
  ],
});
const browser = await chromium.connect(server.wsEndpoint());
const results = [];
try {
  for (const [name, width, height, dpr] of [
    ["phone", 390, 844, 3],
    ["small-phone", 320, 740, 2],
    ["desktop", 1280, 800, 1],
  ].filter(([name]) => !process.argv[4] || name === process.argv[4])) {
    const context = await browser.newContext({
      viewport: { width, height },
      deviceScaleFactor: dpr,
      isMobile: dpr !== 1,
      hasTouch: dpr !== 1,
    });
    const page = await context.newPage();
    await page.bringToFront();
    page.setDefaultTimeout(20000);
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    await page.goto(url);
    console.log(name, "startup");
    await page.waitForFunction(
      () => window.yacr?.renderer_state_report().includes("adapter=Some"),
      null,
      { timeout: 60000 },
    );
    await page.waitForTimeout(1000);
    const geometry = await page.evaluate(() => {
      const canvas = document.getElementById("canvas");
      const rect = canvas.getBoundingClientRect();
      return {
        width: canvas.width,
        height: canvas.height,
        cssWidth: rect.width,
        cssHeight: rect.height,
        report: window.yacr.renderer_state_report(),
        error: window.yacrStartupError,
      };
    });
    console.log(name, "ready", geometry.cssWidth, geometry.cssHeight);
    assert.equal(geometry.error, undefined);
    assert.equal(geometry.width, Math.round(geometry.cssWidth * dpr));
    assert.equal(geometry.height, Math.round(geometry.cssHeight * dpr));
    assert.match(geometry.report, /error=None/);
    const surface = geometry.report.match(
      /surface=Some\(\(\[([\d.]+), ([\d.]+)\]/,
    );
    assert.ok(
      surface &&
        Number(surface[1]) > width * 0.8 &&
        Number(surface[2]) > height * 0.4,
    );
    // Default-layout screenshots live in check-web-ribbon.mjs. Capture only
    // the final rotated state here to avoid repeated SwiftShader readbacks.
    if (dpr !== 1) {
      const rect = await page.locator("#canvas").boundingBox();
      await page.mouse.click(rect.x + width * 0.5, rect.y + rect.height - 30);
      await page.keyboard.type("TOOLS");
      await page.keyboard.press("Enter");
      console.log(name, "TOOLS", await page.evaluate(() => Array.from(window.yacr.shell_geometry())));
      await page.locator("#language").waitFor({ state: "visible" });
    }
    const framesBeforeUi = await page.evaluate(
      () => window.yacr.renderer_state_report().match(/cad_frames=(\d+)/)?.[1],
    );
    await page.locator("#language").selectOption("en");
    console.log(name, "locale");
    await page.waitForTimeout(300);
    const framesAfterUi = await page.evaluate(
      () => window.yacr.renderer_state_report().match(/cad_frames=(\d+)/)?.[1],
    );
    assert.ok(framesBeforeUi, "runtime exposes render count");
    assert.equal(
      framesAfterUi,
      framesBeforeUi,
      "language/UI-only redraw must reuse CAD pixels",
    );
    assert.equal(await page.locator("#open-drawing").textContent(), "Open DWG");
    const chooser = page.waitForEvent("filechooser");
    await page.locator("#open-drawing").click();
    await (await chooser).setFiles("fixtures/dwg/synthetic-four-lines.dwg");
    console.log(name, "import");
    await page.waitForFunction(() =>
      window.yacr.renderer_state_report().includes("entities=4"),
    );
    await page.waitForTimeout(400);
    assert.match(
      await page.evaluate(() => window.yacr.renderer_state_report()),
      /error=None/,
    );
    await page.setViewportSize({ width: height, height: width });
    await page.waitForTimeout(500);
    const resized = await page.evaluate(() => ({
      width: canvas.width,
      css: canvas.getBoundingClientRect().width,
      report: window.yacr.renderer_state_report(),
    }));
    assert.equal(resized.width, Math.round(resized.css * dpr));
    assert.match(resized.report, /entities=4/);
    // Rotation is asserted from backing-store size and retained document above.
    // Repeated dynamic high-DPI SwiftShader captures can stall Chromium 153;
    // default/desktop pixel evidence is captured by the companion scripts.
    assert.deepEqual(errors, []);
    results.push({ name, geometry, resized, errors });
    await context.close();
  }
  const failure = await browser.newPage();
  await failure.route("**/pkg/yacr.js", (route) => route.abort());
  await failure.goto(url);
  await failure
    .locator("#retry-renderer")
    .waitFor({ state: "visible", timeout: 60000 });
  assert.match(
    await failure.locator("#host-state").textContent(),
    /失败|failed/i,
  );
  await failure.screenshot({ path: `${output}/startup-failed.png` });
  await failure.close();
  writeFileSync(
    `${output}/report.json`,
    JSON.stringify({ passed: true, results }, null, 2),
  );
  console.log(
    JSON.stringify({
      passed: true,
      scenarios: results.map((r) => r.name),
      output,
    }),
  );
} finally {
  server.process().kill("SIGKILL");
}
