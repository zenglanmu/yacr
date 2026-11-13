// Contract tests for the plugin `Editor` prompt in the chrome.
//
// A pending prompt turns the command line into **answer data**: the typed line
// travels on `cad_set_tool_text`, an empty line is `CONFIRM`, Escape is `CANCEL`,
// and no host-command suggestion or admission is offered for the line. These are
// the same plain-Node assertions the rest of the suite uses — the wire parser, the
// routing module and the echo model are imported directly, and the React wiring is
// asserted as source contracts (no test here mounts React).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import { parseCadStatus, parseCadPendingPrompt, pendingPromptOf } from '../src/cad/cad-status.ts';
import { pluginPromptIntent, submitPluginPrompt } from '../src/commands/plugin-prompt.ts';
import { setToolText } from '../src/tools/text.ts';
import { toolCommandForKey } from '../src/commands/commands.ts';
import {
  commandLineCanSubmit,
  commandLineListOpen,
  commandLinePlaceholderKey,
  commandLineSuggestions,
} from '../src/commands/command-line-model.ts';
import {
  PLUGIN_ECHO_MAX_LINES,
  pluginEchoLines,
  pluginEchoSnapshot,
  startPluginEcho,
} from '../src/commands/plugin-message-echo.ts';
import { appendPluginMessage, resetPluginMessages } from '../src/plugin-api/messages.ts';

/** The required `cad_status` fields (same shape as performance-status.test.ts). */
const base = {
  documentLabel: null, entityCount: null, loading: false, importProgress: null,
  error: null, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false,
  snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false,
  backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

const status = (overrides = {}) => ({ ...base, ...overrides });

const prompt = {
  kind: 'string',
  message: 'Block name:',
  keywords: [],
  error: null,
};

const rejects = (value, message) => {
  assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/, message);
};

test('a status with no pending prompt, null, and a well-formed block all parse', () => {
  // Absent (an older host) and explicit `null` both mean "nothing is pending".
  assert.equal(pendingPromptOf(parseCadStatus(status())), null);
  assert.equal(pendingPromptOf(parseCadStatus(status({ pendingPrompt: null }))), null);

  const double = { kind: 'double', message: 'Width <0.5>:', keywords: [], error: null };
  const parsed = parseCadStatus(status({ pendingPrompt: double }));
  assert.deepEqual(pendingPromptOf(parsed), double);

  const keyword = {
    kind: 'keyword',
    message: 'Choose a mode:',
    keywords: ['Yes', 'No', 'Cancel'],
    error: 'Invalid option.',
  };
  assert.deepEqual(pendingPromptOf(parseCadStatus(status({ pendingPrompt: keyword }))), keyword);
});

test('every documented prompt kind is accepted', () => {
  for (const kind of ['point', 'entity', 'selection', 'string', 'double', 'integer', 'keyword']) {
    const block = { ...prompt, kind };
    assert.equal(pendingPromptOf(parseCadStatus(status({ pendingPrompt: block }))).kind, kind);
  }
});

test('an unknown prompt kind is a parse refusal, never a coerced prompt', () => {
  // The chrome routes the user's next line by this kind, so a value the contract
  // does not name must stop the poll rather than become "some prompt".
  rejects(status({ pendingPrompt: { ...prompt, kind: 'color' } }), 'unknown kind');
  rejects(status({ pendingPrompt: { ...prompt, kind: 'Point' } }), 'case is significant');
  rejects(status({ pendingPrompt: { ...prompt, kind: null } }), 'null kind');
  rejects(status({ pendingPrompt: { ...prompt, kind: 3 } }), 'non-string kind');
});

test('a malformed prompt block is refused rather than guessed', () => {
  rejects(status({ pendingPrompt: 42 }), 'non-object block');
  rejects(status({ pendingPrompt: [] }), 'array block');
  rejects(status({ pendingPrompt: { ...prompt, message: null } }), 'non-string message');
  rejects(status({ pendingPrompt: { ...prompt, keywords: 'Yes/No' } }), 'keyword list is a string');
  rejects(status({ pendingPrompt: { ...prompt, keywords: ['Yes', 2] } }), 'non-string keyword');
  rejects(status({ pendingPrompt: { ...prompt, error: 0 } }), 'non-string error');
  // A block missing a key is not "a prompt with a default": it is malformed.
  const { error: _error, ...noError } = prompt;
  rejects(status({ pendingPrompt: noError }), 'missing error key');
  // The standalone reader refuses the same shapes.
  assert.throws(() => parseCadPendingPrompt(null), /CAD_STATUS_INVALID/);
});

test('a prompt is answer data, and an empty line is the confirm answer', () => {
  assert.deepEqual(pluginPromptIntent(null, 'LINE'), { kind: 'command' });
  assert.deepEqual(pluginPromptIntent(undefined, ''), { kind: 'command' });
  assert.deepEqual(pluginPromptIntent(prompt, ''), { kind: 'confirm' });
  assert.deepEqual(pluginPromptIntent(prompt, '   '), { kind: 'confirm' });
  // The text is submitted exactly as typed: the host matches keywords and parses
  // numbers in its own spelling rules, and `getString` keeps its spaces.
  assert.deepEqual(pluginPromptIntent(prompt, '  two words '), { kind: 'answer', text: '  two words ' });
  // A line that looks like a command is an answer while the prompt is pending.
  assert.deepEqual(pluginPromptIntent(prompt, 'LINE'), { kind: 'answer', text: 'LINE' });
});

test('a typed answer reaches cad_set_tool_text and never cad_command', async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push([command, args]);
    return { accepted: true };
  };
  let confirms = 0;
  const deps = {
    confirm: () => { confirms += 1; },
    answer: text => setToolText(invoke, text),
  };

  const result = await submitPluginPrompt(prompt, 'LINE', deps);
  assert.deepEqual(result, { kind: 'accepted' });
  assert.deepEqual(calls, [['cad_set_tool_text', { text: 'LINE' }]]);
  assert.equal(confirms, 0, 'a typed answer is not a confirm');

  const keyword = await submitPluginPrompt(prompt, 'fast', deps);
  assert.deepEqual(keyword, { kind: 'accepted' });
  assert.deepEqual(calls, [
    ['cad_set_tool_text', { text: 'LINE' }],
    ['cad_set_tool_text', { text: 'fast' }],
  ]);
  assert.equal(calls.some(([command]) => command === 'cad_command'), false, 'no answer is a command');
});

test('an empty prompt line answers CONFIRM through the shell funnel', async () => {
  const calls = [];
  let confirms = 0;
  const result = await submitPluginPrompt(prompt, '', {
    confirm: () => { confirms += 1; calls.push(['cad_command', { command: 'CONFIRM' }]); },
    answer: () => { calls.push(['cad_set_tool_text', { text: 'never' }]); return Promise.resolve({ kind: 'accepted' }); },
  });
  assert.deepEqual(result, { kind: 'accepted' });
  assert.equal(confirms, 1);
  assert.deepEqual(calls, [['cad_command', { command: 'CONFIRM' }]]);
});

test('the text channel refuses an over-long answer before IPC', async () => {
  const calls = [];
  const result = await submitPluginPrompt(prompt, 'x'.repeat(2000), {
    confirm: () => assert.fail('a typed answer is not a confirm'),
    answer: text => setToolText(async (...args) => { calls.push(args); return { accepted: true }; }, text),
  });
  assert.equal(result.kind, 'refused');
  assert.equal(result.reason, 'too_long');
  assert.deepEqual(calls, [], 'a refused draft never reaches IPC');
});

test('no prompt leaves the line on the command lane (ignored, no IPC)', async () => {
  const result = await submitPluginPrompt(null, 'LINE', {
    confirm: () => assert.fail('no prompt to confirm'),
    answer: () => assert.fail('no prompt to answer'),
  });
  assert.deepEqual(result, { kind: 'ignored' });
});

test('Enter and Escape answer the pending prompt from the drawing surface', () => {
  // A prompt is not an active tool (`tool` stays whatever it was), so the keys
  // are claimed through the prompt flag alone.
  assert.equal(toolCommandForKey('Enter', { promptPending: true }), 'CONFIRM');
  assert.equal(toolCommandForKey('Escape', { promptPending: true }), 'CANCEL');
  // The field owns the answer path while focused, and an IME or held key is not
  // an answer (the shared key rule).
  assert.equal(toolCommandForKey('Enter', { promptPending: true, typing: true }), null);
  assert.equal(toolCommandForKey('Escape', { promptPending: true, typing: true }), null);
  assert.equal(toolCommandForKey('Enter', { promptPending: true, composing: true }), null);
  assert.equal(toolCommandForKey('Enter', { promptPending: true, repeat: true }), null);
  // With no prompt and no tool the keys keep their old owners: Escape closes a
  // panel, Enter does nothing.
  assert.equal(toolCommandForKey('Escape', {}), null);
  assert.equal(toolCommandForKey('Enter', {}), null);
  // A prompt never turns another key into a command.
  for (const key of ['F2', 'F3', 'a', 'Tab', ' ']) {
    assert.equal(toolCommandForKey(key, { promptPending: true }), null, key);
  }
});

test('command suggestions and admission are suppressed while a prompt is pending', () => {
  // The typed line is an answer: no host-command row may be offered for it.
  const input = { textMode: false, focused: true, command: 'LIN', historyOpen: false };
  assert.equal(commandLineListOpen(input), true);
  assert.equal(commandLineListOpen({ ...input, promptMode: true }), false);
  assert.deepEqual(commandLineSuggestions({ textMode: false, promptMode: true, command: 'LIN', history: [], aliases: [] }), []);
  assert.equal(commandLineListOpen({ ...input, promptMode: true, command: '' }), false);
  // The field's send affordance stays live with nothing typed: that is CONFIRM.
  assert.equal(commandLineCanSubmit(false, '', true, false), false);
  assert.equal(commandLineCanSubmit(false, '', true, false, true), true);
  // A prompt asks for an answer, not for a command name.
  assert.equal(commandLinePlaceholderKey(true), 'status.tool_text.prompt');
  assert.equal(commandLinePlaceholderKey(false, true), 'command.plugin_prompt.placeholder');
  assert.equal(commandLinePlaceholderKey(false), 'command.placeholder');
});

test('the prompt branch runs before the command lane, in both the hook and the shell keys', async () => {
  const hook = await readModule('commands/use-command-line.ts');
  // The prompt mode is read from the host block, never inferred from the tool.
  assert.ok(hook.includes('const prompt = shell.pluginPrompt;'));
  assert.ok(hook.includes('const promptMode = prompt !== null;'));
  // The answer branch sits in `submit()` before the command submit, and Escape
  // answers CANCEL instead of falling through to the field/panel behaviour.
  const submitAt = hook.indexOf('function submit()');
  const promptBranchAt = hook.indexOf('if (promptMode) {', submitAt);
  const commandSubmitAt = hook.indexOf('shell.submitCommand(commandLineSubmitText(', submitAt);
  assert.ok(promptBranchAt !== -1 && commandSubmitAt !== -1, 'both branches exist');
  assert.ok(promptBranchAt < commandSubmitAt, 'the prompt answer is routed first');
  assert.ok(hook.includes('void submitPluginPrompt(prompt, line, {'));
  assert.ok(hook.includes('confirm: () => shell.submitCommand(\'CONFIRM\'),'));
  assert.ok(hook.includes('void shell.submitCommand(\'CANCEL\');'));
  // The hook never admits an answer as a command.
  assert.equal(hook.includes('admitCommandText('), false, 'no admission of an answer');
  assert.equal(hook.includes('queueCadCommand('), false, 'no router lane of its own');

  // The shell gives Enter/Escape to the prompt from the drawing surface, before
  // the panel-closing Escape branch.
  const app = await readModule('App.tsx');
  assert.ok(app.includes('const pluginPromptPending = status?.pendingPrompt != null;'));
  assert.ok(app.includes('promptPending: pluginPromptPending,'));
  const keysAt = app.indexOf("toolCommandForKey(event.key, {");
  const panelEscapeAt = app.indexOf("if (event.key === 'Escape') {", keysAt);
  assert.ok(keysAt !== -1 && panelEscapeAt !== -1);
  assert.ok(keysAt < panelEscapeAt, 'the key table runs before Escape closes a panel');
  assert.ok(app.includes('pluginPrompt: status?.pendingPrompt ?? null,'));
});

test('a prompt that clears returns the line to command mode and drops only its draft', async () => {
  const hook = await readModule('commands/use-command-line.ts');
  // Leaving the prompt resets the answer draft; the command draft was never
  // touched, so it comes back intact.
  const clearAt = hook.indexOf('if (promptMode) return;');
  assert.ok(clearAt !== -1);
  assert.ok(hook.slice(clearAt, clearAt + 200).includes("setPromptDraft('');"));
  assert.ok(hook.includes('value: promptMode ? promptDraft : commandLineValue(textMode, shell.command, textDraft),'));
  // And the pure model: once the block is gone the line is a command again.
  assert.deepEqual(pluginPromptIntent(null, 'LIN'), { kind: 'command' });
  assert.equal(commandLineListOpen({ textMode: false, promptMode: false, focused: true, command: 'LIN', historyOpen: false }), true);
});

test('the plugin echo keeps the newest bounded lines and never invents one', () => {
  assert.deepEqual(pluginEchoLines(['one', 'two']), ['one', 'two']);
  assert.deepEqual(pluginEchoLines(['a', 'b', 'c', 'd']), ['b', 'c', 'd']);
  assert.deepEqual(pluginEchoLines(['a', 'b', 'c', 'd'], 2), ['c', 'd']);
  assert.equal(pluginEchoLines(['a', 'b', 'c', 'd']).length, PLUGIN_ECHO_MAX_LINES);
  // Only non-blank strings are messages.
  assert.deepEqual(pluginEchoLines(['ok', '', '   ', null, 42, { text: 'no' }]), ['ok']);
  assert.deepEqual(pluginEchoLines('hello'), []);
  assert.deepEqual(pluginEchoLines(null), []);
  assert.deepEqual(pluginEchoLines(['a'], 0), []);
});

test('the echo follows the plugin message store, bounded and newest first', () => {
  resetPluginMessages();
  const stop = startPluginEcho();
  assert.deepEqual(pluginEchoSnapshot(), [], 'nothing written yet');

  appendPluginMessage('ready');
  appendPluginMessage('step 2');
  assert.deepEqual(pluginEchoSnapshot(), ['ready', 'step 2']);

  // The store keeps eight lines; the echo shows the newest three.
  for (const line of ['a', 'b', 'c', 'd', 'e']) appendPluginMessage(line);
  assert.equal(pluginEchoSnapshot().length, PLUGIN_ECHO_MAX_LINES);
  assert.deepEqual(pluginEchoSnapshot(), ['c', 'd', 'e']);

  stop();
  appendPluginMessage('after dispose');
  assert.deepEqual(pluginEchoSnapshot(), ['c', 'd', 'e'], 'a disposed echo stops following');
  resetPluginMessages();
});

test('both presentations show the prompt, its keywords, its error and the echo', async () => {
  const commandLine = await readModule('components/CommandLine.tsx');
  assert.ok(commandLine.includes('line.promptMessage'));
  assert.ok(commandLine.includes('line.promptKeywords.map('));
  assert.ok(commandLine.includes('line.chooseKeyword(keyword)'));
  assert.ok(commandLine.includes('className="commandline__prompt-step"'));
  assert.ok(commandLine.includes('className="commandline__echo"'));
  assert.ok(commandLine.includes('className="commandline__echo-line is-invalid"'));
  // The history toggle steps aside for a prompt, like it does for a text step.
  assert.ok(commandLine.includes('{line.textMode || line.promptMode ? null : ('));

  const mobile = await readModule('components/MobileCommandBar.tsx');
  assert.ok(mobile.includes('className="mobile-echo"'));
  assert.ok(mobile.includes('className="mobile-echo__prompt"'));
  assert.ok(mobile.includes('className="mobile-echo__error"'));
  assert.ok(mobile.includes("const showHistory = shell.historyOpen && !line.textMode && !line.promptMode;"));
});
