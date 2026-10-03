// Real Slint capture -> host -> drawing transaction -> GPU, software WebGL2 only.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { decodePng } from "./web-pixels.mjs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const chromium = playwright.chromium || playwright.default.chromium;
const url = process.argv[2] || "http://127.0.0.1:8099/";
const output = process.argv[3] || "/tmp/opencode/yacr-drawing-validation";
mkdirSync(output, { recursive: true });
const results = [];
for (const kind of process.argv[4]
  ? [process.argv[4]]
  : ["LINE", "CIRCLE", "MOVE", "TRIM"]) {
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
  try {
    const context = await browser.newContext({
      viewport: { width: 1280, height: 800 },
    });
    const page = await context.newPage();
    page.setDefaultTimeout(20000);
    const cdp = await context.newCDPSession(page);
    const errors = [];
    page.on("pageerror", (error) => {
      errors.push(String(error));
      console.error(error);
    });
    await page.goto(url);
    await page.waitForFunction(
      () => window.yacr?.renderer_state_report().includes("adapter=Some"),
      null,
      { timeout: 60000 },
    );
    const idleHeight = await page.evaluate(
      () => window.yacr.shell_geometry()[3],
    );
    const report = () =>
      page.evaluate(() => window.yacr.renderer_state_report());
    const count = async () =>
      Number((await report()).match(/entities=(\d+)/)[1]);
    const command = async (text, expectClose = true) => {
      const bounds = await page.locator("#canvas").boundingBox();
      await page.mouse.click(
        bounds.x + bounds.width * 0.6,
        bounds.y + bounds.height - 28,
      );
      await page.keyboard.type(text);
      await page.keyboard.press("Enter");
      await page.waitForTimeout(350);
      if (expectClose && (text === "CONFIRM" || text === "CANCEL")) {
        assert.equal(
          await page.evaluate(() => window.yacr.shell_geometry()[3]),
          idleHeight,
          `${text} must close capture and clear preview; rejected commits must not pass`,
        );
      }
    };
    const capture = async (name) => {
      await page.mouse.move(1, 1);
      await page.waitForTimeout(400);
      let timer;
      let data;
      try {
        ({ data } = await Promise.race([
          cdp.send("Page.captureScreenshot", {
            format: "png",
            fromSurface: false,
          }),
          new Promise((_, reject) => {
            timer = setTimeout(
              () => reject(new Error(`capture stalled: ${name}`)),
              20000,
            );
          }),
        ]));
      } finally {
        clearTimeout(timer);
      }
      const png = Buffer.from(data, "base64");
      writeFileSync(`${output}/${name}.png`, png);
      // SwiftShader fromSurface:false can ignore CDP's clip. Hash decoded CAD
      // pixels, not the whole PNG (toolbar focus/history also change after undo).
      const image = decodePng(png);
      const hash = createHash("sha256");
      for (let y = 240; y < 560; y += 1) {
        hash.update(
          image.pixels.subarray(
            y * image.stride + 256 * image.channels,
            y * image.stride + 1024 * image.channels,
          ),
        );
      }
      return hash.digest("hex");
    };
    let initialCount = await count();
    if (kind === "TRIM") {
      for (const points of [
        [
          [450, 400],
          [800, 400],
        ],
        [
          [640, 330],
          [640, 480],
        ],
      ]) {
        await command("LINE");
        for (const [x, y] of points) await page.mouse.click(x, y);
        await command("CONFIRM");
      }
      assert.equal(await count(), initialCount + 2);
      initialCount += 2;
    }
    // The synthetic demo starts in Work mode; all actions use real shell callbacks.
    const baseline = await capture(`${kind}-baseline`);
    if (kind === "LINE" || kind === "CIRCLE") {
      await command(kind);
      const geometry = await page.evaluate(() =>
        Array.from(window.yacr.shell_geometry()),
      );
      await page.mouse.click(
        geometry[2] * 0.4,
        geometry[1] + geometry[3] * 0.42,
      );
      await page.mouse.click(
        geometry[2] * 0.6,
        geometry[1] + geometry[3] * 0.58,
      );
      assert.equal(
        await count(),
        initialCount,
        "capture must not write before confirmation",
      );
      await command("CONFIRM");
      assert.equal(
        await count(),
        initialCount + 1,
        `${kind} must commit exactly one entity`,
      );
      const created = await capture(`${kind}-created`);
      assert.notEqual(created, baseline, `${kind} must change CAD pixels`);
      await command("UNDO");
      assert.equal(await count(), initialCount);
      const undone = await capture(`${kind}-undone`);
      assert.equal(
        undone,
        baseline,
        `${kind} undo must restore baseline CAD pixels`,
      );
      await command("REDO");
      assert.equal(await count(), initialCount + 1);
      await command("UNDO");
      await command(kind);
      await page.mouse.click(500, 350);
      await command("CANCEL");
      assert.equal(await count(), initialCount, "cancel must not write");
      results.push({ kind, baseline, created, undone });
    } else if (kind === "MOVE") {
      // MOVE must retain the selection while its two anchor points are captured.
      const bounds = await page.locator("#canvas").boundingBox();
      const geometry = await page.evaluate(() =>
        Array.from(window.yacr.shell_geometry()),
      );
      const wpp = Number((await report()).match(/wpp=([\d.]+)/)[1]);
      await page.mouse.click(
        bounds.x + geometry[2] / 2 - 800 / wpp,
        bounds.y + geometry[1] + geometry[3] / 2,
      );
      await command("MOVE");
      await page.mouse.click(500, 400);
      await page.mouse.click(560, 430);
      await command("CONFIRM");
      assert.equal(await count(), initialCount);
      // Clear the highlight so only the modified base geometry is compared.
      await page.mouse.click(60, 400);
      const moved = await capture("MOVE-created");
      assert.notEqual(
        moved,
        baseline,
        "MOVE must change base geometry, not only highlight",
      );
      await command("UNDO");
      assert.equal(await capture("MOVE-undone"), baseline);
      await command("REDO");
      results.push({ kind: "MOVE", baseline, created: moved });
    } else {
      await command("TRIM");
      await page.mouse.click(500, 400);
      await page.mouse.click(640, 450);
      await command("CONFIRM");
      const trimmed = await capture("TRIM-created");
      assert.notEqual(trimmed, baseline, "TRIM must change base line geometry");
      assert.equal(await count(), initialCount);
      await command("UNDO");
      assert.equal(await capture("TRIM-undone"), baseline);
      await command("REDO");
      // A curved TRIM target is refused; the host must return the error rather
      // than clearing the tool and pretending to have committed successfully.
      await command("TRIM");
      const bounds = await page.locator("#canvas").boundingBox();
      const geometry = await page.evaluate(() =>
        Array.from(window.yacr.shell_geometry()),
      );
      const wpp = Number((await report()).match(/wpp=([\d.]+)/)[1]);
      const y = bounds.y + geometry[1] + geometry[3] / 2;
      await page.mouse.click(bounds.x + geometry[2] / 2 - 800 / wpp, y);
      await page.mouse.click(bounds.x + geometry[2] / 2 - 2000 / wpp, y);
      await command("CONFIRM", false);
      assert.equal(await count(), initialCount);
      assert.notEqual(
        await page.evaluate(() => window.yacr.shell_geometry()[3]),
        idleHeight,
        "refused TRIM must retain capture parameters",
      );
      await command("CANCEL");
      results.push({ kind: "TRIM", baseline, created: trimmed });
    }
    assert.deepEqual(errors, []);
    console.log(`${kind} passed`);
    await context.close();
  } finally {
    server.process().kill("SIGKILL");
  }
}
writeFileSync(
  `${output}/report.json`,
  JSON.stringify({ passed: true, results }, null, 2),
);
console.log(JSON.stringify({ passed: true, output, results }));
