// Clipboard-with-base-point contracts (2026-10-27): the capture tools, the three
// selectors and the copy that names both pastes.
//
// The prompt tables are exercised as values (they are pure data), and the catalog
// is read through the same admission path the ribbon and the command line use.
// Nothing here mounts the UI or checks a pixel: the router that will accept these
// names is the Rust lane's, and this mirror (`HOST_NAMES`) only stands in for it
// so the chrome's own rules are testable today.
import test from 'node:test';
import assert from 'node:assert/strict';
import { TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad/cad.ts';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
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

/** The three new selectors and the label key each catalog row must carry. */
const SELECTORS = [
  { id: 'COPYBASE', labelKey: 'draw.kind.copybase' },
  { id: 'CUTBASE', labelKey: 'draw.kind.cutbase' },
  { id: 'PASTEORIG', labelKey: 'draw.kind.pasteorig' },
];

test('COPYBASE captures exactly one pick, and that pick is the base point', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS['draw.copybase'], ['tool.prompt.copybase.base']);
  assert.equal(toolStepPromptKey({ key: 'draw.copybase', points: 0 }), 'tool.prompt.copybase.base');
  // A second count cannot invent a step: the capture is one point, so the ready
  // copy takes over — and it must never be the dimension sentence.
  assert.equal(toolStepPromptKey({ key: 'draw.copybase', points: 1 }), 'tool.prompt.confirm');
  assert.equal(toolStepPromptKey({ key: 'draw.copybase', points: 1 }), TOOL_READY_PROMPTS['draw.copybase']);
  assert.notEqual(toolStepPromptKey({ key: 'draw.copybase', points: 1 }), 'tool.prompt.dim.ready');
  // The chip names the tool the host reports (`draw.copybase` -> `tool.draw.copybase`).
  assert.equal(toolLabelKey('draw.copybase'), 'tool.draw.copybase');
  // CUTBASE is the cutting form of the same capture and the host reports it under
  // its own machine key (`cutbase`), so it gets the same one-pick prompt rather
  // than a bare "0 points" chip.
  assert.deepEqual(TOOL_STEP_PROMPTS['draw.cutbase'], ['tool.prompt.copybase.base']);
  assert.equal(toolStepPromptKey({ key: 'draw.cutbase', points: 0 }), 'tool.prompt.copybase.base');
  assert.equal(toolStepPromptKey({ key: 'draw.cutbase', points: 1 }), 'tool.prompt.confirm');
  assert.equal(toolLabelKey('draw.cutbase'), 'tool.draw.cutbase');
});

test('PASTECLIP asks for an insertion point; PASTEORIG takes no pick at all', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS['draw.pasteclip'], ['tool.prompt.pasteclip.insertion']);
  assert.equal(toolStepPromptKey({ key: 'draw.pasteclip', points: 0 }), 'tool.prompt.pasteclip.insertion');
  assert.equal(toolStepPromptKey({ key: 'draw.pasteclip', points: 1 }), 'tool.prompt.confirm');
  assert.notEqual(toolStepPromptKey({ key: 'draw.pasteclip', points: 1 }), 'tool.prompt.dim.ready');
  assert.equal(toolLabelKey('draw.pasteclip'), 'tool.draw.pasteclip');
  // PASTEORIG is the explicit "original coordinates" path: no capture, no step, and
  // therefore no prompt to name — a caption here would promise a pick it never takes.
  assert.equal(TOOL_STEP_PROMPTS['draw.pasteorig'], undefined);
  assert.equal(toolStepPromptKey({ key: 'draw.pasteorig', points: 0 }), null);
});

test('the paste prompt says insertion point and the copy prompt says base point', () => {
  const keys = ['tool.prompt.copybase.base', 'tool.prompt.pasteclip.insertion'];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `${key} is empty in zh-CN`);
    assert.ok(en[key].trim().length > 0, `${key} is empty in en`);
  }
  // "With base point" means the paste that follows is placed relative to that
  // point; the copy prompt has to say so, and the paste prompt names a point.
  assert.match(en['tool.prompt.copybase.base'], /base point/i);
  assert.match(en['tool.prompt.copybase.base'], /relative/i);
  assert.match(zh['tool.prompt.copybase.base'], /基点/);
  assert.match(zh['tool.prompt.copybase.base'], /相对/);
  assert.match(en['tool.prompt.pasteclip.insertion'], /insertion point/i);
  assert.match(zh['tool.prompt.pasteclip.insertion'], /插入点/);
  // The two prompts are different sentences: an insertion point is not a base point.
  assert.notEqual(en['tool.prompt.copybase.base'], en['tool.prompt.pasteclip.insertion']);
  assert.notEqual(zh['tool.prompt.copybase.base'], zh['tool.prompt.pasteclip.insertion']);
});

test('the catalog carries the three clipboard selectors under a shared glyph family', () => {
  for (const { id, labelKey } of SELECTORS) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, labelKey);
    assert.equal(entry.group, 'modify');
    // The icon set has no cut/paste mark; the clipboard family reuses `copy`, the
    // same way JOIN reuses `polyline` (no new glyph is invented here).
    assert.equal(entry.icon, 'copy', id);
    // The catalog is the display/suggestion half; admission is the host vocabulary.
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  }
  // PASTEBLOCK is unimplemented: it must not appear as a catalogued affordance.
  assert.equal(COMMAND_CATALOG.some(command => command.id === 'PASTEBLOCK'), false);
  assert.equal(HOST_NAMES.includes('PASTEBLOCK'), false, 'the vocabulary mirror stays honest');
});

test('every new key exists in both catalogs, and both pastes stay distinguishable', () => {
  const keys = [
    'menu.clipboard.cutbase',
    'menu.clipboard.copybase',
    'menu.clipboard.pasteorig',
    'draw.kind.copybase',
    'draw.kind.cutbase',
    'draw.kind.pasteorig',
    'tool.draw.copybase',
    'tool.draw.cutbase',
    'tool.draw.pasteclip',
    'tool.prompt.copybase.base',
    'tool.prompt.pasteclip.insertion',
  ];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `${key} is empty in zh-CN`);
    assert.ok(en[key].trim().length > 0, `${key} is empty in en`);
  }
  // The menu, the catalog row and the tool chip may share a sentence (they name
  // the same command), but the paste and the paste-at-original may not.
  assert.match(en['menu.clipboard.copybase'], /base point/i);
  assert.match(en['menu.clipboard.cutbase'], /base point/i);
  assert.match(en['draw.kind.copybase'], /base point/i);
  assert.match(zh['menu.clipboard.copybase'], /基点/);
  assert.match(zh['menu.clipboard.cutbase'], /基点/);
  assert.notEqual(en['menu.clipboard.paste'], en['menu.clipboard.pasteorig']);
  assert.notEqual(zh['menu.clipboard.paste'], zh['menu.clipboard.pasteorig']);
  assert.match(en['menu.clipboard.pasteorig'], /original coordinates/i);
  assert.match(zh['menu.clipboard.pasteorig'], /原坐标/);
  // The tool chips exist so a live capture never renders a raw key.
  assert.equal(toolLabelKey('draw.copybase'), 'tool.draw.copybase');
  assert.equal(toolLabelKey('draw.cutbase'), 'tool.draw.cutbase');
  assert.equal(toolLabelKey('draw.pasteclip'), 'tool.draw.pasteclip');
  for (const key of ['tool.draw.copybase', 'tool.draw.cutbase', 'tool.draw.pasteclip']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
  }
});
