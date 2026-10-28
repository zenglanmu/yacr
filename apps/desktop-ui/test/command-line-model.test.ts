// Contract tests for the command line's shared controller.
//
// The docked region and the mobile command bar render from one controller, so
// the decisions that used to live in the TSX are tested as **pure functions**
// here (mode, value, emptiness, suggestion list, Enter line, navigation
// intents, history stepping, the mobile sheet's capped rows) and the wiring is
// asserted as source contracts on the hook and the two presentations.
//
// Nothing here mounts React: these are the same plain-Node assertions the rest
// of the suite uses, not a DOM test.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import {
  MOBILE_HISTORY_ROWS,
  commandLineCanSubmit,
  commandLineDisabled,
  commandLineHistoryStep,
  commandLineListOpen,
  commandLineNavIntent,
  commandLineOptionId,
  commandLinePlaceholderKey,
  commandLineSubmitText,
  commandLineSuggestions,
  commandLineValue,
  mobileHistoryRows,
} from '../src/commands/command-line-model.ts';
import { TEXT_PROMPT_KEY } from '../src/tools/text.ts';

test('the field shows the draft in text mode and the command line otherwise', () => {
  assert.equal(commandLinePlaceholderKey(false), 'command.placeholder');
  assert.equal(commandLinePlaceholderKey(true), TEXT_PROMPT_KEY);
  assert.equal(commandLineValue(false, 'LINE', 'ignored'), 'LINE');
  assert.equal(commandLineValue(true, 'LINE', 'Ground floor'), 'Ground floor');
});

test('a queued command disables the field, except during a text step', () => {
  // No session yet: disabled in both modes.
  assert.equal(commandLineDisabled({ cadReady: false, pending: false, textMode: false }), true);
  assert.equal(commandLineDisabled({ cadReady: false, pending: false, textMode: true }), true);
  // A queued command: the command line waits, the text step must not drop focus.
  assert.equal(commandLineDisabled({ cadReady: true, pending: true, textMode: false }), true);
  assert.equal(commandLineDisabled({ cadReady: true, pending: true, textMode: true }), false);
  assert.equal(commandLineDisabled({ cadReady: true, pending: false, textMode: false }), false);
});

test('the suggestion list opens only for a command draft, focused, without history', () => {
  const open = { textMode: false, focused: true, command: 'ZO', historyOpen: false };
  assert.equal(commandLineListOpen(open), true);
  assert.equal(commandLineListOpen({ ...open, textMode: true }), false, 'a text step has no suggestions');
  assert.equal(commandLineListOpen({ ...open, focused: false }), false);
  assert.equal(commandLineListOpen({ ...open, command: '   ' }), false, 'an empty line lists nothing');
  assert.equal(commandLineListOpen({ ...open, historyOpen: true }), false, 'the two lists never compete');
});

test('the suggestion rows are the shared filter, and none in text mode', () => {
  const rows = commandLineSuggestions({ textMode: false, command: 'L', history: [], aliases: [] });
  assert.ok(rows.some(row => row.id === 'LINE'), 'a canonical prefix match is offered');
  assert.deepEqual(commandLineSuggestions({ textMode: true, command: 'L', history: [], aliases: [] }), []);
  // History floats used commands up, exactly as the filter already did.
  const frequent = commandLineSuggestions({ textMode: false, command: 'L', history: ['LINE', 'LINE'], aliases: [] });
  assert.equal(frequent[0].id, 'LINE');
});

test('Enter keeps a typed argument but accepts a highlighted bare name', () => {
  const suggestions = [{ id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', supported: true }];
  // `ZOOM 2`: the row is a name, the line carries the factor, so the line stays.
  assert.equal(
    commandLineSubmitText({ command: 'ZOOM 2', highlight: 0, suggestions: [{ ...suggestions[0], id: 'ZOOM' }] }),
    'ZOOM 2',
  );
  // `ZO` + a highlighted row: the name is the answer.
  assert.equal(commandLineSubmitText({ command: 'ZO', highlight: 0, suggestions }), 'ZOOMIN');
  // Nothing highlighted: the line is submitted verbatim.
  assert.equal(commandLineSubmitText({ command: 'ZO', highlight: -1, suggestions }), 'ZO');
  assert.equal(commandLineSubmitText({ command: ' zo ', highlight: 5, suggestions }), ' zo ');
});

test('the send affordance is inert until there is something to send', () => {
  assert.equal(commandLineCanSubmit(false, 'LINE', false, false), true);
  assert.equal(commandLineCanSubmit(false, '   ', false, false), false);
  assert.equal(commandLineCanSubmit(true, 'LINE', false, false), true, 'the text step ignores the command draft');
  assert.equal(commandLineCanSubmit(true, 'LINE', true, false), false, 'an empty text step sends nothing');
  assert.equal(commandLineCanSubmit(true, 'LINE', false, true), false, 'an over-long draft is never sent');
});

test('option ids are whitespace-free, so aria-activedescendant names one row', () => {
  assert.equal(commandLineOptionId({ id: 'ZOOM' }), 'completion-ZOOM');
  assert.equal(commandLineOptionId({ id: 'ZOOM EXTENTS' }), 'completion-ZOOM-EXTENTS');
  assert.ok(!commandLineOptionId({ id: 'CHAMFER POLYLINE' }).includes(' '));
});

test('arrow keys step the list while it is open and the history otherwise', () => {
  const open = { listOpen: true, suggestionCount: 3, highlight: 1 };
  assert.deepEqual(commandLineNavIntent({ key: 'ArrowDown', ...open }), { kind: 'suggestion', delta: 1 });
  assert.deepEqual(commandLineNavIntent({ key: 'ArrowUp', ...open }), { kind: 'suggestion', delta: -1 });
  assert.deepEqual(
    commandLineNavIntent({ key: 'ArrowUp', listOpen: false, suggestionCount: 0, highlight: -1 }),
    { kind: 'history', delta: -1 },
  );
  // Tab accepts a highlighted row, and nothing when the list is closed or the
  // highlight is unset.
  assert.deepEqual(commandLineNavIntent({ key: 'Tab', ...open }), { kind: 'accept' });
  assert.equal(commandLineNavIntent({ key: 'Tab', listOpen: false, suggestionCount: 3, highlight: 1 }), null);
  assert.equal(commandLineNavIntent({ key: 'Tab', listOpen: true, suggestionCount: 3, highlight: -1 }), null);
  assert.equal(commandLineNavIntent({ key: 'Tab', listOpen: true, suggestionCount: 3, highlight: 3 }), null);
  // Every other key is not a navigation key.
  for (const key of ['a', 'Enter', 'Escape', ' ']) {
    assert.equal(commandLineNavIntent({ key, ...open }), null, key);
  }
});

test('the history cursor walks back to the newest line and stops at the draft', () => {
  const history = ['A', 'B', 'C'];
  // Up from the draft lands on the newest entry.
  assert.deepEqual(
    commandLineHistoryStep({ history, cursor: null, delta: -1, draft: '' }),
    { cursor: 2, text: 'C', changed: true },
  );
  assert.deepEqual(
    commandLineHistoryStep({ history, cursor: 2, delta: -1, draft: '' }),
    { cursor: 1, text: 'B', changed: true },
  );
  // The oldest entry is a wall: the step does not move, so no state changes.
  assert.deepEqual(
    commandLineHistoryStep({ history, cursor: 0, delta: -1, draft: 'typed' }),
    { cursor: 0, text: 'A', changed: false },
  );
  // Down past the newest entry returns to the draft.
  assert.deepEqual(
    commandLineHistoryStep({ history, cursor: 2, delta: 1, draft: 'typed' }),
    { cursor: null, text: 'typed', changed: true },
  );
  // Nothing recorded: the arrow keys do nothing at all.
  assert.deepEqual(
    commandLineHistoryStep({ history: [], cursor: null, delta: -1, draft: 'x' }),
    { cursor: null, text: 'x', changed: false },
  );
});

test('the mobile history sheet is newest-first and capped, refusals included', () => {
  assert.equal(MOBILE_HISTORY_ROWS, 12);
  // The sheet renders the one log: commands and refusal sentences alike.
  const log = Array.from({ length: 30 }, (_, index) => ({ kind: index % 3 === 2 ? 'error' : 'command', text: `CMD${index}` }));
  const rows = mobileHistoryRows(log);
  assert.equal(rows.length, MOBILE_HISTORY_ROWS);
  assert.equal(rows[0].text, 'CMD29', 'newest first');
  assert.equal(rows.at(-1).text, 'CMD18');
  assert.equal(rows[0].kind, 'error', 'the kind travels with the row');
  assert.equal(rows[1].kind, 'command', 'a command row keeps its own kind');
  // The cap is a parameter, so a wider sheet can ask for more without a new rule.
  assert.deepEqual(mobileHistoryRows(log, 2).map(row => row.text), ['CMD29', 'CMD28']);
  assert.deepEqual(mobileHistoryRows(log, 0), []);
  assert.deepEqual(mobileHistoryRows([], 5), []);
  // The full record is untouched: the sheet is a shortcut list, not the store.
  assert.equal(log.length, 30);
});

test('the controller owns the command-line rules and both presentations render from it', async () => {
  const hook = await readModule('commands/use-command-line.ts');
  const model = await readModule('commands/command-line-model.ts');
  const desktop = await readModule('components/CommandLine.tsx');
  const mobile = await readModule('components/MobileCommandBar.tsx');
  const field = await readModule('components/CommandField.tsx');
  // One controller, two presentations.
  assert.ok(hook.includes('export function useCommandLine(): CommandLineController'));
  assert.ok(desktop.includes("import { useCommandLine } from '../commands/use-command-line.ts';"));
  assert.ok(mobile.includes("import { useCommandLine } from '../commands/use-command-line.ts';"));
  assert.ok(desktop.includes('const line = useCommandLine();'));
  assert.ok(mobile.includes('const line = useCommandLine();'));
  // The rules live in the model, not in the hook and certainly not in the views.
  assert.equal(hook.includes('splitCommandText('), false, 'the model owns the line grammar');
  for (const source of [desktop, mobile]) {
    assert.equal(source.includes('filterCommandSuggestions('), false, 'no view filters suggestions itself');
    assert.equal(source.includes('stepHistoryCursor('), false, 'no view steps the history itself');
  }
  assert.ok(model.includes('filterCommandSuggestions(command, { history, aliases })'));
  assert.ok(model.includes('stepHistoryCursor(history, cursor, delta)'));
  // One field, so both bars wire the same attributes and key handling.
  assert.ok(field.includes('<label className="sr-only" htmlFor={id}>'));
  assert.ok(field.includes('aria-activedescendant={line.activeOptionId}'));
  assert.ok(field.includes('aria-invalid={line.invalid ? true : undefined}'));
  assert.ok(field.includes('onKeyDown={line.onKeyDown}'));
  assert.ok(desktop.includes('<CommandField id="cad-command" className="commandline__input" line={line} />'));
  assert.ok(mobile.includes('<CommandField id="cad-command" className="mobile-commandbar__input" line={line} enterKeyHint="send" />'));
  // The text step's catalog keys are read in the hook, not spelled by a view.
  assert.ok(hook.includes('textStepKey: textMode ? TEXT_STEP_PROMPT_KEY : null'));
  assert.ok(hook.includes('t(commandLinePlaceholderKey(textMode))'));
  assert.ok(desktop.includes('t(line.textStepKey)'));
  // The mobile bar keeps the upward sheet, and it is an overlay in the same tree.
  assert.ok(mobile.includes('className="mobile-sheet"'));
  assert.equal(mobile.includes('createPortal'), false, 'the sheet is in-DOM, not a portal');
});

test('the docked region keeps its visual identity while rendering the controller', async () => {
  const desktop = await readModule('components/CommandLine.tsx');
  // Same region and same classes as before the extraction: the desktop output
  // must not change when the logic moves.
  assert.ok(desktop.includes("className=\"region region--cmdline commandline app-region\""));
  assert.ok(desktop.includes('className="commandline__row"'));
  assert.ok(desktop.includes('className="completions"'));
  assert.ok(desktop.includes('className="history-panel"'));
  assert.ok(desktop.includes('className="commandlog"'));
  // The docked region still owns its own echo/history/completion surfaces; those
  // are the parts the mobile bar replaces with one bounded sheet.
  assert.ok(desktop.includes('commandLogView(logEntries, t)'));
  assert.equal(desktop.includes('useCommandLine'), true);
});
