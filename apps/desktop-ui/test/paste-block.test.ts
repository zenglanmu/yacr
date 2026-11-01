// PASTEBLOCK (2026-11-01) contracts: the clipboard's "paste as block" shape.
//
// The command takes the in-process clipboard and creates a **new auto-named
// block** (`PasteBlock1`, `PasteBlock2`, …) plus one reference at a picked
// insertion point. It is a one-point capture with no text step and no parameter,
// and the clipboard is not cleared. This suite pins the four surfaces the chrome
// owns — catalog, ribbon, shortcut menu, accelerators — and the prompt table, as
// values or source assertions; nothing here mounts the UI or checks a pixel.
//
// `HOST_NAMES` stands in for the router vocabulary: the Rust lane adds the
// `PASTEBLOCK` selector (`CommandId::PasteBlock`) in the same worktree, but these
// tests run without a host, so the mirror is what makes admission testable. These
// assertions describe the chrome's side of the contract, not a host that was
// exercised end to end. If the host never advertises `PASTEBLOCK`, every
// affordance below renders disabled through the shared admission — never locally
// enabled.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
import { acceleratorCommand, acceleratorDispatch } from '../src/commands/accelerators.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { admitCommandText, setCommandCatalog, toolLabelKey } from '../src/commands/commands.ts';
import { buildContextMenu, menuItems } from '../src/commands/context-menu.ts';
import { ribbonTab } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const loaded = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};

const ctrl = (key, extra = {}) => ({ key, ctrlKey: true, ...extra });

test('PASTEBLOCK is catalogued beside the clipboard family under the shared copy glyph', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'PASTEBLOCK');
  assert.ok(entry, 'PASTEBLOCK must be a catalogued display entry');
  assert.deepEqual({ ...entry }, {
    id: 'PASTEBLOCK',
    labelKey: 'draw.kind.pasteblock',
    icon: 'copy',
    group: 'modify',
  });
  // It sits directly after PASTEORIG, the clipboard run's last shipped row, so no
  // other catalog row moves.
  const ids = COMMAND_CATALOG.map(command => command.id);
  assert.equal(ids.indexOf('PASTEBLOCK'), ids.indexOf('PASTEORIG') + 1);
  // The catalog is the display half; the router vocabulary is the admission half.
  assert.equal(admitCommandText('PASTEBLOCK', { status: loaded }).kind, 'admitted');
});

test('the ribbon offers it as a small button in the Home clipboard panel, with no variant popover', () => {
  const home = ribbonTab('home');
  const panel = home.panels.find(candidate => candidate.id === 'clipboard');
  assert.ok(panel, 'the Home tab keeps its clipboard panel');
  const index = panel.items.findIndex(item => item.id === 'PASTEBLOCK');
  assert.ok(index >= 0, 'the clipboard panel must offer PASTEBLOCK');
  const item = panel.items[index];
  assert.equal(item.command, 'PASTEBLOCK');
  assert.equal(item.labelKey, 'draw.kind.pasteblock');
  assert.equal(item.icon, 'copy');
  // Small button (no `large`) and no caret: one pick is the whole input, the block
  // name is generated, and there is no scale/rotation/parameter to choose.
  assert.notEqual(item.large, true);
  assert.equal(item.variants, undefined);
  // It sits directly beside COPY, the clipboard command it complements, and the
  // shipped rows keep their relative order.
  assert.equal(panel.items[index - 1].id, 'COPY');
  const shippedOrder = panel.items
    .filter(candidate => candidate.id !== 'PASTEBLOCK')
    .map(candidate => candidate.id);
  assert.deepEqual(shippedOrder, ['SELECTALL', 'COPY', 'MOVE', 'ROTATE']);
});

test('the shortcut menu carries the third paste beside Paste and Paste original', () => {
  const ready = {
    selection: { count: 1 },
    tool: null,
    lastCommand: 'COPYCLIP',
    canFit: true,
    hasDocument: true,
  };
  const section = buildContextMenu(ready).sections.find(candidate => candidate.id === 'clipboard');
  assert.ok(section, 'the clipboard section exists');
  const items = section.items;
  assert.deepEqual(items.map(item => [item.id, item.command, item.key]), [
    ['clipboard.cut', 'CUTCLIP', 'menu.clipboard.cut'],
    ['clipboard.cutbase', 'CUTBASE', 'menu.clipboard.cutbase'],
    ['clipboard.copy', 'COPYCLIP', 'menu.clipboard.copy'],
    ['clipboard.copybase', 'COPYBASE', 'menu.clipboard.copybase'],
    ['clipboard.paste', 'PASTECLIP', 'menu.clipboard.paste'],
    ['clipboard.pasteorig', 'PASTEORIG', 'menu.clipboard.pasteorig'],
    ['clipboard.pasteblock', 'PASTEBLOCK', 'menu.clipboard.pasteblock'],
  ]);
  // The chrome holds no clipboard-length fact, so the row is enabled whenever no
  // tool is capturing — the same gate `PASTECLIP`/`PASTEORIG` use — and the host
  // refuses an empty clipboard. A capturing tool disables all three.
  const enabledFor = tool => Object.fromEntries(
    buildContextMenu({ ...ready, tool }).sections
      .find(candidate => candidate.id === 'clipboard').items
      .map(item => [item.id, item.enabled]),
  );
  assert.equal(enabledFor(null)['clipboard.pasteblock'], true);
  assert.equal(enabledFor({ key: 'LINE', step: 1 })['clipboard.pasteblock'], false);
  // Nothing is hidden: the disabled row is still present.
  assert.equal(menuItems(buildContextMenu({ ...ready, tool: { key: 'LINE', step: 1 } }))
    .some(item => item.id === 'clipboard.pasteblock'), true);
});

test('Ctrl+Shift+V maps to PASTEBLOCK through the one admission path', () => {
  // Case-folded like every other letter chord (caps lock included).
  assert.equal(acceleratorCommand(ctrl('v', { shiftKey: true })), 'PASTEBLOCK');
  assert.equal(acceleratorCommand(ctrl('V', { shiftKey: true })), 'PASTEBLOCK');
  // The plain chord keeps its own command: a block paste is not a plain paste,
  // so there is no silent fallback in either direction.
  assert.equal(acceleratorCommand(ctrl('v')), 'PASTECLIP');
  assert.notEqual(acceleratorCommand(ctrl('v', { shiftKey: true })), acceleratorCommand(ctrl('v')));
  // Dispatch is the shell's admission: admitted → the name, refused → nothing (the
  // press is left to the webview untouched).
  const admitted = command => admitCommandText(command, { status: loaded }).kind === 'admitted';
  assert.equal(acceleratorDispatch(ctrl('v', { shiftKey: true }), {}, admitted), 'PASTEBLOCK');
  assert.equal(acceleratorDispatch(ctrl('v', { shiftKey: true }), {}, () => false), null);
  // A live tool, an open dialog and a focused text entry all decline before the
  // chord is looked up, so nothing runs against a capture the user is making.
  assert.equal(acceleratorDispatch(ctrl('v', { shiftKey: true }), { toolActive: true }, admitted), null);
  assert.equal(acceleratorDispatch(ctrl('v', { shiftKey: true }), { popupOpen: true }, admitted), null);
  assert.equal(acceleratorDispatch(ctrl('v', { shiftKey: true }), { typing: true }, admitted), null);
  // With no vocabulary the router does not know the name, so the chord is silent:
  // no local acceptance is invented for it.
  setCommandCatalog([]);
  assert.equal(admitCommandText('PASTEBLOCK', { status: loaded }).kind, 'unsupported');
  assert.equal(acceleratorDispatch(ctrl('v', { shiftKey: true }), {}, command => false), null);
  setCommandCatalog(HOST_NAMES);
});

test('the capture is one insertion point, then Enter; there is no text step or parameter', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS['draw.pasteblock'], ['tool.prompt.pasteblock.insertion']);
  assert.equal(TOOL_STEP_PROMPTS['draw.pasteblock'].length, 1, 'the list must stay at one entry');
  assert.equal(toolStepPromptKey({ key: 'draw.pasteblock', points: 0 }), 'tool.prompt.pasteblock.insertion');
  // The complement is complete, so the chip turns into the generic confirm copy —
  // never the dimension sentence, and never a claim that the block exists yet.
  assert.equal(TOOL_READY_PROMPTS['draw.pasteblock'], 'tool.prompt.confirm');
  assert.equal(toolStepPromptKey({ key: 'draw.pasteblock', points: 1 }), 'tool.prompt.confirm');
  assert.notEqual(toolStepPromptKey({ key: 'draw.pasteblock', points: 1 }), 'tool.prompt.dim.ready');
  // A second count cannot invent a further step.
  assert.equal(toolStepPromptKey({ key: 'draw.pasteblock', points: 4 }), 'tool.prompt.confirm');
  // It is a pick, not a pre-selection tool: a preselection entry would hide the
  // insertion point the command asks for.
  assert.equal(PRESELECT_PROMPTS['draw.pasteblock'], undefined);
  // The active-tool chip names the command through the shared derivation.
  assert.equal(toolLabelKey('draw.pasteblock'), 'tool.draw.pasteblock');
  // A tool the host never reports as a key still names nothing.
  assert.equal(toolStepPromptKey({ key: 'draw.pasteblocker', points: 0 }), null);
});

test('both catalogs carry the same four keys with block-specific, honest copy', () => {
  const keys = [
    'draw.kind.pasteblock',
    'menu.clipboard.pasteblock',
    'tool.draw.pasteblock',
    'tool.prompt.pasteblock.insertion',
  ];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `${key} is empty in zh-CN`);
    assert.ok(en[key].trim().length > 0, `${key} is empty in en`);
  }
  // The command name, in the two places the user reads a name.
  assert.equal(en['draw.kind.pasteblock'], 'Paste as block');
  assert.equal(zh['draw.kind.pasteblock'], '粘贴为块');
  assert.equal(en['tool.draw.pasteblock'], 'Paste as block');
  assert.equal(zh['tool.draw.pasteblock'], '粘贴为块');
  // The prompt names the one pick, and says it is the pasted **block**'s insertion
  // point — not the plain paste's, which is relative to the clipboard's base point.
  assert.equal(en['tool.prompt.pasteblock.insertion'], 'Specify the insertion point for the pasted block');
  assert.equal(zh['tool.prompt.pasteblock.insertion'], '指定粘贴块的插入点');
  assert.match(en['tool.prompt.pasteblock.insertion'], /insertion point/i);
  assert.match(en['tool.prompt.pasteblock.insertion'], /block/i);
  assert.match(zh['tool.prompt.pasteblock.insertion'], /插入点/);
  assert.match(zh['tool.prompt.pasteblock.insertion'], /粘贴块/);
  // The plain paste's sentence is a different claim (base point vs block).
  assert.notEqual(en['tool.prompt.pasteblock.insertion'], en['tool.prompt.pasteclip.insertion']);
  assert.notEqual(zh['tool.prompt.pasteblock.insertion'], zh['tool.prompt.pasteclip.insertion']);
  // The menu row is the same "paste as block" choice as the ribbon/tool name, and
  // is not the plain paste.
  assert.equal(en['menu.clipboard.pasteblock'], 'Paste as block');
  assert.equal(zh['menu.clipboard.pasteblock'], '粘贴为块');
  assert.notEqual(en['menu.clipboard.pasteblock'], en['menu.clipboard.paste']);
  assert.notEqual(zh['menu.clipboard.pasteblock'], zh['menu.clipboard.paste']);
});
