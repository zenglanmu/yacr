// Contract tests for copying the command history.
//
// The feature is one record read a third way: the desktop panel copies the merged
// rows it renders (command + attached host echo), the mobile sheet copies its
// typed rows in the order it shows them, and both report the same localized
// "Copied" / "Copy failed" status. The suites pin:
//
//   * the exact plain text of a desktop row — the prompt, the two-space output
//     indent, the `label = value unit meta` joins and refusal rows;
//   * the two orders (desktop oldest-first, mobile newest-first) so a copy cannot
//     silently reverse the log;
//   * the clipboard's fallback ladder with injected fakes (Clipboard API, then the
//     textarea + `execCommand` path, then an honest `false`);
//   * the two components actually wiring the shared controls, and both catalogs
//     carrying the same keys.
//
// Plain Node, no React mount and no DOM: the components are read as source, like
// the rest of `test/`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readModule } from './helpers.ts';
import { HOST_NAMES } from './helpers.ts';
import { createTranslator } from '../src/i18n.ts';
import { setCommandCatalog } from '../src/commands/commands.ts';
import { commandWindowRows } from '../src/commands/command-window.ts';
import {
  HISTORY_OUTPUT_INDENT,
  commandHistoryText,
  commandLogLineText,
  commandWindowText,
  entryText,
  isHistoryCopyChord,
} from '../src/commands/history-copy.ts';
import { mobileHistoryRows } from '../src/commands/command-line-model.ts';
import { writeClipboardText } from '../src/clipboard.ts';

setCommandCatalog(HOST_NAMES);

const en = await readCatalog('en');
const zh = await readCatalog('zh-CN');
const t = createTranslator({ en, 'zh-CN': zh }, 'en');
const prompt = t('command.log.prompt');
const DEGREE = '\u00B0';

/** A fake `<textarea>` host: records the legacy copy path and cleans up like one. */
function fakeDocument(copyResult) {
  const calls = { exec: 0, appended: 0, removed: 0 };
  const body = {
    children: [],
    appendChild(node) {
      calls.appended += 1;
      this.children.push(node);
      node.parentNode = this;
      return node;
    },
    removeChild(node) {
      calls.removed += 1;
      const at = this.children.indexOf(node);
      if (at >= 0) this.children.splice(at, 1);
      node.parentNode = null;
      return node;
    },
  };
  const doc = {
    body,
    activeElement: null,
    getSelection: () => null,
    createElement: () => ({
      value: '',
      style: {},
      setAttribute() {},
      focus() {},
      select() {},
      setSelectionRange() {},
    }),
    execCommand(command) {
      calls.exec += 1;
      assert.equal(command, 'copy', 'only the copy command is issued');
      return copyResult;
    },
  };
  return { doc, calls, body };
}

test('a desktop row copies its prompt and its attached output, indented two spaces', () => {
  const rows = commandWindowRows(
    [{ kind: 'command', text: 'DIST' }],
    [
      { sequence: 1, kind: 'command', command: 'DIST' },
      {
        sequence: 2,
        kind: 'result',
        measurement: { algorithm: 'distance', value: 12.3456, unit: 'mm', points: [[0, 0, 0], [10, 0, 0]] },
      },
    ],
    t,
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].lines.length > 0, true, 'the echo is attached');
  const text = entryText(rows[0], t);
  assert.equal(text, [
    `${prompt} DIST`,
    `${HISTORY_OUTPUT_INDENT}${t('command.log.distance')} = 12.3456 mm`,
    `${HISTORY_OUTPUT_INDENT}${t('command.log.angle')} = 0.0000${DEGREE}`,
    `${HISTORY_OUTPUT_INDENT}${t('command.log.delta_x')} = 10.0000 mm`,
    `${HISTORY_OUTPUT_INDENT}${t('command.log.delta_y')} = 0.0000 mm`,
    `${HISTORY_OUTPUT_INDENT}${t('command.log.delta_z')} = 0.0000 mm`,
  ].join('\n'));
  assert.equal(commandWindowText(rows, t), text, 'one row copies as one block');
});

test('the output joins match the panel: "label = value unit meta", command tone keeps a bare space', () => {
  assert.equal(
    commandLogLineText({ tone: 'value', label: t('command.log.distance'), text: '1.5000', unit: 'mm', meta: null }),
    `${t('command.log.distance')} = 1.5000 mm`,
  );
  // The precision chip is a separate span with a margin in the panel; the text
  // keeps one space so `mm` and `Approximate` cannot paste as `mmApproximate`.
  assert.equal(
    commandLogLineText({
      tone: 'value',
      label: t('command.log.distance'),
      text: '1.5000',
      unit: 'mm',
      meta: t('command.log.precision.approximate'),
    }),
    `${t('command.log.distance')} = 1.5000 mm ${t('command.log.precision.approximate')}`,
  );
  assert.equal(
    commandLogLineText({ tone: 'command', label: t('command.log.prompt'), text: 'ZOOM', unit: null, meta: null }),
    `${t('command.log.prompt')} ZOOM`,
    'a command-tone label has no " = "',
  );
  assert.equal(
    commandLogLineText({ tone: 'error', label: null, text: 'Refused by the host', unit: null, meta: null }),
    'Refused by the host',
    'an unlabelled line is its own text',
  );
});

test('a refusal row copies its sentence verbatim, with no prompt and no output', () => {
  const row = { key: '1:error:sentence', kind: 'error', text: 'No drawing is open.', recall: null, code: null, lines: [] };
  assert.equal(entryText(row, t), 'No drawing is open.');
  // The same sentence read as a one-row record.
  assert.equal(commandWindowText([row], t), 'No drawing is open.');
});

test('the desktop copy is oldest-first, matching the panel top to bottom', () => {
  const rows = [
    { key: '0:command:LINE', kind: 'command', text: 'LINE', recall: 'LINE', code: null, lines: [] },
    { key: '1:error:Busy.', kind: 'error', text: 'Busy.', recall: null, code: null, lines: [] },
    { key: '2:command:FIT', kind: 'command', text: 'FIT', recall: 'FIT', code: null, lines: [] },
  ];
  assert.equal(
    commandWindowText(rows, t),
    [`${prompt} LINE`, 'Busy.', `${prompt} FIT`].join('\n'),
  );
});

test('the mobile copy is newest-first, matching the sheet order', () => {
  const entries = [
    { kind: 'command', text: 'LINE' },
    { kind: 'error', text: 'No drawing is open.' },
    { kind: 'command', text: 'FIT' },
  ];
  const rows = mobileHistoryRows(entries);
  assert.deepEqual(rows.map(entry => entry.text), ['FIT', 'No drawing is open.', 'LINE']);
  assert.equal(commandHistoryText(rows), ['FIT', 'No drawing is open.', 'LINE'].join('\n'));
  // One entry per line, and storage order is never assumed: the serializer keeps
  // whatever order the sheet handed it.
  assert.equal(commandHistoryText(entries), ['LINE', 'No drawing is open.', 'FIT'].join('\n'));
});

test('the copy chord is Ctrl+C / Cmd+C and nothing else', () => {
  assert.equal(isHistoryCopyChord({ key: 'c', ctrlKey: true }), true);
  assert.equal(isHistoryCopyChord({ key: 'C', ctrlKey: true }), true);
  assert.equal(isHistoryCopyChord({ key: 'c', metaKey: true }), true);
  // Shift belongs to COPYBASE, Alt/Meta chords to the desktop, and a bare letter
  // to the field.
  assert.equal(isHistoryCopyChord({ key: 'c', ctrlKey: true, shiftKey: true }), false);
  assert.equal(isHistoryCopyChord({ key: 'c', ctrlKey: true, altKey: true }), false);
  assert.equal(isHistoryCopyChord({ key: 'c' }), false);
  assert.equal(isHistoryCopyChord({ key: 'v', ctrlKey: true }), false);
  assert.equal(isHistoryCopyChord({ key: 'c', ctrlKey: true, isComposing: true }), false);
  assert.equal(isHistoryCopyChord({ key: 'c', ctrlKey: true, keyCode: 229 }), false);
});

test('the Clipboard API path wins when it is available and secure', async () => {
  const writes = [];
  const copied = await writeClipboardText('hello', {
    navigator: { clipboard: { writeText: async text => { writes.push(text); } } },
    document: null,
  });
  assert.equal(copied, true);
  assert.deepEqual(writes, ['hello']);
  assert.equal(await writeClipboardText(''), false, 'empty text is not claimed as copied');
});

test('a rejected Clipboard API falls back to the textarea + execCommand path', async () => {
  const { doc, calls, body } = fakeDocument(true);
  const copied = await writeClipboardText('selectable history', {
    navigator: { clipboard: { writeText: async () => { throw new Error('clipboard denied'); } } },
    document: doc,
  });
  assert.equal(copied, true);
  assert.equal(calls.exec, 1, 'the legacy path ran exactly once');
  assert.equal(calls.removed, 1, 'the temporary textarea was removed');
  assert.equal(body.children.length, 0, 'nothing was left in the document');
});

test('an absent Clipboard API uses the legacy path directly', async () => {
  const { doc, calls } = fakeDocument(true);
  assert.equal(await writeClipboardText('history', { navigator: { clipboard: null }, document: doc }), true);
  assert.equal(calls.exec, 1);
});

test('an insecure context skips the Clipboard API instead of provoking a rejection', async () => {
  let apiCalls = 0;
  const { doc } = fakeDocument(true);
  const copied = await writeClipboardText('history', {
    isSecureContext: false,
    navigator: { clipboard: { writeText: async () => { apiCalls += 1; } } },
    document: doc,
  });
  assert.equal(copied, true);
  assert.equal(apiCalls, 0, 'the async API was never called');
});

test('both paths failing answers false, so the UI can say "Copy failed"', async () => {
  const { doc, calls } = fakeDocument(false);
  const copied = await writeClipboardText('history', {
    navigator: { clipboard: { writeText: async () => { throw new Error('denied'); } } },
    document: doc,
  });
  assert.equal(copied, false);
  assert.equal(calls.exec, 1, 'the fallback was attempted before giving up');
  // No document at all is the same honest answer, not a silent success.
  assert.equal(await writeClipboardText('history', { navigator: null, document: null }), false);
});

test('both presentations wire the header control, the menu, the chord and the status', async () => {
  const desktop = await readModule('components/CommandLine.tsx');
  const mobile = await readModule('components/MobileCommandBar.tsx');
  for (const [name, source] of [['CommandLine.tsx', desktop], ['MobileCommandBar.tsx', mobile]]) {
    assert.ok(source.includes("t('command.history.copy')"), `${name} names the copy control`);
    assert.ok(source.includes('useHistoryCopy'), `${name} uses the shared copy state`);
    assert.ok(source.includes('isHistoryCopyChord'), `${name} owns the Ctrl+C rule`);
    assert.ok(source.includes('historySurfaceReading'), `${name} decides from the reading surface`);
    assert.ok(source.includes('role="status"'), `${name} reports the outcome visibly`);
    assert.ok(source.includes("'command.history.copy_entry'"), `${name} offers a per-row copy`);
    assert.ok(source.includes("'command.history.copy_all'"), `${name} offers copy-all`);
    assert.ok(source.includes('icon="copy"'), `${name} draws the registered copy icon`);
    assert.ok(source.includes('ContextMenu'), `${name} reuses the accessible overlay menu`);
  }
  assert.ok(desktop.includes('commandWindowText'), 'the desktop copies the merged panel rows');
  assert.ok(desktop.includes('data-history-key'), 'a pressed row is identified by its key');
  assert.ok(desktop.includes('history-panel__status'), 'the desktop has its status element');
  assert.ok(mobile.includes('commandHistoryText'), 'the mobile copies the sheet rows');
  assert.ok(mobile.includes('data-history-index'), 'a pressed sheet row is identified');
  assert.ok(mobile.includes('mobile-sheet__status'), 'the sheet has its status element');
  // The clipboard write is reached through one module and one hook, not by a
  // second clipboard call inside a component.
  assert.equal(desktop.includes('navigator.clipboard'), false, 'the panel owns no clipboard path');
  assert.equal(mobile.includes('navigator.clipboard'), false, 'the sheet owns no clipboard path');
  assert.ok((await readModule('clipboard.ts')).includes('writeText'));
  assert.ok((await readModule('commands/use-history-copy.ts')).includes('writeClipboardText'));
});

test('both catalogs carry the five copy strings', () => {
  const keys = [
    'command.history.copy',
    'command.history.copy_all',
    'command.history.copy_entry',
    'command.history.copied',
    'command.history.copy_failed',
  ];
  for (const [locale, catalog] of [['en', en], ['zh-CN', zh]]) {
    for (const key of keys) {
      assert.equal(typeof catalog[key], 'string', `${locale} has ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${locale} ${key} is not empty`);
    }
  }
  // Natural wording, not machine tokens: the status has to read as a sentence
  // answer and the failure must be unambiguous.
  assert.match(en['command.history.copied'], /copied/i);
  assert.match(en['command.history.copy_failed'], /fail/i);
  assert.match(zh['command.history.copied'], /已复制/);
  assert.match(zh['command.history.copy_failed'], /失败/);
  assert.deepEqual(Object.keys(en).sort(), Object.keys(zh).sort());
});
