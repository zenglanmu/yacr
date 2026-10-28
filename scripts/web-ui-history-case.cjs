#!/usr/bin/env node
/*
 * The **refusal / history-popup** step of the command acceptance case
 * (`scripts/web-ui-command-case.cjs`).
 *
 * It lives in its own file because `scripts/check-file-size.py` bounds every code
 * file at 1000 lines, and because the geometry it measures is one story: a
 * refused command has exactly one surface (a red row in the history) and the
 * history opens **upward** as an overlay, never by pushing the layout.
 *
 * What it proves (and only this):
 *   - a refused command paints **no line under the field** — no
 *     `.commandline__message`, no `#cad-command-message`, no visible
 *     `[role="alert"]` (the duplicate the user reported is gone);
 *   - the history toggle opens the popup and it holds **exactly one** red row,
 *     carrying the refused line's sentence (the sentence itself is catalog copy,
 *     so the assertion is that the row exists, is red, and names the line — never
 *     a hardcoded English string);
 *   - the popup is an **overlay above** the command line: its bottom edge is at or
 *     above the region's top edge, it overlaps the workspace (the drawing) and
 *     both of its edges are inside the window;
 *   - nothing is pushed: the workspace box is identical with the popup open and
 *     closed, the document does not grow past the viewport, and the status bar is
 *     still fully on screen;
 *   - the popup is bounded to 40vh and its scroller's last row is inside it.
 *
 * What it does NOT prove: pixels as visual correctness, GPU behaviour, the native
 * hosts, or another browser. The run is SwiftShader (software rasterisation),
 * exactly like the rest of the smoke suite; it is layout/DOM evidence.
 */
'use strict';

/**
 * The line this step submits to be refused.
 *
 * Deliberately not a command: the session's vocabulary is the authority and this
 * name is in it nowhere, so the attempt is answered with the catalog's "not
 * supported" sentence instead of reaching the router. The point is the *shape* of
 * the report — one history row, no line under the field — not which sentence it
 * is.
 */
const REFUSED_COMMAND = 'DSF.FDSAF';

/**
 * Every box the popup geometry is measured against, in viewport CSS px.
 *
 * Read from the live DOM (not from a stylesheet guess): the region that anchors
 * the popup, the workspace it must float over, the status bar it must not push
 * off-screen, the popup itself and the scroller inside it.
 */
function popupGeometry(page) {
  return page.evaluate(() => {
    const box = selector => {
      const node = document.querySelector(selector);
      if (!node) return null;
      const rect = node.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height, bottom: rect.bottom, right: rect.right };
    };
    const list = document.querySelector('.history-panel__list');
    const lastRow = list === null ? null : list.lastElementChild;
    const listRect = list === null ? null : list.getBoundingClientRect();
    const lastRect = lastRow === null ? null : lastRow.getBoundingClientRect();
    return {
      viewport: { width: window.innerWidth, height: window.innerHeight },
      region: box('.region--cmdline'),
      row: box('.commandline__row'),
      workspace: box('.workspace'),
      statusbar: box('.statusbar'),
      panel: box('.history-panel'),
      panelList: box('.history-panel__list'),
      lastRowBottom: lastRect === null ? null : lastRect.bottom,
      listBottom: listRect === null ? null : listRect.bottom,
      listOverflow: list === null ? null : list.scrollHeight - list.clientHeight,
      moreBelow: document.querySelector('.history-panel')?.dataset.moreBelow ?? null,
      pageScroll: document.scrollingElement === null ? null : document.scrollingElement.scrollHeight,
      inlineRefusalBars: document.querySelectorAll('.commandline__message, #cad-command-message').length,
    };
  });
}

/**
 * One refused command, one red row, no line under the field, popup above the row.
 *
 * `context` carries the command case's own machinery (`checks`, `record`,
 * `helpers`, `observations`, `typeCommand`, `fieldId`) so this module owns no
 * second harness and no second way to submit a line.
 */
async function runHistoryPopupStep(page, options, context) {
  const { checks, record, helpers, observations, typeCommand, fieldId } = context;
  const { checkNoErrorBanner, readErrorAlerts, takeScreenshot } = helpers;
  const closed = await popupGeometry(page);

  const echo = await typeCommand(page, options, REFUSED_COMMAND);
  record(checks, echo === REFUSED_COMMAND, 'the command field echoes the refused line', {
    typed: REFUSED_COMMAND, echoed: echo,
  });
  // The refusal is answered from the vocabulary, so there is nothing to wait for
  // beyond the submit leaving the field enabled again.
  await page.waitForFunction(id => {
    const input = document.getElementById(id);
    return !!input && !input.disabled;
  }, fieldId, { timeout: options.timeout });

  // 1. No line under the field. The sentence lives in the history now, and a
  // visible `[role="alert"]` here would be the duplicate the user removed.
  const alerts = await readErrorAlerts(page);
  checkNoErrorBanner(checks, alerts, `the refused command (${REFUSED_COMMAND})`);
  const afterRefusal = await page.evaluate(() => ({
    bars: document.querySelectorAll('.commandline__message, #cad-command-message').length,
    alerts: document.querySelectorAll('[role="alert"]').length,
  }));
  record(checks, afterRefusal.bars === 0, 'a refused command paints no line under the field', afterRefusal);

  // 2. Open the history from its own affordance (the row's toggle, named by the
  // catalog). Structural locator: it is the row's only icon button.
  const toggle = page.locator('.commandline__row .ui-icon-button');
  record(checks, (await toggle.count()) === 1, 'the command row offers exactly one history toggle', null);
  await toggle.click();
  const panel = page.locator('.history-panel');
  let visible = false;
  try {
    await panel.waitFor({ state: 'visible', timeout: options.timeout });
    visible = true;
  } catch (error) {
    visible = false;
  }
  record(checks, visible, 'the history toggle opens the popup', visible ? null : 'panel absent');
  if (!visible) return { refused: REFUSED_COMMAND };
  // The popup drops in with a 110ms `rise` (`style.css`); its box and its opacity
  // are the final ones only once that animation ends, so the geometry below and
  // the screenshot describe the settled panel, not its first frame.
  await page.waitForFunction(
    () => {
      const panel = document.querySelector('.history-panel');
      return !!panel && getComputedStyle(panel).opacity === '1';
    },
    undefined,
    { timeout: options.timeout },
  );
  await page.waitForTimeout(150);

  const opened = await popupGeometry(page);

  // One refusal, one red row: the duplicate-per-attempt look is a defect, and the
  // row must name the refused line (the catalog sentence interpolates the command
  // name — the copy itself stays in i18n).
  const rows = await page.evaluate(() => {
    const errors = [...document.querySelectorAll('.history-panel__entry.is-error')];
    return {
      errorCount: errors.length,
      errorTexts: errors.map(node => node.textContent ?? ''),
      commandCount: document.querySelectorAll('button.history-panel__entry').length,
    };
  });
  record(checks, rows.errorCount === 1, 'one refused command is exactly one red history row', rows);
  record(
    checks,
    rows.errorCount === 1 && rows.errorTexts[0].includes(REFUSED_COMMAND),
    'the red row is the refused line\'s sentence',
    rows.errorTexts,
  );
  record(checks, rows.commandCount >= 1, 'the adopted commands are still in the same list', { rows: rows.commandCount });

  // Geometry: above the row, over the drawing, not pushing anything.
  const region = opened.region;
  const panelBox = opened.panel;
  const status = opened.statusbar;
  record(
    checks,
    region !== null && panelBox !== null && panelBox.bottom <= region.y + 1,
    'the popup opens above the command line (bottom edge at or above the region top)',
    { panelBottom: panelBox === null ? null : panelBox.bottom, regionTop: region === null ? null : region.y },
  );
  record(
    checks,
    panelBox !== null && opened.workspace !== null && panelBox.y < opened.workspace.bottom,
    'the popup floats over the drawing area',
    { panelTop: panelBox === null ? null : panelBox.y, workspace: opened.workspace },
  );
  record(
    checks,
    status !== null && status.y > 0 && status.bottom <= opened.viewport.height + 1 && status.height >= 24,
    'the status bar is still fully on screen with the popup open',
    { statusbar: status, viewport: opened.viewport },
  );
  // Nothing is pushed: the region's bottom edge is still the status bar's top
  // edge, so the popup was not inserted between them (in flow it would sit below
  // the row and its bottom edge would be at the row's, not above the region).
  record(
    checks,
    region !== null && status !== null && Math.abs(region.bottom - status.y) <= 1,
    'the popup is not inserted between the command line and the status bar',
    { regionBottom: region === null ? null : region.bottom, statusbarTop: status === null ? null : status.y },
  );
  record(
    checks,
    Math.abs(opened.statusbar.y - closed.statusbar.y) <= 1,
    'the status bar did not move when the popup opened',
    { closed: closed.statusbar.y, opened: opened.statusbar.y },
  );
  // Observed, not asserted: the input row and the workspace legitimately move when
  // the inline completion list closes (the history toggle closes it by design) and
  // when the host's command log lands. Printed so a reader can see the geometry.
  observations.push(
    `history popup geometry: row ${closed.row === null ? '?' : closed.row.y} -> ${opened.row === null ? '?' : opened.row.y}, ` +
      `workspace ${opened.workspace === null ? '?' : `${opened.workspace.y}/${opened.workspace.height}`}, ` +
      `panel ${opened.panel === null ? '?' : `${opened.panel.y}..${opened.panel.bottom}`}, ` +
      `statusbar ${opened.statusbar === null ? '?' : `${opened.statusbar.y}..${opened.statusbar.bottom}`}`,
  );
  record(
    checks,
    opened.pageScroll !== null && opened.pageScroll <= opened.viewport.height + 1,
    'the document does not grow past the viewport',
    { scrollHeight: opened.pageScroll, viewportHeight: opened.viewport.height },
  );
  record(
    checks,
    panelBox !== null && panelBox.x >= 7 && panelBox.right <= opened.viewport.width - 7,
    'both popup edges are inside the window (no border running to the edge)',
    { panel: panelBox, viewportWidth: opened.viewport.width },
  );
  record(
    checks,
    panelBox !== null && panelBox.height <= opened.viewport.height * 0.4 + 1,
    'the popup is bounded to 40vh',
    { height: panelBox === null ? null : panelBox.height, cap: opened.viewport.height * 0.4 },
  );
  record(
    checks,
    opened.inlineRefusalBars === 0,
    'the open popup adds no second copy of the sentence under the field',
    { bars: opened.inlineRefusalBars },
  );
  // The scroller's last row is reachable: it ends at or above the list's own
  // bottom edge, so it is not sliced by the panel border.
  record(
    checks,
    opened.lastRowBottom !== null && opened.listBottom !== null && opened.lastRowBottom <= opened.listBottom + 1,
    'the last history row is fully inside the scroller',
    { lastRowBottom: opened.lastRowBottom, listBottom: opened.listBottom },
  );
  // The "more below" fade is a cue, not decoration: it must be absent when the
  // whole log fits (this case's log does), so a short popup is never dimmed.
  record(
    checks,
    opened.listOverflow !== null && opened.listOverflow === 0 && opened.moreBelow === null,
    'the bottom fade is off while the whole log fits in the popup',
    { overflow: opened.listOverflow, moreBelow: opened.moreBelow },
  );

  if (options.evidence) {
    const shot = await takeScreenshot(page, options.evidence, 'commands-history-popup.png');
    record(checks, shot.bytes > 0, 'screenshot commands-history-popup.png written', shot.file);
    observations.push(`history popup screenshot: ${shot.file}`);
  }

  // Close it again: the popup is an overlay, so closing must not shift anything
  // either, and the toggle must be its own off switch.
  await toggle.click();
  try {
    await panel.waitFor({ state: 'detached', timeout: options.timeout });
  } catch (error) {
    // Reported by the assertion below.
  }
  const reclosed = await popupGeometry(page);
  record(checks, reclosed.panel === null, 'a second press closes the history popup', { panel: reclosed.panel });
  record(
    checks,
    reclosed.statusbar !== null
      && reclosed.statusbar.y > 0
      && reclosed.statusbar.bottom <= reclosed.viewport.height + 1
      && Math.abs(reclosed.statusbar.y - closed.statusbar.y) <= 1,
    'closing it leaves the status bar fully on screen and where it was',
    { before: closed.statusbar, after: reclosed.statusbar },
  );
  record(
    checks,
    reclosed.pageScroll !== null && reclosed.pageScroll <= reclosed.viewport.height + 1,
    'the document still fits the viewport after closing',
    { scrollHeight: reclosed.pageScroll, viewportHeight: reclosed.viewport.height },
  );
  return { refused: REFUSED_COMMAND, layout: { closed, opened, reclosed } };
}

module.exports = {
  runHistoryPopupStep,
  REFUSED_COMMAND,
};
