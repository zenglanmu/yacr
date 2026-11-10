// Contract tests for the global error report store.
//
// The store is intentionally DOM-free and React-free, so these cases import it
// directly and pin dedupe, the bounded history, newest selection, navigation,
// dismiss, the kind→catalog-key mapping and the clipboard text. No jsdom.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCss, readModule } from './helpers.ts';
import {
  ERROR_REPORT_LIMIT,
  dismissErrorReport,
  errorKindLabelKey,
  formatErrorReport,
  getErrorReportState,
  getReports,
  getVisibleReport,
  isErrorReportVisible,
  reportError,
  resetErrorReportsForTests,
  showNewerErrorReport,
  showOlderErrorReport,
  subscribe,
} from '../src/errors/error-report.ts';

/** A fake translator that makes the catalog key visible in the output. */
function keyTranslator(key) {
  return `«${key}»`;
}

test('identical consecutive reports are not stacked', () => {
  resetErrorReportsForTests();
  const first = reportError({ kind: 'uncaught', message: 'boom', location: 'app.ts:1:2', stack: 'at one' });
  const second = reportError({ kind: 'uncaught', message: 'boom', location: 'app.ts:1:2', stack: 'at one' });
  assert.equal(second.id, first.id, 'the duplicate answers with the stored report');
  assert.equal(getReports().length, 1);

  // Any one of the four identity fields makes it a new report.
  reportError({ kind: 'rejection', message: 'boom', location: 'app.ts:1:2', stack: 'at one' });
  reportError({ kind: 'uncaught', message: 'boom', location: 'app.ts:1:3', stack: 'at one' });
  reportError({ kind: 'uncaught', message: 'boom', location: 'app.ts:1:2', stack: 'at two' });
  assert.equal(getReports().length, 4);
});

test('a duplicate does not notify subscribers', () => {
  resetErrorReportsForTests();
  let calls = 0;
  const off = subscribe(() => { calls += 1; });
  reportError({ kind: 'panic', message: 'panicked' });
  assert.equal(calls, 1, 'a new report notifies');
  reportError({ kind: 'panic', message: 'panicked' });
  assert.equal(calls, 1, 'the duplicate is silent');
  off();
});

test('history is bounded to the newest 20 reports', () => {
  resetErrorReportsForTests();
  for (let index = 0; index < 25; index += 1) {
    reportError({ kind: 'host', message: `error ${index}` });
  }
  const reports = getReports();
  assert.equal(reports.length, ERROR_REPORT_LIMIT);
  assert.equal(reports[0].message, 'error 5', 'the oldest five fell off');
  assert.equal(reports[reports.length - 1].message, 'error 24');
});

test('the dialog text is real selectable text and the card is sized, not auto', async () => {
  const css = await readCss();
  // The whole point of the dialog: a user can select and copy a Rust diagnostic.
  const selectableAt = css.indexOf('.error-report__message,');
  assert.notEqual(selectableAt, -1);
  const block = css.slice(selectableAt, css.indexOf('}', selectableAt));
  assert.ok(block.includes('user-select: text'), 'message/location/stack are selectable');
  assert.ok(block.includes('-webkit-user-select: text'), 'WebKit needs the prefixed property too');
  assert.equal(block.includes('user-select: none'), false);
  // A readable card on desktop; the mobile bottom sheet overrides it in mobile.css.
  const cardAt = css.indexOf('.error-report {');
  assert.notEqual(cardAt, -1);
  const card = css.slice(cardAt, css.indexOf('}', cardAt));
  assert.ok(card.includes('width: min(760px, calc(100vw - 32px))'), 'desktop width is bounded by the viewport');
  assert.ok(card.includes('min(72dvh, 620px)'), 'desktop height leaves the drawing visible');
  assert.ok(css.includes('.popup-overlay.error-report-overlay { z-index: 70; }'), 'above popup (60) and notices (62)');
});

test('capture is installed before render and the dialog lives in its own root', async () => {
  const main = await readModule('main.tsx');
  const installAt = main.indexOf('installGlobalErrorCapture()');
  const renderAt = main.indexOf('.render(');
  assert.ok(installAt !== -1 && renderAt !== -1 && installAt < renderAt,
    'capture must be active before anything renders');
  assert.ok(main.includes('mountErrorReportHost()'), 'the dialog is mounted up front');
  assert.ok(main.includes('onUncaughtError'), 'an uncaught render failure reaches the store');
  assert.equal(main.includes('onCaughtError: (error, info) => reportRenderError'),
    false, 'a caught error keeps its local fallback instead of stacking a modal');

  const mount = await readModule('errors/mount-error-report-host.tsx');
  assert.ok(mount.includes('createRoot(container)'), 'the host owns a separate React root');
  assert.ok(mount.includes('document.body.appendChild(container)'), 'the root sits outside #root');
});

test('the shell can ask whether the dialog owns the keyboard', () => {
  // `App.tsx` gates its global shortcuts on this live value, because the error
  // dialog is portalled from its own React root and is not part of `popupOpen`.
  resetErrorReportsForTests();
  assert.equal(isErrorReportVisible(), false, 'an empty store shows no dialog');
  reportError({ kind: 'host', message: 'import.rvt.no_geometry: …' });
  assert.equal(isErrorReportVisible(), true, 'a report owns Escape and the accelerators');
  dismissErrorReport();
  assert.equal(isErrorReportVisible(), false, 'dismissing hands the keyboard back');
});

test('newest is visible by default, and older/later walk the history', () => {
  resetErrorReportsForTests();
  reportError({ kind: 'uncaught', message: 'a' });
  reportError({ kind: 'rejection', message: 'b' });
  reportError({ kind: 'render', message: 'c' });
  assert.equal(getVisibleReport()?.message, 'c');
  assert.equal(getErrorReportState().visibleIndex, 2);

  showOlderErrorReport();
  assert.equal(getVisibleReport()?.message, 'b');
  showOlderErrorReport();
  assert.equal(getVisibleReport()?.message, 'a');
  showOlderErrorReport();
  assert.equal(getVisibleReport()?.message, 'a', 'the oldest end does not wrap');

  showNewerErrorReport();
  assert.equal(getVisibleReport()?.message, 'b');
  showNewerErrorReport();
  assert.equal(getVisibleReport()?.message, 'c');
  showNewerErrorReport();
  assert.equal(getVisibleReport()?.message, 'c', 'the newest end does not wrap');
});

test('dismissing removes the visible report and reveals the next one', () => {
  resetErrorReportsForTests();
  reportError({ kind: 'uncaught', message: 'a' });
  reportError({ kind: 'rejection', message: 'b' });
  reportError({ kind: 'render', message: 'c' });

  dismissErrorReport();
  assert.equal(getReports().length, 2);
  assert.equal(getVisibleReport()?.message, 'b', 'dismissing the newest reveals the previous report');

  showOlderErrorReport();
  assert.equal(getVisibleReport()?.message, 'a');
  dismissErrorReport();
  assert.equal(getVisibleReport()?.message, 'b', 'dismissing an older report reveals the newer one');

  dismissErrorReport();
  assert.equal(getReports().length, 0);
  assert.equal(getVisibleReport(), null);
  dismissErrorReport();
  assert.equal(getVisibleReport(), null, 'dismiss on an empty history is a no-op');
});

test('a dismissed failure can be reported again', () => {
  resetErrorReportsForTests();
  reportError({ kind: 'host', message: 'import.rvt.no_geometry' });
  dismissErrorReport();
  assert.equal(getReports().length, 0);
  reportError({ kind: 'host', message: 'import.rvt.no_geometry' });
  assert.equal(getReports().length, 1, 'dismissal removes the entry, so capture is not muted');
});

test('every kind maps to its catalog key', () => {
  assert.equal(errorKindLabelKey('uncaught'), 'error.kind.uncaught');
  assert.equal(errorKindLabelKey('rejection'), 'error.kind.rejection');
  assert.equal(errorKindLabelKey('render'), 'error.kind.render');
  assert.equal(errorKindLabelKey('panic'), 'error.kind.panic');
  assert.equal(errorKindLabelKey('host'), 'error.kind.host');
});

test('the clipboard block carries kind, time, message, location and stack', () => {
  resetErrorReportsForTests();
  const report = reportError({
    kind: 'render',
    message: 'Cannot read properties of undefined',
    location: 'App.tsx:42:7',
    stack: 'Error: boom\n    at App.tsx:42:7',
  });
  assert.match(report.at, /^\d{4}-\d{2}-\d{2}T/, 'at is an ISO timestamp');
  assert.equal(report.at, new Date(report.at).toISOString());

  const text = formatErrorReport(report, keyTranslator);
  assert.ok(text.includes('«error.kind.render»'), 'kind label comes from the catalog');
  assert.ok(text.includes('«error.label.time»'), 'time label comes from the catalog');
  assert.ok(text.includes('«error.label.message»'), 'message label comes from the catalog');
  assert.ok(text.includes('«error.label.location»'), 'location label comes from the catalog');
  assert.ok(text.includes('«error.label.stack»'), 'stack label comes from the catalog');
  for (const part of [report.at, report.message, report.location, report.stack]) {
    assert.ok(text.includes(part), `the copied block contains ${part}`);
  }
});

test('a host report keeps the raw sentence in detail but does not duplicate it', () => {
  resetErrorReportsForTests();
  const report = reportError({ kind: 'host', message: 'import.rvt.no_geometry: empty scene' });
  assert.equal(report.detail, 'import.rvt.no_geometry: empty scene', 'detail carries the raw host string');
  const text = formatErrorReport(report, keyTranslator);
  assert.equal(
    text.split('import.rvt.no_geometry: empty scene').length - 1,
    1,
    'the same string is not printed twice',
  );
});
