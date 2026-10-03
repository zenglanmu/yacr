// Slint ribbon/command layout plus real Chromium multi-touch (no display server).
import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
const playwright = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = playwright.chromium || playwright.default.chromium;
const launchOptions = {
  headless: true,
  args: [
    "--no-sandbox",
    "--enable-unsafe-swiftshader",
    "--use-gl=angle",
    "--use-angle=swiftshader",
  ],
};
const server = await chromium.launchServer(launchOptions);
const browser = await chromium.connect(server.wsEndpoint());
const url = process.argv[2] || "http://127.0.0.1:8099/";
const output = process.argv[3] || "/tmp/opencode/yacr-ribbon-validation";
mkdirSync(output, { recursive: true });
const results = [];
try {
  for (const [name, width, height, dpr] of [
    ["desktop", 1280, 800, 1],
    ["phone", 390, 844, 3],
    ["small-phone", 320, 740, 2],
  ].filter(([name]) => !process.argv[4] || name === process.argv[4])) {
    const context = await browser.newContext({
      viewport: { width, height },
      deviceScaleFactor: dpr,
      hasTouch: true,
      isMobile: dpr > 1,
    });
    const page = await context.newPage();
    await page.bringToFront();
    page.setDefaultTimeout(20000);
    const cdp = await context.newCDPSession(page);
    const screenshot = async (name) => {
      console.log("snapshot", name);
      await page.mouse.move(1, 1);
      // Force a browser compositor frame: Slint's on-demand canvas can otherwise
      // leave Chromium/SwiftShader screenshot capture waiting indefinitely.
      const { data } = await cdp.send("Page.captureScreenshot", {
        format: "png",
        fromSurface: false,
      });
      writeFileSync(`${output}/${name}.png`, Buffer.from(data, "base64"));
    };
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto(url);
    await page.waitForFunction(
      () => window.yacr?.renderer_state_report().includes("adapter=Some"),
      null,
      { timeout: 60000 },
    );
    await page.waitForTimeout(500);
    const geometry = () =>
      page.evaluate(() => Array.from(window.yacr.shell_geometry()));
    const initial = await geometry();
    assert.equal(initial[4], dpr > 1 ? 1 : 0);
    if (dpr > 1) assert.equal(initial[1], 0, "phone starts with no ribbon");
    assert.ok(initial[3] > height * 0.55, "canvas gets most of viewport");
    await screenshot(`${name}-default`);
    let bounds = await page.locator("#canvas").boundingBox();
    // Expand command history; the persistent row remains visible when collapsed.
    await page.mouse.click(
      bounds.x + width - 28,
      bounds.y + bounds.height - 28,
    );
    await page.waitForTimeout(150);
    assert.equal((await geometry())[7], 1);
    await page.mouse.click(
      bounds.x + width - 28,
      bounds.y + bounds.height - 94,
    );
    await page.waitForTimeout(150);
    assert.equal((await geometry())[7], 0);
    // Command entry exercises the same callbacks as ribbon buttons, not a fake API.
    await page.mouse.click(
      bounds.x + width * 0.5,
      bounds.y + bounds.height - 28,
    );
    await page.keyboard.type("TOOLS");
    await page.keyboard.press("Enter");
    await page.waitForTimeout(2300);
    const expanded = await geometry();
    if (dpr > 1) assert.equal(expanded[5], 1);
    else assert.equal(expanded[6], 0, "desktop command collapses ribbon");
    bounds = await page.locator("#canvas").boundingBox();
    await page.mouse.click(
      bounds.x + width * 0.5,
      bounds.y + bounds.height - 28,
    );
    await page.keyboard.type("TOOLS");
    await page.keyboard.press("Enter");
    await page.waitForTimeout(2300);
    bounds = await page.locator("#canvas").boundingBox();
    // Open from the Slint command row: the real File API chooser must fire.
    const chooser = page.waitForEvent("filechooser");
    await page.mouse.click(bounds.x + 40, bounds.y + bounds.height - 28);
    await (await chooser).setFiles("fixtures/dwg/synthetic-four-lines.dwg");
    await page.waitForFunction(() =>
      window.yacr.renderer_state_report().includes("entities=4"),
    );
    await page.waitForTimeout(400);
    const before = await page.evaluate(() =>
      window.yacr.renderer_state_report(),
    );
    const wpp = (report) => Number(report.match(/wpp=([\d.]+)/)[1]);
    const rect = await geometry();
    const x = bounds.x + width * 0.5,
      y = bounds.y + rect[1] + rect[3] * 0.5;
    const touch = (type, points) =>
      cdp.send("Input.dispatchTouchEvent", {
        type,
        touchPoints: points.map(([px, py], id) => ({
          x: px,
          y: py,
          id,
          radiusX: 3,
          radiusY: 3,
        })),
      });
    await touch("touchStart", [
      [x - 35, y],
      [x + 35, y],
    ]);
    await touch("touchMove", [
      [x - 70, y],
      [x + 70, y],
    ]);
    await touch("touchEnd", []);
    await page.waitForTimeout(300);
    const afterPinch = await page.evaluate(() =>
      window.yacr.renderer_state_report(),
    );
    assert.ok(
      wpp(afterPinch) < wpp(before) * 0.8,
      "two fingers zoom CAD, not browser page",
    );
    await touch("touchStart", [[x, y]]);
    await touch("touchMove", [[x + 50, y + 25]]);
    await touch("touchEnd", []);
    await page.waitForTimeout(300);
    const afterDrag = await page.evaluate(() =>
      window.yacr.renderer_state_report(),
    );
    assert.ok(
      Number(afterDrag.match(/cad_frames=(\d+)/)[1]) >
        Number(afterPinch.match(/cad_frames=(\d+)/)[1]),
    );
    assert.equal(wpp(afterDrag), wpp(afterPinch), "drag pans without zoom");
    assert.notEqual(
      afterDrag.match(/center=(\[[^\]]+\])/)[1],
      afterPinch.match(/center=(\[[^\]]+\])/)[1],
      "drag changes the authoritative CAD camera center",
    );
    assert.deepEqual(errors, []);
    results.push({
      name,
      initial,
      expanded,
      before,
      afterPinch,
      afterDrag,
      errors,
    });
    await context.close();
  }
  writeFileSync(
    `${output}/report.json`,
    JSON.stringify({ passed: true, results }, null, 2),
  );
  console.log(JSON.stringify({ passed: true, output }));
} finally {
  // Explicitly terminate only this test's browser; software GPU shutdown may
  // hang after touch/compositor capture even when all assertions have completed.
  server.process().kill("SIGKILL");
}
