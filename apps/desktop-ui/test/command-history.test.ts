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
//     dedupe, the command-only recall list, that the appended error text is
//     byte-for-byte the sentence the builder produced, and that one submitted
//     refusal is exactly one row;
//   * **source contracts** on the two presentations, `App.tsx`, the controller,
//     the stylesheets and the catalogs — the toggle is named "history", the error
//     rows carry the red class, **no refusal line is rendered under either
//     field** (the history is the only surface a refusal has), and the desktop
//     panel is an absolute popup above the input row, not a row of the region.
//
// Nothing here mounts React: these are plain-Node assertions, not a DOM test.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCss, readCatalog, readModule } from './helpers.ts';
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

test('the appended error text is the sentence the message builder produced', () => {
  // The one builder the whole refusal path uses: `App` hands the memo's text to
  // the log, which is now the *only* place a user reads it (the red line under
  // the field is gone), so the record and the built sentence cannot drift.
  const unsupported = commandMessageFor({ kind: 'unsupported', command: 'NOPE', reasonKey: 'command.unsupported' }, t);
  assert.equal(unsupported.kind, 'error');
  assert.equal(unsupported.text, en['command.unsupported'].replace('{command}', 'NOPE'));
  let log = appendCommand([], 'NOPE');
  log = appendError(log, unsupported.text);
  assert.equal(historyRows(log)[0].text, unsupported.text, 'the log records exactly what the builder produced');

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

test('no refusal line is rendered under either command field', async () => {
  // The duplicate the screenshots showed: the same sentence in a red line under
  // the input **and** in the history. The history is the only surface now, so
  // neither presentation may render a message element at all.
  for (const name of ['components/CommandLine.tsx', 'components/MobileCommandBar.tsx']) {
    const source = await readModule(name);
    assert.equal(source.includes('commandline__message'), false, `${name} renders no docked refusal line`);
    assert.equal(source.includes('line.messageText'), false, `${name} reads no controller message`);
    assert.equal(source.includes('shell.commandMessage'), false, `${name} re-renders no command result`);
    // Nothing hides the sentence behind a condition either: the element is gone,
    // not merely styled out, so there is no `role="alert"` to find on refusal.
    assert.equal(source.includes('cad-command-message'), false, `${name} has no message element id`);
  }
  // The mobile bar keeps exactly one message element, and it is the **host
  // banner** (`shell.displayedError`): a failed canvas module or poll still needs
  // a surface on a phone, and it is not a command-line refusal.
  const mobile = await readModule('components/MobileCommandBar.tsx');
  assert.equal(mobile.split('mobile-commandbar__message').length - 1, 1, 'one message element, the host banner');
  assert.ok(mobile.includes('{shell.displayedError}'), 'the banner still renders the host sentence');
  assert.equal(mobile.includes('line.messageText'), false, 'no refusal line shares that slot');
  // The controller no longer publishes a message (and so the field cannot point
  // `aria-describedby` at an element that does not exist).
  const hook = await readModule('commands/use-command-line.ts');
  const field = await readModule('components/CommandField.tsx');
  assert.equal(hook.includes('messageText'), false, 'the controller publishes no message text');
  assert.equal(hook.includes('describedBy'), false, 'the controller publishes no described-by id');
  assert.equal(field.includes('aria-describedby={'), false, 'the field points at no removed element');
  // The mobile start hint is not suppressed by a refusal any more: it has one
  // reason of its own (no drawing, no host failure).
  const hint = mobile.slice(mobile.indexOf('const showHint'), mobile.indexOf(';', mobile.indexOf('const showHint')));
  assert.equal(hint.includes('messageText'), false, 'the hint does not read the gone message');
  assert.equal(hint.includes('commandMessage'), false, 'the hint does not read the gone message');
  assert.ok(hint.includes('shell.displayedError'), 'the host banner still wins over the hint');
});

test('the desktop history panel is an upward popup over the drawing, not a row of the region', async () => {
  const css = await readCss();
  const block = (selector) => {
    const start = css.indexOf(`${selector} {`);
    assert.notEqual(start, -1, `${selector} must exist`);
    return css.slice(start, css.indexOf('}', start));
  };
  // The region is the containing block; without it the popup would be positioned
  // against the viewport (or the nearest ancestor) and land nowhere useful.
  const region = block('.region--cmdline');
  assert.match(region, /position:\s*relative/, 'the docked region anchors the popup');
  assert.match(region, /z-index:\s*\d+/, 'the popup paints over the workspace');
  // The panel itself: absolute, above the row (`bottom: calc(100% + 6px)`), inset
  // by the region's own 8px so both borders stay visible.
  const panel = block('.history-panel');
  assert.match(panel, /position:\s*absolute/, 'the panel is an overlay, not in flow');
  assert.match(panel, /bottom:\s*calc\(100% \+ 6px\)/, 'it opens above the input row');
  assert.match(panel, /left:\s*8px/, 'the left border is inside the window');
  assert.match(panel, /right:\s*8px/, 'the right border is inside the window');
  assert.match(panel, /max-height:\s*40vh/, 'the popup is bounded');
  assert.match(panel, /border:\s*1px solid var\(--line-soft\)/, 'it keeps the chrome border');
  assert.match(panel, /box-shadow:/, 'a floating panel carries an edge cue');
  assert.match(panel, /box-shadow:[^;]*rgba\(0, 0, 0, /, 'the shared shadow step, not a new one');
  assert.match(panel, /overflow:\s*hidden/, 'the head stays pinned and the list scrolls');
  // The old in-flow shape is gone: a top margin is exactly what used to push the
  // status bar off-screen.
  assert.equal(panel.includes('margin: 5px 0 0 24px'), false, 'the panel no longer sits in the flow below the row');
  // Bounded, scrollable list with the affordance the mobile sheet's scroller
  // uses (thin `--line-soft` bar), and a bottom pad so the newest row is
  // reachable rather than sliced at the panel edge.
  const list = block('.history-panel__list');
  assert.match(list, /flex:\s*1 1 auto/, 'the list takes the popup\'s remaining height');
  assert.match(list, /min-height:\s*0/, 'a flex child may shrink below its content');
  assert.match(list, /overflow-y:\s*auto/, 'the list scrolls inside the popup');
  assert.match(list, /overscroll-behavior:\s*contain/);
  assert.match(list, /scrollbar-width:\s*thin/, 'the scroller is visible');
  assert.match(list, /scrollbar-color:\s*var\(--line-soft\) transparent/);
  assert.match(list, /padding: 3px 3px 5px/, 'the last row clears the edge');
  assert.equal(list.includes('max-height: 150px'), false, 'the cap moved to the panel (40vh)');
  // The visible "more below" cue is the mobile drawer's fade, not a new style, and
  // it is driven by a data attribute so a short log is never dimmed.
  assert.ok(css.includes('.history-panel::after'), 'the popup carries the drawer\'s bottom fade');
  const fade = block('.history-panel::after');
  assert.match(fade, /pointer-events:\s*none/, 'the fade never swallows a scroll');
  assert.match(fade, /linear-gradient\(to bottom, rgba\(43, 51, 63, 0\) 0%, var\(--chrome-3\) 100%\)/, 'the drawer pattern');
  assert.match(fade, /opacity:\s*0/, 'hidden until there is something below');
  assert.ok(css.includes(".history-panel[data-more-below='true']::after { opacity: 1; }"), 'the cue is data-driven');
  // The component sets that attribute from the scroller's own geometry.
  const desktop = await readModule('components/CommandLine.tsx');
  assert.ok(desktop.includes("data-more-below={moreBelow ? 'true' : undefined}"));
  assert.ok(desktop.includes('node.scrollHeight - node.scrollTop - node.clientHeight > 2'));
});

test('the shell keeps one log: commands for recall, entries for the panels', async () => {
  const context = await readModule('context.ts');
  assert.ok(context.includes('readonly history: string[]'), 'the command-only recall list stays');
  assert.ok(context.includes('readonly historyEntries: readonly CommandHistoryEntry[]'), 'the typed log is exposed');
  assert.ok(context.includes('readonly recordHistoryError: (text: string) => void'), 'the refusal writer is exposed');
  assert.ok(context.includes("import type { CommandHistoryEntry } from './commands/command-history.ts'"));
  // The dead surface is gone: there is no shell value for a rendered message.
  assert.equal(context.includes('commandMessage'), false, 'no message line state is exposed to the views');

  const app = await readModule('App.tsx');
  assert.ok(app.includes('useCommandHistory()'), 'App owns no history rule of its own');
  assert.ok(app.includes('commandMessageFor(commandResult, t)'), 'the log reads the one builder');
  assert.ok(app.includes('commandHistory.appendCommand(result.command)'), 'an admitted line is stored');
  assert.ok(
    app.includes('recordHistoryError(commandMessage.text)'),
    'the refusal is stored with the sentence the builder produced',
  );
  assert.ok(app.includes('history: commandHistory.commands as string[]'), 'recall reads the derived list');
  assert.ok(app.includes('historyEntries: commandHistory.entries'), 'the panels read the log');
  assert.ok(app.includes('recordHistoryError: commandHistory.recordError'), 'the hook writes through the shell');
  assert.ok(app.includes('clearHistory: commandHistory.clear'), 'clear clears the whole log');

  // The text step's refusal takes the same one-row path, and only that path: the
  // removed `setTextMessage` state would have been the second surface.
  const hook = await readModule('commands/use-command-line.ts');
  assert.ok(
    hook.includes('const recordHistoryError = shell.recordHistoryError;'),
    'the controller holds the one writer as a stable alias',
  );
  assert.ok(hook.includes('recordHistoryError(text);'), 'a text-step refusal reaches the log');
  assert.equal(hook.includes('setTextMessage'), false, 'a text-step refusal is not stored twice');
  assert.equal(
    hook.includes('setTextMessage({ kind: \'error\', text: result.error })'),
    false,
    'no second wording of the same refusal',
  );
});

test('one submitted refusal is exactly one history row', async () => {
  // Pure layer: the same sentence appended twice by the same submit is one row
  // (an adjacent same-kind repeat is dropped), so a re-run of the recording
  // effect — a re-render, a locale change, React's double-invoked effects — can
  // never show the duplicate the screenshot had.
  const sentence = commandMessageFor({ kind: 'unsupported', command: 'DSF.FDSAF' }, t).text;
  let log = appendCommand([], 'COPYCLIP');
  log = appendError(log, sentence);
  const once = log;
  log = appendError(log, sentence);
  assert.equal(log, once, 'the second write is a no-op on the same reference');
  assert.equal(historyRows(log).filter(entry => entry.kind === 'error').length, 1, 'one refusal, one red row');

  // Source layer: exactly one writer per submit path. `App` records the memo's
  // text in one effect, and the controller records a text-step refusal in one
  // function; neither has a second call site that could append the same
  // sentence again.
  const app = await readModule('App.tsx');
  assert.equal(app.split('recordHistoryError(commandMessage.text)').length - 1, 1, 'one recording site in App');
  const hook = await readModule('commands/use-command-line.ts');
  const refuseBody = hook.slice(hook.indexOf('function refuse('), hook.indexOf('function submitText('));
  assert.equal(refuseBody.split('recordHistoryError(').length - 1, 1, 'a refusal is written exactly once');
  assert.ok(refuseBody.includes('recordHistoryError(text);'));
  // The refusal is built once per result and recorded once: the effect reads the
  // memo, not the raw result.
  assert.ok(app.includes('if (commandMessage) recordHistoryError(commandMessage.text);'));
  assert.equal(app.includes('appendError('), false, 'App never appends an error behind the builder');
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
