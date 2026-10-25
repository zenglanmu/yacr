// TEXT text-entry contract (ADR 0019 decision A, P7a).
//
// TEXT takes one point and then its text, and the text is the command's
// **terminal** step: submitting it completes the command, with no second
// confirm. The text therefore travels on its own IPC (`cad_set_tool_text`) —
// never `cad_command` — and the UI enters text mode only when the host's
// `tool.awaiting_text` flag says so. Nothing here may infer "needs text" from
// the tool key or from the captured point count, and nothing may rewrite what
// the user typed: an over-long draft is refused with the real bound shown.
//
// The keyboard rules are part of the contract too: Enter submits once (an IME
// composition's Enter never does), a single Escape only dismisses the field and
// the second cancels, and typed characters go to the field, never to the canvas.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  awaitingToolText,
  checkToolText,
  setToolText,
  textDraftState,
  textEscapeAction,
  textLength,
  TEXT_PICK_PROMPT_KEY,
  TEXT_PROMPT_KEY,
  TEXT_STEP_PROMPT_KEY,
  TEXT_TOO_LONG_KEY,
  TOOL_TEXT_COMMAND,
  TOOL_TEXT_MAX_LENGTH,
  toolAwaitingText,
} from '../src/text.ts';
import { handleCommandKey, parseCadStatus, TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad.ts';
import { COMMAND_CATALOG, filterCommandSuggestions, isSupportedCommand, setCommandCatalog, toolLabelKey } from '../src/commands.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon-model.ts';
import { readAppSource, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The router vocabulary once the text channel lands (ADR 0019 P7a): TEXT is a
// real selector, admission — not a local flag — is what enables the button.
const HOST_NAMES = ['TEXT', 'LINE', 'CONFIRM', 'CANCEL'];
setCommandCatalog(HOST_NAMES);
const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };
const withTool = tool => ({ ...loaded, tool });

// A structurally complete status payload, so `parseCadStatus` is exercised for
// real rather than through a partial stub.
const BASE = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  importProgress: null,
  error: null,
  canOpen: true,
  canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false,
  canSave: true,
  canPlot: true,
  canFit: true,
  backend: null,
  adapter: null,
  camera: null,
  importCount: 0,
  frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

const TEXT_TOOL = { key: 'draw.text', points: 1, ready: false };
const AWAITING = { ...TEXT_TOOL, awaiting_text: true };

const drawPanel = tabId => RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');

test('text mode is the host flag alone — never the tool key, never the point count', () => {
  assert.equal(awaitingToolText(withTool(AWAITING)), true);
  // The same tool, the same point count, without the flag: no text mode.
  assert.equal(awaitingToolText(withTool(TEXT_TOOL)), false);
  assert.equal(awaitingToolText(withTool({ ...TEXT_TOOL, ready: true })), false);
  // The flag with no point yet still opens text mode: the host decides, and a
  // count that disagrees is not a reason to guess.
  assert.equal(awaitingToolText(withTool({ ...AWAITING, points: 0 })), true);
  // Another tool's "ready"/open-ended state never opens it.
  assert.equal(awaitingToolText(withTool({ key: 'draw.line', points: 3, ready: true, openEnded: true })), false);
  // Only a real boolean opens it: a truthy string/number/object does not.
  for (const malformed of ['true', 'false', 1, 0, {}, [], null]) {
    assert.equal(toolAwaitingText({ key: 'draw.text', awaiting_text: malformed }), false, String(malformed));
  }
  for (const absent of [null, undefined, {}, { tool: null }, { tool: 'draw.text' }, { tool: [] }]) {
    assert.equal(awaitingToolText(absent), false, String(absent));
  }
});

test('the status payload validates the text flag instead of coercing it', () => {
  for (const tool of [null, TEXT_TOOL, AWAITING, { key: 'draw.text', awaiting_text: false }, { key: 'draw.line', points: 2 }]) {
    const status = { ...BASE, tool };
    assert.equal(parseCadStatus(status), status, JSON.stringify(tool));
  }
  // A present flag that is not a boolean is a contract violation, exactly like
  // the other optional flags: it must never be read as "awaiting text".
  for (const awaiting_text of ['true', 1, {}, [], null]) {
    assert.throws(() => parseCadStatus({ ...BASE, tool: { key: 'draw.text', awaiting_text } }), /CAD_STATUS_INVALID/);
  }
  // A tool block that is not an object (nor null) is malformed too.
  for (const tool of ['draw.text', 1, []]) {
    assert.throws(() => parseCadStatus({ ...BASE, tool }), /CAD_STATUS_INVALID/);
  }
});

test('Enter sends exactly one cad_set_tool_text carrying the typed text', async () => {
  const calls = [];
  const result = await setToolText(async (...args) => { calls.push(args); return { accepted: true }; }, 'Ground floor');
  assert.deepEqual(calls, [['cad_set_tool_text', { text: 'Ground floor' }]]);
  assert.equal(TOOL_TEXT_COMMAND, 'cad_set_tool_text');
  assert.deepEqual(result, { kind: 'accepted' });
  // Acceptance is queue admission: the completion comes from the next status.
  assert.equal('completed' in result, false);
  assert.equal('text' in result, false);
  // The text is sent exactly as typed — never trimmed, never rewritten.
  calls.length = 0;
  await setToolText(async (...args) => { calls.push(args); return { accepted: true }; }, '  Spaced  ');
  assert.deepEqual(calls, [['cad_set_tool_text', { text: '  Spaced  ' }]]);
});

test('empty, whitespace-only and over-long text never reach IPC', async () => {
  const invoke = () => assert.fail('a refused text reached IPC');
  for (const text of ['', '   ', '\n\t ', null, undefined, 42, {}]) {
    const result = await setToolText(invoke, text);
    assert.equal(result.kind, 'refused', String(text));
    assert.ok(result.error);
  }
  assert.equal((await setToolText(invoke, '   ')).reason, 'empty');
  assert.equal((await setToolText(invoke, null)).reason, 'kind');
  // The bound is inclusive: 1024 characters are sent, 1025 are refused.
  const max = 'a'.repeat(TOOL_TEXT_MAX_LENGTH);
  assert.equal(checkToolText(max).kind, 'ok');
  const over = 'a'.repeat(TOOL_TEXT_MAX_LENGTH + 1);
  const refused = checkToolText(over);
  assert.equal(refused.kind, 'refused');
  assert.equal(refused.reason, 'too_long');
  assert.equal(refused.length, TOOL_TEXT_MAX_LENGTH + 1);
  // Characters, not UTF-16 code units: the host counts `chars()`, so an astral
  // character is one character (this is 1024 characters, 2048 code units).
  assert.equal(textLength('😀'.repeat(1024)), 1024);
  assert.equal(checkToolText('😀'.repeat(TOOL_TEXT_MAX_LENGTH)).kind, 'ok');
  assert.equal(checkToolText('😀'.repeat(TOOL_TEXT_MAX_LENGTH + 1)).reason, 'too_long');
  // A real host refusal is explicit, never a silent success.
  for (const failing of [
    async () => { throw Error('queue full'); },
    async () => ({ accepted: false }),
    async () => null,
    async () => undefined,
  ]) {
    const result = await setToolText(failing, 'ok text');
    assert.equal(result.kind, 'failed');
    assert.ok(result.error);
  }
});

test('over the bound the limit is shown, the draft is kept and nothing is truncated', async () => {
  const draft = textDraftState('a'.repeat(TOOL_TEXT_MAX_LENGTH + 1));
  assert.deepEqual(draft, { length: TOOL_TEXT_MAX_LENGTH + 1, max: TOOL_TEXT_MAX_LENGTH, tooLong: true, empty: false });
  assert.equal(textDraftState('hello').tooLong, false);
  assert.equal(textDraftState('   ').empty, true);
  assert.equal(textDraftState('   ').tooLong, false);
  assert.equal(TOOL_TEXT_MAX_LENGTH, 1024);
  const commandline = await readModule('components/CommandLine.tsx');
  // The bound is rendered from the catalog with the real number, in both the
  // live warning and the refusal message.
  assert.ok(commandline.includes('t(TEXT_TOO_LONG_KEY, { max: TOOL_TEXT_MAX_LENGTH })'), 'the limit is shown from the catalog');
  assert.ok(commandline.includes('const tooLongText = draft.tooLong'));
  assert.ok(commandline.includes('role="alert"'), 'the limit/refusal is announced');
  // No silent truncation: the field has no `maxLength`, and the draft is only
  // cleared on acceptance.
  assert.ok(!commandline.includes('maxLength'), 'the field must not truncate the draft');
  assert.ok(commandline.includes('const [textDraft, setTextDraft] = useState(\'\')'));
  const accepted = commandline.indexOf("if (result.kind === 'accepted') {");
  const cleared = commandline.indexOf("setTextDraft('');", accepted);
  assert.ok(accepted !== -1 && cleared > accepted, 'the draft is cleared only after acceptance');
  // An over-long draft is not sent at all: the visible limit is the answer.
  assert.ok(commandline.includes('if (draft.empty || draft.tooLong) return;'));
});

test('one Escape dismisses the text field and only the next one cancels', () => {
  // While the text step is focused, Escape is a dismissal — the tool stays
  // awaiting text, so the command must not be cancelled.
  assert.equal(textEscapeAction({ awaitingText: true, focused: true }), 'dismiss');
  // With no field focused the press belongs to the global handler, which sends
  // CANCEL: that is the second press (or a press after the user clicked away).
  assert.equal(textEscapeAction({ awaitingText: true, focused: false }), 'cancel');
  // No text step: Escape keeps its panel-closing behaviour untouched.
  assert.equal(textEscapeAction({ awaitingText: false, focused: true }), null);
  assert.equal(textEscapeAction({ awaitingText: false, focused: false }), null);
  assert.equal(textEscapeAction(), null);
});

test('the Escape wiring keeps the two-step behaviour and never cancels from the field', async () => {
  const commandline = await readModule('components/CommandLine.tsx');
  const app = await readAppSource();
  // The field's own handler takes the focused press and hands the keyboard back.
  assert.ok(commandline.includes("textEscapeAction({ awaitingText: textMode, focused: true }) === 'dismiss'"));
  assert.ok(commandline.includes('event.preventDefault();'));
  assert.ok(commandline.includes('inputRef.current?.blur()'));
  // It never submits a command (and never CANCEL) itself: the second press is
  // the global tool handler's job.
  assert.ok(!/['"]CANCEL['"]/.test(commandline), 'the command line must not cancel the tool itself');
  assert.ok(!commandline.includes('shell.runCommand'));
  // The global handler runs after the field's handler, so a prevented press is
  // not also a tool command; a press with no field focused is.
  assert.ok(app.includes('if (event.defaultPrevented || event.metaKey || event.altKey) return;'));
  assert.ok(app.includes('toolCommandForKey(event.key, {'));
  assert.ok(app.includes('typing: isTyping(event.target)'));
  // First Escape while typing must never reach that branch; the existing blur
  // path is what restores the pointer lane.
  assert.ok(app.includes('if (isTyping(event.target)) event.target.blur();'));
});

test('an IME-composing Enter is ignored, and Enter submits once', () => {
  let submitted = 0;
  let prevented = 0;
  const event = props => ({ preventDefault() { prevented++; }, ...props });
  handleCommandKey(event({ key: 'Enter', isComposing: true }), () => { submitted++; });
  // Legacy IMEs report the committing Enter as keyCode 229 without `isComposing`.
  handleCommandKey(event({ key: 'Enter', keyCode: 229 }), () => { submitted++; });
  // A held Enter is one submit, not a burst.
  handleCommandKey(event({ key: 'Enter', repeat: true }), () => { submitted++; });
  assert.equal(submitted, 0);
  assert.equal(prevented, 0, 'an IME/held Enter must not be swallowed either');
  handleCommandKey(event({ key: 'Enter' }), () => { submitted++; });
  assert.equal(submitted, 1);
  assert.equal(prevented, 1);
  // Other keys are never submits.
  for (const key of ['Tab', 'Escape', 'a', ' ', 'ArrowDown']) {
    handleCommandKey(event({ key }), () => { submitted++; });
  }
  assert.equal(submitted, 1);
});

test('typed characters stay in the field and Enter never also runs a command', async () => {
  const commandline = await readModule('components/CommandLine.tsx');
  const app = await readAppSource();
  // The text draft is local component state; typing it never dispatches an IPC.
  assert.ok(commandline.includes('setTextDraft(event.target.value)'));
  assert.ok(!commandline.includes("invoke("), 'the view never talks to the host directly');
  // The text submit and the command submit are two distinct Enter paths, and the
  // text one returns before the command/nav path can run.
  const commandPath = commandline.indexOf('handleCommandKey(event, () => shell.submitCommand(effectiveText()));');
  const textPath = commandline.indexOf('handleCommandKey(event, submitText);');
  assert.ok(commandPath !== -1, 'the command Enter path must stay');
  assert.ok(textPath !== -1, 'the text Enter path must exist');
  assert.ok(textPath > commandPath);
  assert.ok(commandline.slice(0, commandPath).includes('if (!textMode)'), 'the command path is command-mode only');
  assert.ok(commandline.slice(commandPath, textPath).includes('return;'), 'text mode never falls through to a command');
  // One IPC for the text, from the shell, and nothing queued as a command.
  assert.ok(app.includes('setToolText(invoke, text)'));
  assert.ok(app.includes('submitToolText,'));
  assert.ok(!app.includes("invoke('cad_command', { text"), 'the text step must not ride cad_command');
  // The field owns the keyboard while it is focused (the pointer lane is
  // suspended), so a keystroke can never become a canvas shortcut.
  assert.ok(commandline.includes('shell.suspendPointer(true)'));
  assert.ok(commandline.includes('shell.suspendPointer(false)'));
  // Awaiting text never disables the field mid-edit (that would drop focus).
  assert.ok(commandline.includes('const disabledInput = !shell.cadReady || (shell.pending && !textMode);'));
});

test('the command line is a text prompt only while the host says so, then returns to command names', async () => {
  const commandline = await readModule('components/CommandLine.tsx');
  // The mode comes from the flag (through the shared reader) — no tool key and
  // no `points` may decide it.
  assert.ok(commandline.includes('awaitingToolText(shell.status)'));
  assert.ok(!commandline.includes('draw.text'), 'the mode must not be keyed to the TEXT tool');
  assert.ok(!commandline.includes('.points'), 'the mode must not be derived from the point count');
  // Text mode: the prompt, the payload and the step name all come from the
  // catalog, and the completion list is not offered.
  assert.ok(commandline.includes("placeholder={textMode ? t(TEXT_PROMPT_KEY) : t('command.placeholder')}"));
  assert.ok(commandline.includes("value={textMode ? textDraft : shell.command}"));
  assert.ok(commandline.includes('t(TEXT_STEP_PROMPT_KEY)'));
  assert.ok(commandline.includes('const listOpen = !textMode && focused'));
  // Leaving the step clears the draft, so the field is a command line again.
  assert.ok(commandline.includes('if (textMode) return;'));
  assert.ok(commandline.includes('setTextDraft(\'\');'));
  // A new text step focuses the field so typing can start right away.
  assert.ok(commandline.includes('inputRef.current?.focus();'));
});

test('TEXT is a draw affordance in the ribbon and the catalog, enabled only by router admission', async () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'TEXT');
  assert.ok(entry, 'TEXT must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.text');
  assert.equal(entry.icon, 'text');
  assert.equal(entry.group, 'draw');
  // Both draw panels carry it, declared exactly like the other draw commands.
  for (const tabId of ['home', 'draw']) {
    const item = drawPanel(tabId).items.find(candidate => candidate.id === 'TEXT');
    assert.ok(item, `${tabId}/draw must carry the TEXT affordance`);
    assert.equal(item.command, 'TEXT');
    assert.equal(item.labelKey, 'draw.kind.text');
    assert.equal(item.icon, 'text');
    assert.equal(item.action, undefined, 'TEXT must dispatch through the router');
    assert.equal(item.large, undefined, 'TEXT is a small draw button, not a primary tile');
    assert.equal(item.variants, undefined, 'TEXT has no variant popover');
    // HATCH still closes the draw group.
    const items = drawPanel(tabId).items;
    assert.equal(items.at(-1).id, 'HATCH', `${tabId}/draw still ends with HATCH`);
    assert.ok(ribbonCommandIds().includes('TEXT'));
  }
  // Router admission is the single enable gate.
  assert.equal(isSupportedCommand('TEXT'), true);
  assert.equal(filterCommandSuggestions('TEXT').find(suggestion => suggestion.id === 'TEXT').supported, true);
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('TEXT'), false);
  assert.equal(filterCommandSuggestions('TEXT').find(suggestion => suggestion.id === 'TEXT').supported, false);
  setCommandCatalog(HOST_NAMES);
  // The active-tool chip key is what the host key maps onto.
  assert.equal(toolLabelKey('draw.text'), 'tool.draw.text');
});

test('the awaiting-text chip names the text step instead of claiming a dimension is ready', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS['draw.text'], [TEXT_PICK_PROMPT_KEY]);
  assert.equal(TOOL_READY_PROMPTS['draw.text'], TEXT_STEP_PROMPT_KEY);
  // Before the point: the pick is named.
  assert.equal(toolStepPromptKey({ ...TEXT_TOOL, points: 0 }), 'tool.prompt.text.pick_point');
  // Awaiting text: the terminal step is named, whatever the count says.
  for (const tool of [AWAITING, { ...AWAITING, points: 0 }, { ...AWAITING, points: 7, ready: true }]) {
    assert.equal(toolStepPromptKey(tool), TEXT_STEP_PROMPT_KEY);
  }
  // The text step is never the shared dimension "ready" copy, and never a
  // dimension claim for the text tool.
  assert.notEqual(toolStepPromptKey({ ...TEXT_TOOL, points: 1, ready: true }), 'tool.prompt.dim.ready');
  assert.equal(toolStepPromptKey({ ...TEXT_TOOL, points: 1, ready: true }), TEXT_STEP_PROMPT_KEY);
  // The flag is key-agnostic: it names the step for any text-carrying tool.
  assert.equal(toolStepPromptKey({ key: 'draw.mleader', points: 0, awaiting_text: true }), TEXT_STEP_PROMPT_KEY);
  // A tool with no text step and no named picks is unchanged.
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 1, ready: false }), null);
});

test('every text-mode catalog key resolves in both locales', () => {
  const keys = [
    'draw.kind.text',
    'tool.draw.text',
    TEXT_PICK_PROMPT_KEY,
    TEXT_STEP_PROMPT_KEY,
    TEXT_PROMPT_KEY,
    TEXT_TOO_LONG_KEY,
  ];
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
  // The button and the chip say the same word, like the other draw commands.
  assert.equal(en['draw.kind.text'], en['tool.draw.text']);
  assert.equal(zh['draw.kind.text'], zh['tool.draw.text']);
  // The keys are the frozen contract names, not paraphrases.
  assert.equal(TEXT_PICK_PROMPT_KEY, 'tool.prompt.text.pick_point');
  assert.equal(TEXT_STEP_PROMPT_KEY, 'tool.prompt.text.type_text');
  assert.equal(TEXT_PROMPT_KEY, 'status.tool_text.prompt');
  assert.equal(TEXT_TOO_LONG_KEY, 'status.tool_text.too_long');
});

test('the TEXT glyph is a dedicated baseline mark, distinct from the hatch and dimension glyphs', async () => {
  const icons = await readModule('icons.tsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.jsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  assert.ok(defined.has('text'), "icons.jsx must define 'text'");
  // Ribbon and catalog agree on the dedicated name (an unknown name would make
  // <Icon> silently fall back to a dot).
  for (const tabId of ['home', 'draw']) {
    assert.equal(drawPanel(tabId).items.find(item => item.id === 'TEXT').icon, 'text', tabId);
  }
  assert.equal(COMMAND_CATALOG.find(command => command.id === 'TEXT').icon, 'text');
  // `text` belongs to TEXT alone: no other catalogued command reuses it.
  assert.equal(COMMAND_CATALOG.filter(command => command.icon === 'text').length, 1);
  // Every draw-panel glyph is a real name, so no draw button falls back to a dot.
  for (const tabId of ['home', 'draw']) {
    for (const item of drawPanel(tabId).items) {
      assert.ok(defined.has(item.icon), `${item.id} icon '${item.icon}' must exist`);
    }
  }
  // Shape: a capital A (two legs plus a crossbar) standing on a baseline, and no
  // dimension vocabulary — no extension lines, ticks, arrowheads or dashes.
  const glyph = map[1].match(/\n\s*text:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(glyph, 'expected a text glyph');
  assert.equal((glyph[1].match(/<path/g) ?? []).length, 3, 'text = A legs, A crossbar, baseline');
  assert.match(glyph[1], /M4 19\.5h16/, 'the baseline is one horizontal stroke');
  assert.match(glyph[1], /M8 14\.5 12 4\.5l4 10/, 'the A is two legs meeting at the apex');
  assert.match(glyph[1], /M9\.6 11h4\.8/, 'the A has a crossbar');
  assert.doesNotMatch(glyph[1], /<rect/, 'no hatch loop');
  assert.doesNotMatch(glyph[1], /<circle/, 'no vertex, feature or centre dot');
  assert.doesNotMatch(glyph[1], /strokeDasharray/);
  // The marks a user must not confuse it with stay separate glyphs.
  const neighbours = new Set(['hatch', 'dimlinear', 'dimaligned', 'dimradius', 'dimdiameter', 'dimcenter', 'dimangular', 'dimordinate', 'rectang', 'polygon', 'spline']);
  for (const name of neighbours) assert.ok(defined.has(name), `${name} must stay a glyph`);
  assert.equal(neighbours.has(COMMAND_CATALOG.find(command => command.id === 'TEXT').icon), false);
});
