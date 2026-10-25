// MLEADER user-facing surface contract (ADR 0019 decision F.2, P7c).
//
// MLEADER captures exactly two points — the **arrowhead position**, then the
// **landing point** — and then the *same* terminal text step `TEXT` uses
// (`cad_set_tool_text`, 1024 characters, no prefill, Enter submits, two-step
// Escape, IME never submits). The command has **no `Options` sub-prompt**
// (leader type / content type / max points / angular constraint), so nothing in
// this chrome may offer or imply one: no variant popover, no typed tool
// parameter, no extra pick.
//
// Nothing here infers a step from the tool key: the text mode is the host's
// `awaiting_text` flag alone, and the two pick prompts come from the real
// captured count. Every user-visible string is a catalog key.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  TEXT_STEP_PROMPT_KEY,
  awaitingToolText,
  checkToolText,
  setToolText,
  textEscapeAction,
  toolTextPrefill,
} from '../src/text.mjs';
import {
  TOOL_PARAMETER_SPECS,
  TOOL_PARAMETERS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  parseCadStatus,
  toolParameterViews,
  toolStepPromptKey,
} from '../src/cad.mjs';
import {
  CAPABILITY_GATES,
  COMMAND_CATALOG,
  GESTURE_ONLY_COMMANDS,
  admitCommandText,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands.mjs';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon-model.mjs';
import { readAppSource, readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The router's real vocabulary once P7c lands. Admission — not a local flag — is
// what enables the button.
const HOST_NAMES = ['MLEADER', 'TEXT', 'CONFIRM', 'CANCEL'];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };
const withTool = tool => ({ ...loaded, tool });

// A structurally complete status payload, so `parseCadStatus` is exercised for
// real instead of through a partial stub.
const BASE = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  importProgress: null,
  error: null,
  canOpen: true,
  canNew: true, gridEnabled: true, orthoEnabled: false,
  canSave: true,
  canPlot: true,
  canFit: true,
  backend: null,
  adapter: null,
  camera: null,
  importCount: 0,
  frameCount: 0,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

// The frozen surface names, written out here so a rename cannot pass silently.
const MLEADER = Object.freeze({
  id: 'MLEADER',
  labelKey: 'draw.kind.mleader',
  icon: 'mleader',
  toolKey: 'draw.mleader',
  toolLabelKey: 'tool.draw.mleader',
  arrow: 'tool.prompt.mleader.arrow',
  landing: 'tool.prompt.mleader.landing',
  text: 'tool.prompt.mleader.type_text',
});

// The three steps, in order: two picks and the terminal text.
const TOOL = points => ({ key: MLEADER.toolKey, points, ready: false });
const AWAITING = points => ({ ...TOOL(points), awaiting_text: true });

const drawPanel = tabId => RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');

test('MLEADER sits between TEXT and HATCH on both draw panels, with no variant popover', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const ids = items.map(item => item.id);
    const at = ids.indexOf(MLEADER.id);
    assert.ok(at !== -1, `${tabId}/draw must carry the MLEADER affordance`);
    // Annotation neighbours: directly after TEXT, and HATCH still closes the group.
    assert.equal(ids[at - 1], 'TEXT', `${tabId}: MLEADER follows TEXT`);
    assert.deepEqual(ids.slice(-4), ['OFFSET', 'TEXT', 'MLEADER', 'HATCH'], `${tabId}: fixed tail order`);
    assert.equal(items.at(-1).id, 'HATCH', `${tabId}/draw still ends with HATCH`);
    assert.equal(items.filter(item => item.id === MLEADER.id).length, 1, `${tabId}: one MLEADER button`);

    const item = items[at];
    assert.equal(item.command, MLEADER.id);
    assert.equal(item.labelKey, MLEADER.labelKey);
    assert.equal(item.icon, MLEADER.icon);
    assert.equal(item.action, undefined, 'MLEADER must dispatch through the router');
    assert.equal(item.large, undefined, 'MLEADER is a small draw button, not a primary tile');
    // The command has no `Options` sub-prompt, so a caret here would imply choices
    // the tool does not have (leader type / content type / max points / angle).
    assert.equal(item.variants, undefined, 'MLEADER has no variant popover');
    assert.ok(ribbonCommandIds().includes(MLEADER.id));
  }
  // No other panel smuggles the command in.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === MLEADER.id), false, `${tab.id}/${panel.id}`);
    }
  }
});

test('MLEADER is a catalogued draw command enabled only by router admission', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === MLEADER.id);
  assert.ok(entry, 'MLEADER must be in the command catalog');
  assert.equal(entry.labelKey, MLEADER.labelKey);
  assert.equal(entry.icon, MLEADER.icon);
  assert.equal(entry.group, 'draw');
  assert.equal(entry.variants, undefined, 'the listing must not imply variants either');
  // The listing keeps the draw family contiguous: TEXT, MLEADER, HATCH.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf(MLEADER.id), order.indexOf('TEXT') + 1);
  assert.equal(order.indexOf(MLEADER.id), order.indexOf('HATCH') - 1);

  // Router admission is the single gate — the same as every other draw command.
  assert.equal(isSupportedCommand('mleader'), true);
  assert.equal(admitCommandText('MLEADER', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('MLEADER', { status: null }).kind, 'unavailable');
  assert.equal(admitCommandText('MLEADER', { status: { ...loaded, loading: true } }).kind, 'unavailable');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('MLEADER'), false);
  assert.equal(admitCommandText('MLEADER', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // Neither capability-gated nor a gesture-only name.
  assert.equal(CAPABILITY_GATES[MLEADER.id], undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes(MLEADER.id), false);
  // The command line offers it with the same admission state and catalog label.
  const suggestion = filterCommandSuggestions('MLEADER').find(candidate => candidate.id === MLEADER.id);
  assert.ok(suggestion);
  assert.equal(suggestion.supported, true);
  assert.equal(suggestion.labelKey, MLEADER.labelKey);
  // The active-tool chip key is what the host's `tool.key` maps onto.
  assert.equal(toolLabelKey(MLEADER.toolKey), MLEADER.toolLabelKey);
});

test('the two picks are named by the captured count and the text step is never a pick', () => {
  // Exactly two point-index prompts: arrowhead, then landing. A third entry would
  // claim a pick the command does not take.
  assert.deepEqual(TOOL_STEP_PROMPTS[MLEADER.toolKey], [MLEADER.arrow, MLEADER.landing]);
  assert.equal(toolStepPromptKey(TOOL(0)), MLEADER.arrow);
  assert.equal(toolStepPromptKey(TOOL(1)), MLEADER.landing);
  // A malformed/absent count is read as "no points yet" — never a later step.
  assert.equal(toolStepPromptKey({ key: MLEADER.toolKey }), MLEADER.arrow);
  assert.equal(toolStepPromptKey({ key: MLEADER.toolKey, points: '1' }), MLEADER.arrow);
  assert.equal(toolStepPromptKey({ key: MLEADER.toolKey, points: -1 }), MLEADER.arrow);

  // Between the landing pick and the host's text flag the next step is the text.
  // MLEADER must never borrow the shared dimension copy (it produces a leader,
  // not a dimension) and must never claim a centre mark was defined.
  assert.equal(TOOL_READY_PROMPTS[MLEADER.toolKey], MLEADER.text);
  assert.equal(toolStepPromptKey(TOOL(2)), MLEADER.text);
  assert.equal(toolStepPromptKey(TOOL(9)), MLEADER.text);
  for (const tool of [TOOL(2), TOOL(9), AWAITING(1), AWAITING(2)]) {
    assert.notEqual(toolStepPromptKey(tool), 'tool.prompt.dim.ready');
    assert.notEqual(toolStepPromptKey(tool), 'tool.prompt.dim.center_ready');
  }
  // The text step itself is the host flag's, key-agnostic, and unchanged: the
  // table above never names it.
  for (const points of [0, 1, 2, 7]) {
    assert.equal(toolStepPromptKey(AWAITING(points)), TEXT_STEP_PROMPT_KEY, `points ${points}`);
  }
  assert.equal(TOOL_STEP_PROMPTS[MLEADER.toolKey].includes(TEXT_STEP_PROMPT_KEY), false);
  assert.notEqual(MLEADER.text, TEXT_STEP_PROMPT_KEY, 'MLEADER keeps its own next-step name');
  // The neighbouring tools keep their own derivations.
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 1 }), null);
  assert.equal(toolStepPromptKey({ key: 'draw.text', points: 0 }), 'tool.prompt.text.pick_point');
});

test('launching MLEADER walks arrow → landing → text, and the text field starts empty', async () => {
  // 1. The command is admitted by the router vocabulary, exactly like TEXT.
  assert.equal(admitCommandText('MLEADER', { status: loaded }).kind, 'admitted');

  // 2. The host's tool block is valid for every step of the capture.
  for (const tool of [TOOL(0), TOOL(1), TOOL(2), AWAITING(0), AWAITING(2)]) {
    const status = { ...BASE, tool };
    assert.equal(parseCadStatus(status), status, JSON.stringify(tool));
  }

  // 3. The three steps, in order.
  const flow = [TOOL(0), TOOL(1), TOOL(2)].map(tool => toolStepPromptKey(tool));
  assert.deepEqual(flow, [MLEADER.arrow, MLEADER.landing, MLEADER.text]);
  // No step before the text reports text mode: the flag is the only source.
  for (const tool of [TOOL(0), TOOL(1), TOOL(2)]) {
    assert.equal(awaitingToolText(withTool(tool)), false, JSON.stringify(tool));
  }

  // 4. The text step opens on the flag, and with **no prefill** — a fresh MLEADER
  // starts empty, exactly like a fresh TEXT.
  const textStatus = withTool(AWAITING(2));
  assert.equal(awaitingToolText(textStatus), true);
  assert.equal(toolTextPrefill(textStatus.tool), null, 'MLEADER sends no prefill');
  // Only a real string from the host is a prefill; a malformed one is ignored.
  for (const prefill of [undefined, null, 0, {}, []]) {
    assert.equal(toolTextPrefill({ ...AWAITING(2), text_prefill: prefill }), null, String(prefill));
  }

  // 5. The field is seeded from that prefill-read, so MLEADER lands on ''.
  const commandline = await readModule('components/CommandLine.jsx');
  assert.ok(commandline.includes("const [textDraft, setTextDraft] = useState('')"), 'the draft starts empty');
  assert.ok(commandline.includes("setTextDraft(toolTextPrefill(tool) ?? '')"), 'the step seeds from the host prefill only');
  assert.ok(commandline.includes('awaitingToolText(shell.status)'), 'the mode is the host flag');
  assert.ok(commandline.includes('const prefill = textMode ? toolTextPrefill(tool) : null;'));
  // Nothing here is special-cased for the leader: the view knows no tool key.
  assert.doesNotMatch(commandline, /mleader/i, 'the command line must not special-case MLEADER');
});

test('the text step keeps its generic semantics for MLEADER — no special case', async () => {
  // One Enter, one `cad_set_tool_text`, the text exactly as typed (never
  // rewritten, never prefixed with the leader style).
  const calls = [];
  assert.deepEqual(
    await setToolText(async (...args) => { calls.push(args); return { accepted: true }; }, 'Ground floor'),
    { kind: 'accepted' },
  );
  assert.deepEqual(calls, [['cad_set_tool_text', { text: 'Ground floor' }]]);
  // Empty text is refused before IPC — a leader is never created without its text.
  assert.equal((await setToolText(() => assert.fail('empty text reached IPC'), '   ')).reason, 'empty');
  assert.equal(checkToolText('').kind, 'refused');
  // Two-step Escape: the first press only dismisses the field; the tool stays
  // awaiting text, so a single press never cancels a MLEADER mid-typing.
  assert.equal(textEscapeAction({ awaitingText: awaitingToolText(withTool(AWAITING(2))), focused: true }), 'dismiss');
  assert.equal(textEscapeAction({ awaitingText: awaitingToolText(withTool(AWAITING(2))), focused: false }), 'cancel');

  // The submit path is the shared one: the shell's text channel, never
  // `cad_command` with a text payload; and no leader-specific branch in the app.
  const app = await readModule('App.jsx');
  assert.ok(app.includes('submitToolText'), 'the shared text submit is wired');
  assert.ok(app.includes('setToolText(invoke, text)'));
  assert.equal(app.includes("invoke('cad_command', { text"), false);
  assert.doesNotMatch(app, /mleader/i, 'the app shell must not special-case MLEADER');
  // Across the whole chrome, only the model, the listing, the prompt table and
  // the glyph mention the leader — the text step is inherited, not reimplemented.
  const all = await readAppSource();
  const owners = [...all.matchAll(/\/\* (.*?) \*\/\n([\s\S]*?)(?=\n\/\* |$)/g)]
    .filter(([, , body]) => /mleader/i.test(body))
    .map(([, path]) => path.split('/').pop())
    .sort();
  assert.deepEqual(owners, ['cad.mjs', 'commands.mjs', 'icons.jsx', 'ribbon-model.mjs']);
});

test('no control implies the Options sub-prompt MLEADER does not have', () => {
  // No typed tool parameter is projected for the leader: no stepper, no flag.
  assert.equal(TOOL_PARAMETERS[MLEADER.toolKey], undefined);
  assert.deepEqual(
    toolParameterViews({ tool: { key: MLEADER.toolKey, points: 0, ready: false, params: {} } }, () => true),
    [],
  );
  // Even a host that wrongly projected values renders nothing, because the group
  // has no map entry to reach a spec through.
  assert.deepEqual(
    toolParameterViews({
      tool: {
        key: MLEADER.toolKey,
        points: 1,
        params: { leaderType: 1, contentType: 2, maxPoints: 3, angularConstraint: 4 },
      },
    }, () => true),
    [],
  );
  for (const key of Object.keys(TOOL_PARAMETER_SPECS)) {
    assert.doesNotMatch(key, /leader|mleader/i, `no leader parameter spec may exist (${key})`);
  }
  // No second point-index table or pre-selection table entry sneaks a control in.
  assert.equal(TOOL_STEP_PROMPTS[MLEADER.toolKey].length, 2, 'exactly two picks — no extra point step');
});

test('every MLEADER catalog key resolves in both locales, and both locales agree', () => {
  const keys = [MLEADER.labelKey, MLEADER.toolLabelKey, MLEADER.arrow, MLEADER.landing, MLEADER.text];
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
  // The button, the command line and the active-tool chip name the same tool —
  // the same convention TEXT and HATCH follow.
  assert.equal(en[MLEADER.labelKey], en[MLEADER.toolLabelKey]);
  assert.equal(zh[MLEADER.labelKey], zh[MLEADER.toolLabelKey]);
  // The three step names are distinct copy (never one key reused for two steps).
  assert.equal(new Set([MLEADER.arrow, MLEADER.landing, MLEADER.text]).size, 3);
  // The keys are the frozen contract names, not paraphrases.
  assert.equal(MLEADER.toolKey, 'draw.mleader');
  assert.equal(MLEADER.labelKey, 'draw.kind.mleader');
  assert.equal(MLEADER.toolLabelKey, 'tool.draw.mleader');
  assert.equal(MLEADER.arrow, 'tool.prompt.mleader.arrow');
  assert.equal(MLEADER.landing, 'tool.prompt.mleader.landing');
  assert.equal(MLEADER.text, 'tool.prompt.mleader.type_text');
});

test('the MLEADER glyph is a dedicated leader-with-text mark in the shared icon language', async () => {
  const icons = await readModule('icons.jsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.jsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  assert.ok(defined.has(MLEADER.icon), `icons.jsx must define '${MLEADER.icon}'`);
  // Ribbon and catalog agree on the dedicated name (an unknown name would make
  // <Icon> silently fall back to a dot).
  for (const tabId of ['home', 'draw']) {
    assert.equal(drawPanel(tabId).items.find(item => item.id === MLEADER.id).icon, MLEADER.icon, tabId);
  }
  assert.equal(COMMAND_CATALOG.find(command => command.id === MLEADER.id).icon, MLEADER.icon);
  // `mleader` belongs to MLEADER alone, and every draw-panel glyph is real.
  assert.equal(COMMAND_CATALOG.filter(command => command.icon === MLEADER.icon).length, 1);
  for (const tabId of ['home', 'draw']) {
    for (const item of drawPanel(tabId).items) {
      assert.ok(defined.has(item.icon), `${item.id} icon '${item.icon}' must exist`);
    }
  }
  // Shape: one arrowhead, one leader that runs to a horizontal landing, and two
  // text strokes above that landing. No boundary loop and no circled feature, so
  // it cannot be read as `hatch`, `rectang` or `dimordinate`.
  const glyph = map[1].match(/\n\s*mleader:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(glyph, 'expected a mleader glyph');
  assert.equal((glyph[1].match(/<path/g) ?? []).length, 3, 'mleader = arrowhead, leader+landing, text lines');
  assert.doesNotMatch(glyph[1], /<rect/, 'no boundary loop');
  assert.doesNotMatch(glyph[1], /<circle/, 'no vertex, feature or centre dot');
  assert.doesNotMatch(glyph[1], /strokeDasharray/);
  // The leader ends in a horizontal landing segment (`h`), the dogleg the text
  // hangs from; a pure diagonal would read as a plain line.
  assert.match(glyph[1], /<path d="[^"]*h[\d.]+"/, 'the leader has a horizontal landing');
  // The two text strokes are two short horizontals in one path.
  assert.match(glyph[1], /<path d="M[\d.]+ [\d.]+h[\d.]+M[\d.]+ [\d.]+h[\d.]+"\s*\/>/);
  // The marks a user must not confuse it with stay separate glyphs.
  const neighbours = new Set([
    'text', 'hatch', 'dimordinate', 'dimlinear', 'dimaligned', 'dimradius', 'dimdiameter',
    'dimcenter', 'dimangular', 'rectang', 'polygon', 'line', 'spline',
  ]);
  for (const name of neighbours) assert.ok(defined.has(name), `${name} must stay a glyph`);
  assert.equal(neighbours.has(COMMAND_CATALOG.find(command => command.id === MLEADER.id).icon), false);
});
