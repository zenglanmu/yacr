// `cad_set_view` frontend contracts: the fitted BIM presets, admission rules,
// the typed request and the refusal path. Pure Node tests (no DOM, no React).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import {
  SET_VIEW_COMMAND,
  SET_VIEW_PRESETS,
  isSetViewPreset,
  requestSetView,
  setViewAvailable,
  setViewNeedsThreeD,
  setViewUnavailableReasonKey,
  submitSetView,
} from '../src/cad/set-view.ts';
import { BIM_RIBBON_TABS } from '../src/domains/bim/ribbon.ts';

test('the six fitted presets are the wire vocabulary in order', () => {
  assert.deepEqual([...SET_VIEW_PRESETS], ['overview', 'plan', 'front', 'back', 'left', 'right']);
  assert.equal(SET_VIEW_COMMAND, 'cad_set_view');
  for (const preset of SET_VIEW_PRESETS) assert.equal(isSetViewPreset(preset), true);
  assert.equal(isSetViewPreset('isometric'), false);
  assert.equal(setViewNeedsThreeD('plan'), false);
  for (const preset of ['overview', 'front', 'back', 'left', 'right']) {
    assert.equal(setViewNeedsThreeD(preset), true);
  }
});

test('the BIM ribbon exposes one viewPreset item per preset', () => {
  const presets = BIM_RIBBON_TABS
    .flatMap(tab => tab.panels)
    .flatMap(panel => panel.items)
    .filter(item => typeof item.viewPreset === 'string')
    .map(item => item.viewPreset);
  assert.deepEqual(presets, [...SET_VIEW_PRESETS]);
});

test('admission needs a loaded document and refuses 3D on a 2D-only carrier', () => {
  const live = { status: { loading: false, hostProfile: { orbit: null } }, documentOpen: true };
  for (const preset of SET_VIEW_PRESETS) assert.equal(setViewAvailable(live, preset), true);

  assert.equal(setViewAvailable({ status: null, documentOpen: true }, 'overview'), false);
  assert.equal(
    setViewAvailable({ status: { loading: true }, documentOpen: true }, 'overview'),
    false,
  );
  assert.equal(setViewAvailable(live, 'plan'), true);

  const canvas = { status: { loading: false, hostProfile: { orbit: false } }, documentOpen: true };
  assert.equal(setViewAvailable(canvas, 'overview'), false);
  assert.equal(setViewAvailable(canvas, 'front'), false);
  assert.equal(setViewAvailable(canvas, 'plan'), true, 'the 2D plan is a real 2D observation');
  assert.equal(setViewUnavailableReasonKey(canvas, 'overview'), 'view.orbit_unavailable');
  assert.equal(
    setViewUnavailableReasonKey({ status: live.status, documentOpen: false }, 'plan'),
    'plot.no_document',
  );
});

test('requestSetView sends exactly one typed placement and returns admission only', async () => {
  const calls = [];
  const accepted = await requestSetView(async (...args) => {
    calls.push(args);
    return { accepted: true };
  }, 'overview');
  assert.deepEqual(accepted, { kind: 'accepted', preset: 'overview' });
  assert.deepEqual(calls, [['cad_set_view', { preset: 'overview' }]]);

  const refused = await requestSetView(async () => ({ accepted: false }), 'plan');
  assert.equal(refused.kind, 'refused');
  assert.equal(refused.kind === 'refused' ? refused.error : '', 'Error: CAD_SET_VIEW_NOT_ACCEPTED');

  const rejected = await requestSetView(async () => {
    throw new Error('view.set.empty: the active space has no measurable extent');
  }, 'front');
  assert.equal(
    rejected.kind === 'refused' ? rejected.error : '',
    'Error: view.set.empty: the active space has no measurable extent',
  );
});

test('an unknown preset never reaches IPC', async () => {
  let called = false;
  const result = await requestSetView(async () => {
    called = true;
    return { accepted: true };
  }, 'nope');
  assert.equal(result.kind, 'refused');
  assert.equal(called, false);
  assert.equal(result.kind === 'refused' ? result.error : '', 'CAD_SET_VIEW_UNKNOWN_PRESET');
});

test('submitSetView reports a refusal through the banner and stays silent on admission', async () => {
  const reported = [];
  const ok = await submitSetView({
    invoke: async () => ({ accepted: true }),
    preset: 'plan',
    t: (key, args) => `${key}:${args?.error ?? ''}`,
    report: message => reported.push(message),
  });
  assert.equal(ok.kind, 'accepted');
  assert.deepEqual(reported, []);

  await submitSetView({
    invoke: async () => { throw new Error('view.set.empty'); },
    preset: 'overview',
    t: (key, args) => `${key}:${args?.error ?? ''}`,
    report: message => reported.push(message),
  });
  assert.deepEqual(reported, ['command.failed:Error: view.set.empty']);
});

test('runItem dispatches a viewPreset through the typed runner, not cad_command', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('item.viewPreset'), 'the preset branch exists');
  assert.ok(ribbon.includes('shell.runSetView(item.viewPreset)'), 'it calls the typed runner');
  assert.ok(ribbon.includes('setViewUnavailableReasonKey'), 'the refusal sentence is its own branch');
});
