// Contract test for the 3D view affordances: the dedicated typed
// `cad_standard_view` channel, the View tab's standard-view items and the honest
// disable states (document-scoped toggles, gesture-only orbit).
//
// The point of this suite is the distinction the product makes: `{accepted:true}`
// is **queued**, never applied, and a click can never fabricate a camera move.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  COMMAND_CATALOG,
  DOCUMENT_SCOPED_COMMANDS,
  isSupportedCommand,
  NO_DOCUMENT_KEY,
  setCommandCatalog,
} from '../src/commands.mjs';
import { createTranslator } from '../src/i18n.mjs';
import { RIBBON_TABS } from '../src/ribbon-model.mjs';
import {
  requestStandardView,
  STANDARD_VIEWS,
  standardViewAvailable,
  standardViewFailure,
  standardViewName,
  standardViewUnavailableReasonKey,
  submitStandardView,
} from '../src/view3d.mjs';
import { readCatalog, readModule } from './helpers.mjs';

// Only the names this suite exercises. `ORBIT` is here because the router does
// know it — the chrome still refuses a click, which is the point of the gesture
// list, not a missing vocabulary entry.
setCommandCatalog(['SWITCH2D3D', 'PROJECTION', 'ORBIT', 'FIT', 'PAN', 'ZOOMIN', 'ZOOMOUT']);

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');
const loaded = { documentLabel: 'fixture.dxf', entityCount: 12, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false };
const EXPECTED_VIEWS = ['top', 'bottom', 'front', 'back', 'left', 'right', 'isometric'];

function viewTab() {
  const tab = RIBBON_TABS.find(candidate => candidate.id === 'view');
  assert.ok(tab, 'the View tab must exist');
  return tab;
}

test('the View tab exposes the 3D panel, the orbit hint and all seven standard views', async () => {
  const tab = viewTab();
  const mode = tab.panels.find(panel => panel.id === 'view3d');
  assert.ok(mode, 'the View tab must carry the 3D panel');
  // The 3D affordances lead the tab: mode toggle, projection toggle, orbit hint.
  assert.deepEqual(mode.items.map(item => item.id), ['SWITCH2D3D', 'PROJECTION', 'ORBIT']);
  const toggle = mode.items.find(item => item.id === 'SWITCH2D3D');
  assert.equal(toggle.command, 'SWITCH2D3D');
  assert.equal(toggle.labelKey, 'ribbon.command.view_switch2d3d');
  assert.equal(toggle.large, true);
  assert.equal(toggle.standardView, undefined, 'the 2D/3D toggle carries no payload');
  const projection = mode.items.find(item => item.id === 'PROJECTION');
  assert.equal(projection.command, 'PROJECTION');
  assert.equal(projection.labelKey, 'view.projection');
  assert.equal(projection.standardView, undefined, 'PROJECTION is payload-free');
  assert.equal(typeof zh[mode.titleKey], 'string', mode.titleKey);
  assert.equal(typeof en[mode.titleKey], 'string', mode.titleKey);

  const standard = tab.panels.find(panel => panel.id === 'standard');
  assert.ok(standard, 'the View tab must carry the standard-view panel');
  assert.deepEqual(standard.items.map(item => item.standardView), EXPECTED_VIEWS);
  // The panel follows the shared data table, so a renamed host key cannot drift.
  assert.deepEqual(standard.items, STANDARD_VIEWS.map(entry => ({ ...entry })));
  const icons = await readModule('icons.jsx');
  for (const item of standard.items) {
    // A payload-carrying view is never also a `cad_command` name: that funnel
    // refuses a payload variant, so offering it as one would be a broken button.
    assert.equal(item.command, undefined, `${item.id} must not carry a command`);
    assert.equal(item.action, undefined, `${item.id} is not a shell action`);
    assert.equal(item.large, undefined, `${item.id} stays a small view button`);
    // Both catalogs word every label, and every glyph really exists (a missing
    // name would silently fall back to the `dot` icon).
    for (const key of [item.labelKey]) {
      assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
      assert.equal(typeof en[key], 'string', `en missing ${key}`);
    }
    assert.ok(icons.includes(`${item.icon}:`), `icons.jsx has no ${item.icon} glyph`);
  }
  assert.equal(typeof zh[standard.titleKey], 'string', standard.titleKey);
  assert.equal(typeof en[standard.titleKey], 'string', standard.titleKey);
  // The seven labels are distinct views, never one word repeated.
  assert.equal(new Set(standard.items.map(item => item.labelKey)).size, 7);
});

test('a standard-view click sends exactly one cad_standard_view with its own name', async () => {
  const standard = viewTab().panels.find(panel => panel.id === 'standard');
  for (const item of standard.items) {
    const calls = [];
    const reported = [];
    const result = await submitStandardView({
      invoke: async (name, args) => { calls.push([name, args]); return { accepted: true }; },
      view: item.standardView,
      t,
      report: message => reported.push(message),
    });
    assert.equal(result.kind, 'accepted', item.standardView);
    assert.equal(result.view, item.standardView);
    // One IPC, the typed channel, and the host's own machine key as the payload.
    assert.deepEqual(calls, [['cad_standard_view', { view: item.standardView }]], item.standardView);
    // Admission is not completion: the ribbon reports nothing on a queued view.
    assert.deepEqual(reported, [], `${item.standardView} must not be reported as applied`);
  }
  // The payload name itself is never offered to the name-only command funnel.
  assert.equal(COMMAND_CATALOG.some(command => command.id === 'STANDARDVIEW'), false);
});

test('a refused or throwing cad_standard_view is surfaced, never swallowed', async () => {
  // The host answers, but not with an admission.
  const calls = [];
  const reported = [];
  const refused = await submitStandardView({
    invoke: async (name, args) => { calls.push(name); return { accepted: false }; },
    view: 'isometric',
    t,
    report: message => reported.push(message),
  });
  assert.equal(refused.kind, 'refused');
  assert.equal(calls.length, 1);
  assert.equal(reported.length, 1, 'a non-admission must reach the banner');
  assert.ok(reported[0].includes('CAD_STANDARD_VIEW_NOT_ACCEPTED'), reported[0]);
  assert.notEqual(reported[0], 'CAD_STANDARD_VIEW_NOT_ACCEPTED', 'the banner shows catalog copy, not the bare code');

  // A rejected invoke surfaces the host's own reason verbatim.
  reported.length = 0;
  const thrown = await submitStandardView({
    invoke: async () => { throw Error('view.standard_unknown: nope'); },
    view: 'top',
    t,
    report: message => reported.push(message),
  });
  assert.equal(thrown.kind, 'refused');
  assert.equal(reported.length, 1);
  assert.ok(reported[0].includes('view.standard_unknown'), reported[0]);

  // A name outside the seven never reaches IPC at all.
  let invoked = false;
  const unknown = await requestStandardView(async () => { invoked = true; return { accepted: true }; }, 'sideways');
  assert.equal(unknown.kind, 'refused');
  assert.equal(unknown.error, 'CAD_STANDARD_VIEW_UNKNOWN');
  assert.equal(invoked, false, 'an unknown view must not be sent to the host');
  assert.ok(standardViewFailure(unknown, t).includes('CAD_STANDARD_VIEW_UNKNOWN'));
  // The admitted case yields no line, so the banner cannot show a false failure.
  assert.equal(standardViewFailure({ kind: 'accepted', view: 'top' }, t), '');
  assert.equal(standardViewFailure(null, t).includes('CAD_STANDARD_VIEW_'), true, 'a missing result is a named refusal');
});

test('the wire name is matched like the host does: trimmed and case-insensitive', () => {
  assert.equal(standardViewName(' Isometric '), 'isometric');
  assert.equal(standardViewName('TOP'), 'top');
  assert.equal(standardViewName('sideways'), null);
  assert.equal(standardViewName(''), null);
  assert.equal(standardViewName(null), null);
  assert.deepEqual([...EXPECTED_VIEWS], STANDARD_VIEWS.map(entry => entry.standardView));
});

test('standard views and the 3D toggles need an open drawing, with a real reason', () => {
  // No status yet, an import in flight, and the start page all stay disabled.
  assert.equal(standardViewAvailable({ status: null, documentOpen: false }), false);
  assert.equal(standardViewAvailable({ status: { ...loaded, loading: true }, documentOpen: true }), false);
  assert.equal(standardViewAvailable({ status: empty, documentOpen: false }), false);
  assert.equal(standardViewAvailable({ status: loaded, documentOpen: true }), true);
  // The reason is the catalog's existing "no drawing is open" sentence, in both
  // languages, and it is not the generic unsupported line.
  assert.equal(standardViewUnavailableReasonKey({ status: empty, documentOpen: false }), NO_DOCUMENT_KEY);
  assert.notEqual(NO_DOCUMENT_KEY, 'command.unsupported');
  assert.equal(typeof zh[NO_DOCUMENT_KEY], 'string');
  assert.equal(typeof en[NO_DOCUMENT_KEY], 'string');
  // Before the host has answered, the honest answer is the generic key.
  assert.equal(standardViewUnavailableReasonKey({ status: null, documentOpen: false }), 'command.unsupported');
  assert.equal(standardViewUnavailableReasonKey({ status: loaded, documentOpen: true }), 'command.unsupported');

  // The two payload-free toggles are router commands, and they are gated on a
  // document the same way (disabled, not "unsupported", on the start page).
  assert.deepEqual([...DOCUMENT_SCOPED_COMMANDS], ['SWITCH2D3D', 'PROJECTION']);
  for (const id of DOCUMENT_SCOPED_COMMANDS) {
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    assert.equal(admitCommandText(id, { status: empty }).kind, 'unavailable', id);
    assert.equal(admitCommandText(id, { status: null }).kind, 'unavailable', id);
  }
});

test('the orbit affordance stays visible, gesture-disabled, and emits no command', async () => {
  const orbit = viewTab().panels.find(panel => panel.id === 'view3d').items
    .find(item => item.id === 'ORBIT');
  assert.ok(orbit, 'the orbit hint must stay visible in the 3D panel');
  assert.equal(orbit.command, 'ORBIT');
  assert.equal(orbit.labelKey, 'view.orbit_label');
  assert.equal(orbit.standardView, undefined, 'orbit is a gesture, not a view payload');
  assert.equal(orbit.action, undefined, 'orbit is never a local shell action');
  // The router knows ORBIT, yet a click cannot supply the drag payload.
  assert.equal(isSupportedCommand('ORBIT'), false);
  assert.equal(admitCommandText('ORBIT', { status: loaded }).kind, 'unsupported');
  // Both catalogs carry the gesture reason the disabled button shows.
  assert.equal(typeof zh['view.orbit.gesture'], 'string');
  assert.equal(typeof en['view.orbit.gesture'], 'string');
  // A disabled ribbon button drops its click handler: no IPC can be emitted.
  const primitives = await readModule('components/primitives.jsx');
  assert.ok(
    primitives.includes('onClick={disabled ? undefined : onClick}'),
    'a disabled ribbon button must not carry a click handler',
  );
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('unavailableReasonKey(item.command, { status: shell.status, pending: shell.pending })'),
    'the gesture hint comes from the shared reason key',
  );
});

test('the renderer routes a standard view through the typed IPC, never through runCommand', async () => {
  const ribbon = await readModule('components/Ribbon.jsx');
  const typed = ribbon.indexOf('if (item.standardView) return shell.runStandardView(item.standardView);');
  const command = ribbon.indexOf('if (item.command) return shell.runCommand(item.command);');
  assert.ok(typed !== -1, 'ribbon items must dispatch their standardView token');
  assert.ok(command !== -1, 'ribbon items keep dispatching their command token');
  assert.ok(typed < command, 'the payload branch must precede the name-only command branch');
  const app = await readModule('App.jsx');
  assert.ok(
    app.includes('submitStandardView({ invoke, view, t, report: setError })'),
    'the shell must surface a refused view through the error banner',
  );
});
