// Contract tests for the command line's history log.
//
// The bug this suite pins: the collapse button next to the field read as a
// toggle for the *error line* (the chevron pointed at the red sentence under the
// row), the history held admitted commands only, and a refusal was therefore
// never in the record. The log is now one ordered list of both kinds, rendered by
// the desktop panel and the mobile sheet from the same state.
//
// Two layers are asserted here, like the rest of the suite:
//
//   * the **pure helpers** (`src/commands/command-history.ts`, plus the shared
//     message builder in `src/commands/command-message.ts`): order, bound,
//     dedupe, the command-only recall list, and the fact that the appended error
//     text is byte-for-byte the sentence the message line shows;
//   * **source contracts** on the two presentations, `App.tsx`, the controller
//     and the catalogs — the toggle is named "history", the error rows carry the
//     red class, the message line lives outside the panel, and clearing clears
//     the whole log.
//
// Nothing here mounts React: these are plain-Node assertions, not a DOM test.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readModule } from './helpers.ts';
import {
  COMMAND_HISTORY_LIMIT,
  appendCommand,
  appendError,
  appendHistory,
  commandOnly,
  historyRows,
} from '../src/commands/command-history.ts';
import { commandMessageFor } from '../src/commands/command-message.ts';
import { createTranslator } from '../src/i18n.ts';
import { setCommandCatalog } from '../src/commands/commands.ts';
import { HOST_NAMES } from './helpers.ts';
import { mobileHistoryRows } from '../src/commands/command-line-model.ts';

setCommandCatalog(HOST_NAMES);

const en = await readCatalog('en');
const zh = await readCatalog('zh-CN');
const t = createTranslator({ en, 'zh-CN': zh }, 'en');

test('the log appends in order and separates commands from refusals', () => {
  let log = [];
  log = appendCommand(log, 'LINE');
  log = appendError(log, 'No drawing is open.');
  log = appendCommand(log, 'FIT');
  assert.deepEqual(log, [
    { kind: 'command', text: 'LINE' },
    { kind: 'error', text: 'No drawing is open.' },
    { kind: 'command', text: 'FIT' },
  ]);
  // Oldest first in the array, newest first in the rendered rows: both
  // presentations read the same order out of `historyRows`.
  assert.deepEqual(historyRows(log).map(entry => entry.text), ['FIT', 'No drawing is open.', 'LINE']);
  // Trimmed, and an empty append is a no-op that keeps the same reference.
  assert.deepEqual(appendCommand([], '  LINE  '), [{ kind: 'command', text: 'LINE' }]);
  const untouched = appendCommand([], 'LINE');
  assert.equal(appendError(untouched, '   '), untouched);
  assert.equal(appendHistory(untouched, { kind: 'command', text: '' }), untouched);
});

test('the log is bounded, dropping the oldest entries first', () => {
  assert.equal(COMMAND_HISTORY_LIMIT, 64, 'the bound the string history always had');
  let log = [];
  for (let index = 0; index < COMMAND_HISTORY_LIMIT + 5; index += 1) log = appendCommand(log, `CMD${index}`);
  assert.equal(log.length, COMMAND_HISTORY_LIMIT);
  assert.equal(log[0].text, 'CMD5', 'the first five were dropped');
  assert.equal(log.at(-1).text, `CMD${COMMAND_HISTORY_LIMIT + 4}`);
  // An explicit smaller bound (the old `pushHistory(…, 2)` shape) still works.
  assert.deepEqual(appendCommand(appendCommand(appendCommand([], 'A'), 'B'), 'C', 2).map(e => e.text), ['B', 'C']);
  // A zero bound is explicit, not a silent "unbounded".
  assert.deepEqual(appendCommand([{ kind: 'command', text: 'A' }], 'B', 0), []);
});

test('adjacent duplicates are dropped per kind, so a repeated command and a repeated refusal are one row', () => {
  let log = appendCommand(appendCommand([], 'LINE'), 'LINE');
  assert.deepEqual(commandOnly(log), ['LINE'], 'a repeated command is one row');
  log = appendError(appendError([], 'Busy.'), 'Busy.');
  assert.deepEqual(log, [{ kind: 'error', text: 'Busy.' }], 'a repeated refusal is one row');
  // Same words, different kind: two rows, because they are two facts.
  log = appendError(appendCommand([], 'LINE'), 'LINE');
  assert.deepEqual(log, [{ kind: 'command', text: 'LINE' }, { kind: 'error', text: 'LINE' }]);
  // Non-adjacent repeats are history, not duplicates.
  log = appendCommand(appendError(appendCommand([], 'LINE'), 'Busy.'), 'LINE');
  assert.deepEqual(commandOnly(log), ['LINE', 'LINE']);
});

test('the recall list is commands only: a refusal can never reach the field', () => {
  const log = [
    { kind: 'command', text: 'ZOOM 2' },
    { kind: 'error', text: 'Command ZOOM 2 is not supported yet.' },
    { kind: 'command', text: 'FIT' },
  ];
  assert.deepEqual(commandOnly(log), ['ZOOM 2', 'FIT']);
  assert.equal(log[1].kind, 'error', 'the refusal stays in the record, out of the recall list');
  // The mobile sheet is the same log, capped: errors stay in the record there
  // (that is the point of the fix) while the recall list stays command-only.
  const rows = mobileHistoryRows(log, 2);
  assert.deepEqual(rows.map(entry => entry.kind), ['command', 'error']);
  assert.deepEqual(commandOnly(rows), ['FIT']);
});

test('the appended error text is the sentence the message line shows', () => {
  // The one builder both readers use: `App` puts its text in `commandMessage`
  // (the live line) and hands the very same string to the log.
  const unsupported = commandMessageFor({ kind: 'unsupported', command: 'NOPE', reasonKey: 'command.unsupported' }, t);
  assert.equal(unsupported.kind, 'error');
  assert.equal(unsupported.text, en['command.unsupported'].replace('{command}', 'NOPE'));
  let log = appendCommand([], 'NOPE');
  log = appendError(log, unsupported.text);
  assert.equal(historyRows(log)[0].text, unsupported.text, 'the log records exactly what the line said');

  const reason = commandMessageFor({ kind: 'failed', command: 'FIT', error: 'CAD_COMMAND_UNAVAILABLE', reasonKey: 'plot.no_document' }, t);
  assert.equal(reason.text, en['plot.no_document']);
  // A host-side failure without a reason key is worded by the catalog, never by
  // the machine error alone.
  const failed = commandMessageFor({ kind: 'failed', error: 'CAD_COMMAND_NOT_ACCEPTED' }, t);
  assert.equal(failed.text, en['command.failed'].replace('{error}', 'CAD_COMMAND_NOT_ACCEPTED'));
  // An accepted command produces no line at all — and therefore no log entry.
  assert.equal(commandMessageFor({ kind: 'accepted', command: 'FIT' }, t), null);
  assert.equal(commandMessageFor({ kind: 'local', command: 'LAYERS' }, t), null);
  assert.equal(commandMessageFor(null, t), null);
});

test('both presentations render the one log and mark refusals as red rows', async () => {
  const desktop = await readModule('components/CommandLine.tsx');
  const mobile = await readModule('components/MobileCommandBar.tsx');
  for (const [name, source] of [['CommandLine.tsx', desktop], ['MobileCommandBar.tsx', mobile]]) {
    // The typed record, not the command-only recall list.
    assert.ok(source.includes('shell.historyEntries'), `${name} renders the typed log`);
    assert.ok(source.includes('is-error'), `${name} marks a refusal row`);
    assert.equal(source.includes('shell.history as string[]'), false, `${name} must not rebuild a string history`);
    // The toggle is the history toggle, by name.
    assert.ok(source.includes("t('command.toggle_history')"), `${name} names the toggle "history"`);
    assert.ok(source.includes('pressed={shell.historyOpen}'), `${name} keeps the pressed state`);
    assert.ok(source.includes('setHistoryOpen(!shell.historyOpen)'), `${name} toggles only the panel`);
    // An error row is never a button: it is plain text carrying the red class.
    assert.match(source, /<(p|div)[^>]*className="[^"]*is-error[^"]*"/, `${name} renders refusals as text`);
  }
  // Desktop: newest first from the shared rows helper, clear clears the log.
  assert.ok(desktop.includes('historyRows(shell.historyEntries)'));
  assert.ok(desktop.includes('disabled={shell.historyEntries.length === 0}'));
  // Mobile: the capped sheet of the same log.
  assert.ok(mobile.includes('mobileHistoryRows(shell.historyEntries)'));
});

test('the message line stays outside the history panel', async () => {
  const desktop = await readModule('components/CommandLine.tsx');
  const panel = desktop.slice(desktop.indexOf('className="history-panel"'));
  const panelEnd = panel.indexOf(') : null}');
  const panelMarkup = panel.slice(0, panelEnd);
  // The live sentences are rendered *before* the panel, as their own elements:
  // opening or closing the log can never hide, move or swallow them.
  assert.equal(panelMarkup.includes('commandline__message'), false, 'no message line inside the panel');
  const messages = desktop.indexOf('className="commandline__message error"');
  const panelStart = desktop.indexOf('className="history-panel"');
  assert.ok(messages > 0 && messages < panelStart, 'the message line sits between the row and the panel');
  // The mobile bar keeps its own three lines outside the sheet for the same reason.
  const mobile = await readModule('components/MobileCommandBar.tsx');
  const sheet = mobile.slice(mobile.indexOf('className="mobile-sheet"'), mobile.indexOf('className="mobile-commandbar__row"'));
  assert.equal(sheet.includes('mobile-commandbar__message'), false, 'no message line inside the sheet');
  assert.ok(mobile.indexOf('className="mobile-commandbar__message') > mobile.indexOf('className="mobile-commandbar__row"'));
});

test('the shell keeps one log: commands for recall, entries for the panels', async () => {
  const context = await readModule('context.ts');
  assert.ok(context.includes('readonly history: string[]'), 'the command-only recall list stays');
  assert.ok(context.includes('readonly historyEntries: readonly CommandHistoryEntry[]'), 'the typed log is exposed');
  assert.ok(context.includes('readonly recordHistoryError: (text: string) => void'), 'the refusal writer is exposed');
  assert.ok(context.includes("import type { CommandHistoryEntry } from './commands/command-history.ts'"));

  const app = await readModule('App.tsx');
  assert.ok(app.includes('useCommandHistory()'), 'App owns no history rule of its own');
  assert.ok(app.includes('commandMessageFor(commandResult, t)'), 'the line and the log share one builder');
  assert.ok(app.includes('commandHistory.appendCommand(result.command)'), 'an admitted line is stored');
  assert.ok(
    app.includes('recordHistoryError(commandMessage.text)'),
    'the refusal is stored with the text the line shows',
  );
  assert.ok(app.includes('history: commandHistory.commands as string[]'), 'recall reads the derived list');
  assert.ok(app.includes('historyEntries: commandHistory.entries'), 'the panels read the log');
  assert.ok(app.includes('recordHistoryError: commandHistory.recordError'), 'the hook writes through the shell');
  assert.ok(app.includes('clearHistory: commandHistory.clear'), 'clear clears the whole log');

  // The text step's refusal takes the same one-string path.
  const hook = await readModule('commands/use-command-line.ts');
  assert.ok(hook.includes('shell.recordHistoryError(text)'), 'a text-step refusal reaches the log');
  assert.ok(hook.includes('setTextMessage({ kind: \'error\', text })'), 'and the live line keeps the same text');
  assert.equal(
    hook.includes('setTextMessage({ kind: \'error\', text: result.error })'),
    false,
    'no second wording of the same refusal',
  );
});

test('the toggle is labelled as history in both catalogs', () => {
  for (const [locale, catalog] of [['en', en], ['zh-CN', zh]]) {
    const label = catalog['command.toggle_history'];
    assert.equal(typeof label, 'string', `${locale} has command.toggle_history`);
    assert.ok(label.trim().length > 0, `${locale} label is not empty`);
    // The accessible name has to say what the button toggles.
    assert.match(
      label.toLowerCase(),
      locale === 'zh-CN' ? /历史/ : /history/,
      `${locale} names the panel it toggles`,
    );
    assert.ok(/history|历史/.test(catalog['command.history']));
  }
  // The two catalogs' key sets stay identical (check-i18n.py enforces the same
  // thing for CI; this is the in-suite counterpart).
  assert.deepEqual(Object.keys(en).sort(), Object.keys(zh).sort());
});
