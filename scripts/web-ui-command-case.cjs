#!/usr/bin/env node
/*
 * The **command** acceptance case of `scripts/web-ui-smoke.cjs`: the desktop
 * command set, driven through the real chrome on the `dom-canvas` carrier.
 *
 * Split into its own file because `scripts/check-file-size.py` bounds every code
 * file at 1000 lines; this module owns one case and nothing else. It is not a
 * second harness: `web-ui-smoke.cjs` passes its page helpers in, so the browser
 * launch, the server, the reporting and every other assertion stay where they were.
 *
 * What the case proves (and only this):
 *   - the session advertises the three camera selectors and reports `canFit: true`
 *     with a document;
 *   - `ZOOMIN`/`FIT`/`ZOOMOUT` typed into `#cad-command` + Enter change the live
 *     camera scale by exactly one step, with no error banner;
 *   - **`LINE` runs end to end through the UI**: the tool starts from the command
 *     line, two Playwright clicks on the CAD partition are accepted by the
 *     session's own input seam (`tool.points` 1 → 2, which only the shared
 *     `probe_input` lane can produce), Enter finishes the command, the session's
 *     `entityCount` grows and the composited canvas pixels change (the frame loop
 *     repaints after an input, which is what makes the click visible at all);
 *   - **`OPEN` runs through the picker**: the chosen DWG's bytes reach the
 *     session's `open_bytes`, and the session then reports that file as its
 *     `documentLabel`, its `entityCount` and a larger `importCount` — the document
 *     really was replaced;
 *   - **the About overlay opens from the ribbon** as an in-DOM dialog whose gate
 *     refuses CAD input: a canvas click while it is open changes nothing, and the
 *     session's own `probe_input` answer is `accepted: false` with the modal reason;
 *   - **a refused command has one surface**: no line under the field, exactly one
 *     red row in the history popup, and that popup opens **above** the input row as
 *     an overlay — measured, not assumed: the status bar is still fully on screen,
 *     the workspace box does not move, and both popup edges are inside the window.
 *
 * What it does NOT prove: pixels as *visual correctness*, GPU behaviour,
 * performance, the X11 host or a shipped product. The run is SwiftShader (software
 * rasterisation), exactly like the rest of the smoke suite.
 */
'use strict';

const path = require('node:path');

const { measureCanvasPixels } = require('./canvas-pixels.cjs');
const { runHistoryPopupStep } = require('./web-ui-history-case.cjs');

const REPO_ROOT = path.resolve(__dirname, '..');

/** The desktop viewport this case runs at (the command line exists there). */
const COMMAND_VIEWPORT = Object.freeze({ width: 1280, height: 800 });

/**
 * The zoom step the `ZOOMIN` selector applies
 * (`cad_ui_core::session::ZOOM_IN_FACTOR`), mirrored here only to predict the
 * measured scale.
 */
const ZOOM_IN_FACTOR = 1.25;

/**
 * The drawing the `OPEN` case picks, as an **absolute** path (Playwright's
 * `setInputFiles` never resolves relative paths).
 *
 * `fixtures/dwg/synthetic-four-lines.dwg` is the only DWG in the repository
 * (20 755 bytes, four LINE entities) and the same synthetic fixture the wasm
 * `start` loads. That is deliberate: the case then proves the *command* path
 * (picker → bytes → `open_bytes` → replaced document) without depending on an
 * external sample, and `importCount` is what distinguishes the re-open of the same
 * bytes from no open at all.
 */
const OPEN_DWG = path.join(REPO_ROOT, 'fixtures', 'dwg', 'synthetic-four-lines.dwg');

/** The label the session reports after opening {@link OPEN_DWG}. */
const OPEN_DWG_LABEL = path.basename(OPEN_DWG);

/** The four LINE entities that fixture holds (`app-web`'s shell tests say 4). */
const OPEN_DWG_ENTITY_COUNT = 4;

/**
 * Where the two `LINE` picks land, as fractions of the CAD partition.
 *
 * Inside the canvas, away from the corners: a pick outside the viewport would be
 * clamped by the session (or refused), and the point of the step is the accepted
 * click, not the clamp.
 */
const LINE_PICKS = Object.freeze([
  Object.freeze({ fx: 0.40, fy: 0.45 }),
  Object.freeze({ fx: 0.62, fy: 0.55 }),
]);

const FIELD_ID = 'cad-command';


/** The live session's own `shell_status()` snapshot, parsed (or `null`). */
function readShellStatus(page) {
  return page.evaluate(() => {
    const module = window.__YACR_CANVAS_MODULE__;
    if (!module || typeof module.shell_status !== 'function') return null;
    try {
      return JSON.parse(module.shell_status());
    } catch (error) {
      return { unreadable: String(error) };
    }
  });
}

/** The camera scale the live session reports, or `null` when it reports none. */
async function readCameraScale(page) {
  const status = await readShellStatus(page);
  const scale = status?.camera?.scale;
  return typeof scale === 'number' && Number.isFinite(scale) ? scale : null;
}

/**
 * Wait until the session's camera scale is `target` ± `tolerance`.
 *
 * `target === null` accepts the first finite scale (used to read the settled
 * value). The predicate runs inside the page against the session's own
 * `shell_status()`; nothing in it is evaluated from a string, because the page CSP
 * has no `'unsafe-eval'`.
 */
async function waitForScale(page, target, tolerance, timeout) {
  const selectors = { target, tolerance };
  try {
    await page.waitForFunction(args => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      let status;
      try {
        status = JSON.parse(module.shell_status());
      } catch (error) {
        return false;
      }
      const scale = status?.camera?.scale;
      if (typeof scale !== 'number' || !Number.isFinite(scale)) return false;
      return args.target === null || Math.abs(scale - args.target) <= args.tolerance;
    }, selectors, { timeout });
  } catch (error) {
    return null;
  }
  return readCameraScale(page);
}

/** Type `text` into the command line and press Enter; returns the echoed draft. */
async function typeCommand(page, options, text) {
  const field = page.locator(`#${FIELD_ID}`);
  await field.waitFor({ state: 'visible', timeout: options.timeout });
  // A submit is in flight while the field is disabled, and the shell drops an
  // Enter that arrives then (its in-flight guard), so wait for the previous one.
  await page.waitForFunction(id => {
    const input = document.getElementById(id);
    return !!input && !input.disabled;
  }, FIELD_ID, { timeout: options.timeout });
  await field.fill(text);
  const echo = await field.inputValue();
  await field.press('Enter');
  return echo;
}

/** `|a - b| / max(|a|, |b|, 1)` — a relative difference that tolerates tiny doubles. */
function relativeDifference(a, b) {
  return Math.abs(a - b) / Math.max(Math.abs(a), Math.abs(b), 1);
}

/** The CAD partition's box in viewport CSS px (the click target). */
async function partitionBox(page) {
  return page.evaluate(() => {
    const node = document.querySelector('.native-input');
    if (!node) return null;
    const rect = node.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  });
}

/** Click one fraction of the partition (Playwright's real mouse, not a synthetic event). */
async function clickPartition(page, box, { fx, fy }) {
  await page.mouse.click(box.x + box.width * fx, box.y + box.height * fy);
}

/** The live session's captured-point count for the running tool (`null` if none). */
async function readToolPoints(page) {
  const status = await readShellStatus(page);
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object') return null;
  return typeof tool.points === 'number' ? tool.points : null;
}

/** Wait until the session reports a running tool with at least `points` picks. */
async function waitForToolPoints(page, points, timeout) {
  try {
    await page.waitForFunction(minimum => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      let status;
      try {
        status = JSON.parse(module.shell_status());
      } catch (error) {
        return false;
      }
      const tool = status?.tool;
      return !!tool && typeof tool.points === 'number' && tool.points >= minimum;
    }, points, { timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/** Wait until the chrome's own status shows an active tool (`.statusbar__tool`). */
async function waitForChromeTool(page, timeout) {
  try {
    await page.locator('.statusbar__tool').first().waitFor({ state: 'visible', timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/** Wait until the session reports no active tool. */
async function waitForNoTool(page, timeout) {
  try {
    await page.waitForFunction(() => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      try {
        return JSON.parse(module.shell_status()).tool == null;
      } catch (error) {
        return false;
      }
    }, undefined, { timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/** Wait until the session reports more entities than `before`. */
async function waitForEntityCount(page, before, timeout) {
  try {
    await page.waitForFunction(minimum => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      try {
        const count = JSON.parse(module.shell_status()).entityCount;
        return typeof count === 'number' && count > minimum;
      } catch (error) {
        return false;
      }
    }, before, { timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/**
 * Wait until the session reports more completed opens than `before`.
 *
 * This — not the label — is what says the document was **replaced**: the case opens
 * the same fixture the session started with, so the label is unchanged by design and
 * only the import counter moves.
 */
async function waitForImportCount(page, before, timeout) {
  try {
    await page.waitForFunction(minimum => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      try {
        const count = JSON.parse(module.shell_status()).importCount;
        return typeof count === 'number' && count > minimum;
      } catch (error) {
        return false;
      }
    }, before, { timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/** Wait until `documentLabel` is `label`. */
async function waitForLabel(page, label, timeout) {
  try {
    await page.waitForFunction(expected => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.shell_status !== 'function') return false;
      try {
        return JSON.parse(module.shell_status()).documentLabel === expected;
      } catch (error) {
        return false;
      }
    }, label, { timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/** One value of the session's own `cad_settings` answer (through its `invoke`). */
function readSetting(page, key) {
  return page.evaluate(async name => {
    const module = window.__YACR_CANVAS_MODULE__;
    if (!module || typeof module.invoke !== 'function') return null;
    try {
      const answer = JSON.parse(await module.invoke('cad_settings', '{}'));
      const value = answer?.ok === true ? answer.value?.[name] : null;
      return value === undefined ? null : value;
    } catch (error) {
      return null;
    }
  }, key);
}

/** Wait until the session's own `cad_settings[key]` differs from `previous`. */
async function waitForSettingChange(page, key, previous, timeout) {
  try {
    await page.waitForFunction(async args => {
      const module = window.__YACR_CANVAS_MODULE__;
      if (!module || typeof module.invoke !== 'function') return false;
      try {
        const answer = JSON.parse(await module.invoke('cad_settings', '{}'));
        const value = answer?.ok === true ? answer.value?.[args.key] : null;
        return value !== args.previous;
      } catch (error) {
        return false;
      }
    }, { key, previous }, { timeout });
    return true;
  } catch (error) {
    return false;
  }
}

/** The raw PNG of the composited CAD canvas (the bytes a user would see). */
async function grabCanvas(page) {
  return page.screenshot({ clip: await page.evaluate(() => {
    const canvas = document.getElementById('cad-canvas');
    const box = canvas.getBoundingClientRect();
    return {
      x: Math.round(box.x + window.scrollX),
      y: Math.round(box.y + window.scrollY),
      width: Math.round(box.width),
      height: Math.round(box.height),
    };
  }) });
}

/**
 * Walk the chrome's ribbon to `itemId` and click it.
 *
 * The ribbon is the real affordance (no test-only hook): the tab is a `role="tab"`
 * labelled from the catalog, the item is a `role="button"` labelled the same way.
 * `locale` is pinned to `en-US` by the case's browser context, so the names are the
 * English catalog's own.
 */
async function clickRibbonItem(page, options, tabName, itemName, panelName) {
  const tab = page.getByRole('tab', { name: tabName, exact: true });
  await tab.waitFor({ state: 'visible', timeout: options.timeout });
  await tab.click();
  // The status bar repeats a few of these labels (its own Options/About buttons), so
  // the ribbon panel is named: one affordance, one click.
  const scope = panelName === undefined
    ? page
    : page.getByRole('region', { name: panelName });
  const item = scope.getByRole('button', { name: itemName, exact: true });
  await item.waitFor({ state: 'visible', timeout: options.timeout });
  await item.click();
}

/**
 * The `LINE` step: start the tool, pick two points, finish it.
 *
 * `true` when the session's own status shows the whole gesture: a tool with two
 * captured points after the clicks, and one more entity after Enter.
 */
async function runLineStep(page, options, checks, record, helpers) {
  const { checkNoErrorBanner, readErrorAlerts } = helpers;
  const box = await partitionBox(page);
  record(checks, box !== null && box.width > 50 && box.height > 50, 'the CAD partition has a clickable box', box);
  const before = await readShellStatus(page);
  const entityBefore = typeof before?.entityCount === 'number' ? before.entityCount : null;
  record(checks, entityBefore !== null, 'the live session reports an entity count to grow', {
    entityCount: before?.entityCount,
  });
  const pixelsBefore = await measureCanvasPixels(page, options.evidence, 'line-before');
  // The raw PNG is the other half of the evidence: `nonModalPixels` counts the ink,
  // the bytes prove the composite really differs.
  const shotBefore = await grabCanvas(page);

  const echo = await typeCommand(page, options, 'LINE');
  record(checks, echo === 'LINE', 'the command field echoes LINE', { typed: 'LINE', echoed: echo });
  const started = await waitForToolPoints(page, 0, options.timeout);
  const startedStatus = await readShellStatus(page);
  record(
    checks,
    started && startedStatus?.tool != null,
    'LINE starts a tool in the session (the command funnel reaches the router)',
    { tool: startedStatus?.tool ?? null },
  );
  // Enter is the tool's confirm key **only once the chrome's own status has the
  // tool**: the key handler reads the polled `cad_status` (4 Hz), not the session
  // directly, and a chord pressed before that poll reloads the handler belongs to
  // nobody. The status bar's tool chip is that fact made visible, so the case waits
  // for the affordance instead of sleeping.
  record(checks, await waitForChromeTool(page, options.timeout), 'the chrome shows the active tool it can confirm', null);

  if (box !== null) {
    // Two real mouse clicks on the partition: this is the pointer lane
    // (`probe_input`), not a synthetic session call.
    await clickPartition(page, box, LINE_PICKS[0]);
    const first = await waitForToolPoints(page, 1, options.timeout);
    record(checks, first, 'the first canvas click is accepted as a pick', { points: await readToolPoints(page) });
    await clickPartition(page, box, LINE_PICKS[1]);
    const second = await waitForToolPoints(page, 2, options.timeout);
    record(checks, second, 'the second canvas click is accepted as a pick', { points: await readToolPoints(page) });
  }

  // The press that confirms must not be aimed at a text field: the same Enter in
  // the command line is the line's submit, not the tool's confirm.
  const focusBefore = await page.evaluate(() => {
    const active = document.activeElement;
    return {
      hasFocus: document.hasFocus(),
      tag: active === null ? null : active.tagName,
      isTextField: active !== null && ['INPUT', 'TEXTAREA', 'SELECT'].includes(active.tagName),
    };
  });
  record(checks, focusBefore.hasFocus && focusBefore.isTextField === false, 'the keyboard belongs to the drawing, not a text field', focusBefore);
  await page.keyboard.press('Enter');
  const grew = entityBefore === null ? false : await waitForEntityCount(page, entityBefore, options.timeout);
  const ended = await waitForNoTool(page, options.timeout);
  const after = await readShellStatus(page);
  const logTail = (after?.commandLog ?? []).slice(-3).map(entry => ({
    kind: entry.kind, command: entry.command, code: entry.code,
  }));
  record(checks, grew, 'Enter finishes LINE and the session holds one more entity', {
    before: entityBefore,
    after: after?.entityCount,
    focusBefore,
    commandLogTail: logTail,
  });
  record(checks, ended, 'the tool ends after the command is finished', { tool: after?.tool ?? null });

  // The frame loop is what makes the click visible: an input asks for frames, and
  // the composited pixels must change (a session state that never repaints would
  // pass every status assertion above and still show nothing).
  const pixelsAfter = await measureCanvasPixels(page, options.evidence, 'line-after');
  record(checks, pixelsAfter.ok, 'the canvas still shows a frame after the gesture', pixelsAfter.detail);
  const inkBefore = pixelsBefore.detail?.nonModalPixels ?? null;
  const inkAfter = pixelsAfter.detail?.nonModalPixels ?? null;
  record(
    checks,
    typeof inkBefore === 'number' && typeof inkAfter === 'number' && inkAfter > inkBefore,
    'the new line adds ink to the composited canvas (the frame loop repainted)',
    { nonModalPixelsBefore: inkBefore, nonModalPixelsAfter: inkAfter },
  );
  const shotAfter = await grabCanvas(page);
  record(
    checks,
    shotBefore.length > 0 && shotAfter.length > 0 && !shotBefore.equals(shotAfter),
    'the composited canvas differs before and after the gesture',
    { bytesBefore: shotBefore.length, bytesAfter: shotAfter.length, identical: shotBefore.equals(shotAfter) },
  );
  const alerts = await readErrorAlerts(page);
  checkNoErrorBanner(checks, alerts, 'the LINE gesture');
  return { entityBefore, entityAfter: after?.entityCount ?? null, inkBefore, inkAfter };
}

/**
 * The palette / settings step: the two docked surfaces and the Options dialog.
 *
 * Both used to be hidden or refused on this carrier (`palettesAvailable: false`,
 * `cad_settings`/`cad_set_setting` unmapped). They are the session's own data now,
 * so the check is the ordinary one: `LAYER` fills the palette from `cad_layers`, and
 * the Options dialog's Apply really writes through `cad_set_setting`.
 */
async function runPanelsStep(page, options, checks, record, helpers) {
  const { checkNoErrorBanner, readErrorAlerts } = helpers;
  // `LAYER` is a chrome-local selector: the palette opens from the chrome and reads
  // the session's layer table itself.
  await typeCommand(page, options, 'LAYER');
  const palette = page.locator('section.palette.layers');
  let paletteVisible = false;
  try {
    await palette.waitFor({ state: 'visible', timeout: options.timeout });
    paletteVisible = true;
  } catch (error) {
    paletteVisible = false;
  }
  record(checks, paletteVisible, 'LAYER opens the docked layers palette on this carrier', null);
  if (paletteVisible) {
    const rows = await page.locator('.layers__list li').count();
    record(checks, rows >= 1, 'the palette is filled from the session\'s own layer table', { rows });
  }
  checkNoErrorBanner(checks, await readErrorAlerts(page), 'LAYER');

  // The Options dialog reads `cad_settings` and writes `cad_set_setting` on Apply.
  const gridBefore = await readSetting(page, 'grid');
  await clickRibbonItem(page, options, 'File', 'Options', 'Utilities');
  const overlay = page.locator('.popup-overlay[data-yacr-popup="options"]');
  let opened = false;
  try {
    await overlay.waitFor({ state: 'visible', timeout: options.timeout });
    opened = true;
  } catch (error) {
    opened = false;
  }
  record(checks, opened, 'the ribbon Options entry opens the in-DOM Options dialog', null);
  if (!opened) return;
  const gridBox = page.locator('#options-field-grid').first();
  record(checks, await gridBox.count() === 1, 'the dialog is populated from cad_settings', null);
  await gridBox.click();
  await page.getByRole('button', { name: 'Apply', exact: true }).click();
  const changed = await waitForSettingChange(page, 'grid', gridBefore, options.timeout);
  const gridAfter = await readSetting(page, 'grid');
  record(checks, changed, 'Apply writes the setting through the session (cad_set_setting)', {
    before: gridBefore, after: gridAfter,
  });
  checkNoErrorBanner(checks, await readErrorAlerts(page), 'the Options dialog');
  await page.getByRole('button', { name: 'OK', exact: true }).click();
  let closed = false;
  try {
    await overlay.waitFor({ state: 'detached', timeout: options.timeout });
    closed = true;
  } catch (error) {
    closed = false;
  }
  record(checks, closed, 'OK closes the Options dialog', null);
  record(
    checks,
    (await readShellStatus(page))?.popup == null,
    'the session cleared its modal gate after OK',
    (await readShellStatus(page))?.popup ?? null,
  );
}

/**
 * The About step: open the dialog from the ribbon, prove the gate, close it.
 *
 * The overlay is the chrome's own in-DOM layer (`data-yacr-popup`), and the gate is
 * the **session's**: while a modal popup is open every CAD interaction is refused by
 * name, so a click that reaches `.native-input` can never move the drawing.
 */
async function runAboutStep(page, options, checks, record, helpers) {
  const { checkNoErrorBanner, readErrorAlerts } = helpers;
  const opened = await readShellStatus(page);
  await clickRibbonItem(page, options, 'File', 'About', 'Utilities');

  const overlay = page.locator('.popup-overlay');
  let visible = false;
  try {
    await overlay.waitFor({ state: 'visible', timeout: options.timeout });
    visible = true;
  } catch (error) {
    visible = false;
  }
  record(checks, visible, 'the ribbon About entry opens the in-DOM overlay', visible ? null : 'overlay absent');
  if (!visible) return;
  record(
    checks,
    (await overlay.getAttribute('data-yacr-popup')) === 'about',
    'the overlay is the About dialog',
    await overlay.getAttribute('data-yacr-popup'),
  );
  record(checks, (await page.locator('.popup-dialog').count()) === 1, 'exactly one dialog is rendered', null);

  // The session's own modal gate: a CAD interaction is refused by name while the
  // dialog is open (ADR-0021 D2). Asked of the session, not of the DOM.
  const gate = await page.evaluate(() => {
    const module = window.__YACR_CANVAS_MODULE__;
    if (!module || typeof module.probe_input !== 'function') return null;
    try {
      return JSON.parse(module.probe_input(JSON.stringify({
        sequence: 1, kind: 'pointer_down', x: 10, y: 10, button: 0, shift: false,
      })));
    } catch (error) {
      return { threw: String(error) };
    }
  });
  record(
    checks,
    gate !== null && gate.accepted === false && typeof gate.reason === 'string' && gate.reason.length > 0,
    'the session refuses a CAD interaction while the dialog is open',
    gate,
  );

  // And a real click on the partition changes nothing: the overlay owns the
  // pointer, and the lane is suspended for as long as the dialog is up.
  const box = await partitionBox(page);
  const before = await readShellStatus(page);
  if (box !== null) {
    await clickPartition(page, box, LINE_PICKS[0]);
    await page.waitForTimeout(400);
  }
  const after = await readShellStatus(page);
  record(
    checks,
    before !== null && after !== null
      && before.entityCount === after.entityCount
      && JSON.stringify(before.tool ?? null) === JSON.stringify(after.tool ?? null),
    'a canvas click while the dialog is open changes nothing in the session',
    { before: { entityCount: before?.entityCount, tool: before?.tool ?? null }, after: { entityCount: after?.entityCount, tool: after?.tool ?? null } },
  );
  checkNoErrorBanner(checks, await readErrorAlerts(page), 'the About dialog');

  await page.keyboard.press('Escape');
  let closed = false;
  try {
    await overlay.waitFor({ state: 'detached', timeout: options.timeout });
    closed = true;
  } catch (error) {
    closed = false;
  }
  record(checks, closed, 'Escape closes the dialog (the overlay is gone)', null);
  record(
    checks,
    (await readShellStatus(page))?.popup == null,
    'the session cleared its modal gate',
    (await readShellStatus(page))?.popup ?? null,
  );
  return opened;
}

/**
 * One labelled case: the desktop command set, through the chrome.
 *
 * `helpers` are the smoke's own page utilities, so this module owns no second
 * browser harness (see the header).
 */
async function runCommandCase(browser, options, helpers) {
  const { record, newPage, waitForSession, readErrorAlerts, checkNoErrorBanner, takeScreenshot } = helpers;
  const checks = [];
  const bucket = { pageErrors: [], failedRequests: [], consoleLines: [] };
  const name = 'commands FIT/ZOOMIN/ZOOMOUT, LINE, OPEN, LAYER, Options, About, refusal history';
  const screenshots = [];
  const observations = [];
  // `locale` is pinned so the ribbon's tab/item names are the English catalog's
  // own words: the case clicks real affordances by their accessible names.
  const container = await browser.newContext({
    viewport: { ...COMMAND_VIEWPORT },
    deviceScaleFactor: 1,
    locale: 'en-US',
  });
  const page = await newPage(container, bucket);
  let renderer = { webgl2: false, renderer: null };
  try {
    const response = await page.goto(options.url, { waitUntil: 'load', timeout: options.timeout });
    if (response === null || !response.ok()) {
      throw new Error(`the bundle URL did not load: ${options.url} -> ${response?.status() ?? 'no response'}`);
    }
    await waitForSession(page, options.timeout);

    const advertisement = await page.evaluate(() => {
      const module = window.__YACR_CANVAS_MODULE__;
      return module && typeof module.shell_commands === 'function' ? module.shell_commands() : null;
    });
    const commands = advertisement === null ? null : JSON.parse(advertisement);
    record(
      checks,
      Array.isArray(commands) && ['FIT', 'ZOOMIN', 'ZOOMOUT', 'LINE', 'OPEN'].every(selector => commands.includes(selector)),
      'the session advertises the camera selectors and the drawing commands',
      Array.isArray(commands) ? { count: commands.length, sample: commands.slice(0, 6) } : commands,
    );
    const before = await readShellStatus(page);
    record(
      checks,
      before !== null && before.canFit === true && before.documentLabel !== null,
      'the live session reports canFit and a document (the gates FIT needs)',
      { canFit: before?.canFit, documentLabel: before?.documentLabel, entityCount: before?.entityCount },
    );

    // `FIT` first: it establishes the framing the current viewport really produces
    // (the start dock collapses after the first status, which resizes the canvas
    // without refitting), so the later comparison is like with like.
    const fitEcho = await typeCommand(page, options, 'FIT');
    record(checks, fitEcho === 'FIT', 'the command field echoes the typed selector', { typed: 'FIT', echoed: fitEcho });
    const fitted = await waitForScale(page, null, 0, options.timeout);
    record(checks, fitted !== null, 'FIT leaves the session with a usable camera', { scale: fitted });

    // ZOOMIN: one 1.25x step in, about the viewport centre.
    await typeCommand(page, options, 'ZOOMIN');
    const expectedIn = fitted === null ? null : fitted / ZOOM_IN_FACTOR;
    const zoomed = expectedIn === null
      ? null
      : await waitForScale(page, expectedIn, Math.abs(expectedIn) * 1e-9, options.timeout);
    record(
      checks,
      zoomed !== null && relativeDifference(zoomed, expectedIn) <= 1e-9,
      'ZOOMIN changes the live camera scale by exactly one step',
      { before: fitted, after: zoomed, expected: expectedIn },
    );
    const alertsAfterZoom = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterZoom, 'ZOOMIN');

    // FIT again: a refit must undo the zoom step (same rule, same retained bounds).
    await typeCommand(page, options, 'FIT');
    const refitted = fitted === null
      ? null
      : await waitForScale(page, fitted, Math.abs(fitted) * 1e-9, options.timeout);
    record(
      checks,
      refitted !== null && relativeDifference(refitted, fitted) <= 1e-9,
      'FIT returns the camera to the framing the zoom moved away from',
      { zoomed, refitted, fitted },
    );
    const alertsAfterFit = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterFit, 'FIT');

    // ZOOMOUT: the reciprocal step of ZOOMIN.
    await typeCommand(page, options, 'ZOOMOUT');
    const expectedOut = refitted === null ? null : refitted * ZOOM_IN_FACTOR;
    const zoomedOut = expectedOut === null
      ? null
      : await waitForScale(page, expectedOut, Math.abs(expectedOut) * 1e-9, options.timeout);
    record(
      checks,
      zoomedOut !== null && relativeDifference(zoomedOut, expectedOut) <= 1e-9,
      'ZOOMOUT applies the reciprocal step',
      { before: refitted, after: zoomedOut, expected: expectedOut },
    );
    const alertsAfterZoomOut = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterZoomOut, 'ZOOMOUT');

    // LINE: the whole drawing gesture through the shared input lane.
    const line = await runLineStep(page, options, checks, record, helpers);

    // OPEN: the host's file picker, then the session's own `open_bytes`.
    const importCountBefore = (await readShellStatus(page))?.importCount ?? null;
    const chooserPromise = page.waitForEvent('filechooser', { timeout: options.timeout });
    await typeCommand(page, options, 'OPEN');
    let chooser = null;
    try {
      chooser = await chooserPromise;
    } catch (error) {
      observations.push(`no file chooser: ${error.message}`);
    }
    record(checks, chooser !== null, 'OPEN opens the host file picker', chooser === null ? null : OPEN_DWG);
    if (chooser !== null) {
      await chooser.setFiles(OPEN_DWG);
      // The picker's bytes are read, then imported: `importCount` is the session's
      // own completion signal (the label is the same fixture either way).
      const imported = await waitForImportCount(page, typeof importCountBefore === 'number' ? importCountBefore : -1, options.timeout);
      record(checks, imported, 'the picked file is imported into the session', { before: importCountBefore });
      await waitForLabel(page, OPEN_DWG_LABEL, options.timeout);
      const opened = await readShellStatus(page);
      record(
        checks,
        opened !== null && opened.documentLabel === OPEN_DWG_LABEL,
        'the session reports the opened file as its document label',
        { expected: OPEN_DWG_LABEL, label: opened?.documentLabel },
      );
      record(
        checks,
        opened?.entityCount === OPEN_DWG_ENTITY_COUNT,
        'the opened drawing is the fixture, with its four LINE entities',
        { expected: OPEN_DWG_ENTITY_COUNT, entityCount: opened?.entityCount, afterLine: line.entityAfter },
      );
      record(
        checks,
        typeof opened?.importCount === 'number' && typeof importCountBefore === 'number'
          && opened.importCount > importCountBefore,
        'the open installed another document (importCount grew)',
        { before: importCountBefore, after: opened?.importCount },
      );
    }
    const alertsAfterOpen = await readErrorAlerts(page);
    checkNoErrorBanner(checks, alertsAfterOpen, 'OPEN');

    // The docked palettes and the Options dialog: the surfaces that used to be
    // hidden because the carrier reported no document state of its own.
    await runPanelsStep(page, options, checks, record, helpers);

    // About: the in-DOM dialog, and the modal gate that refuses CAD input.
    await runAboutStep(page, options, checks, record, helpers);

    // The refusal path: one row, no line under the field, and the history popup
    // as an upward overlay that leaves the status bar on screen. Its own module,
    // so this one stays under the 1000-line bound.
    await runHistoryPopupStep(page, options, {
      checks, record, helpers, observations, typeCommand, fieldId: FIELD_ID,
    });

    for (const text of [...alertsAfterZoom, ...alertsAfterFit, ...alertsAfterZoomOut, ...alertsAfterOpen]) {
      observations.push(`visible alert: ${text}`);
    }

    record(checks, bucket.pageErrors.length === 0, 'no uncaught page error during the commands', bucket.pageErrors);
    renderer = await page.evaluate(() => {
      const scratch = document.createElement('canvas');
      const gl = scratch.getContext('webgl2');
      if (!gl) return { webgl2: false, renderer: null };
      const info = gl.getExtension('WEBGL_debug_renderer_info');
      const reported = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
      return { webgl2: true, renderer: reported === null ? null : String(reported) };
    });
    const pixels = await measureCanvasPixels(page, options.evidence, 'presented-commands');
    record(checks, pixels.ok, 'the CAD canvas still shows a frame after the commands', pixels.detail);
    if (options.evidence) {
      const shot = await takeScreenshot(page, options.evidence, 'commands-open.png');
      screenshots.push(shot);
      record(checks, shot.bytes > 0, 'screenshot commands-open.png written', shot.file);
    }
  } catch (error) {
    record(checks, false, 'command case ran to completion', error.message);
  } finally {
    await container.close();
  }
  return { name, checks, bucket, renderer, screenshots, observations };
}

module.exports = {
  runCommandCase,
  OPEN_DWG,
  OPEN_DWG_LABEL,
  OPEN_DWG_ENTITY_COUNT,
  ZOOM_IN_FACTOR,
  COMMAND_VIEWPORT,
};
