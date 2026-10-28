// Contracts for the `dom-canvas` drawing identity (ADR 0024 D1/D2, AGENTS.md
// iron rule 8).
//
// Scope, stated honestly: these are **pure-function** and **source-structure**
// tests. There is no DOM mount, no browser run and no pixel evidence here. What
// they pin is (a) the only acceptable source of a drawing identity on that
// carrier, (b) that a `native-below` host is untouched by it, and (c) that the
// chrome never invents content the carrier does not have.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CANVAS_SESSION_LABEL_MAX,
  canvasSessionFromShell,
  canvasSessionKey,
  canvasSessionPayload,
  canvasSessionReport,
  clampCanvasLabel,
  createCanvasSessionReporter,
  palettesOffered,
} from '../src/cad/surface.ts';
import { readAppSource, readCatalog, readModule } from './helpers.ts';
import { offeredItems, offeredPanels, PALETTE_ACTIONS, PALETTE_COMMANDS, RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';

const LABEL = 'synthetic-four-lines.dwg';

/** A host shell that reports the in-DOM carrier. */
function canvasShell(session, extra = {}) {
  return { status: { surfaceMode: 'dom-canvas', canvasSession: session, ...extra } };
}

/** One healthy session status, shaped like the wasm session's JSON. */
function liveStatus(overrides = {}) {
  return {
    ok: true,
    label: LABEL,
    batches: 2,
    draws: 5,
    presented: 4,
    physical: [800, 600],
    ...overrides,
  };
}

test('a relayed session is the drawing identity of a dom-canvas host', () => {
  assert.deepEqual(
    canvasSessionFromShell(canvasShell({ label: LABEL, batches: 2, draws: 5 })),
    { label: LABEL, batches: 2, draws: 5 },
  );
  // The carrier and the relayed field are read from the live shell status only.
  assert.deepEqual(
    canvasSessionFromShell({ status: { surfaceMode: 'dom-canvas', canvasSession: { label: LABEL } } }),
    { label: LABEL, batches: null, draws: null },
  );
});

test('every other carrier keeps the start state: no identity is ever guessed', () => {
  const session = { label: LABEL, batches: 2, draws: 5 };
  const shells = [
    undefined,
    null,
    {},
    { status: null },
    { status: {} },
    // The field itself is not a capability hint: an older host that reports no
    // carrier is `native-below`, and a `native-below` host has no session.
    { status: { canvasSession: session } },
    { status: { surfaceMode: 'native-below', canvasSession: session } },
    // Near misses are not matched loosely (same policy as `surfaceModeFromShell`).
    { status: { surfaceMode: 'DOM-CANVAS', canvasSession: session } },
    { status: { surfaceMode: 'dom-canvas ', canvasSession: session } },
    { status: { surfaceMode: 'dom_canvas', canvasSession: session } },
  ];
  for (const shell of shells) {
    assert.equal(canvasSessionFromShell(shell), null, `shell ${JSON.stringify(shell)} must name no drawing`);
  }
});

test('only an object with a usable label is a session', () => {
  for (const session of [
    null,
    undefined,
    'synthetic-four-lines.dwg',
    7,
    true,
    [],
    [{ label: LABEL }],
    {},
    { label: null },
    { label: 42 },
    { label: '' },
    { label: '   ' },
    // Control characters collapse to spaces, so this is an empty label, not one.
    { label: '\u0000\u0001\u007f' },
    { label: '\n\r\t' },
  ]) {
    assert.equal(
      canvasSessionFromShell(canvasShell(session)),
      null,
      `canvasSession ${JSON.stringify(session)} must not name a drawing`,
    );
  }
});

test('the label is flattened to one bounded line before anyone prints it', () => {
  assert.equal(clampCanvasLabel('plain.dwg'), 'plain.dwg');
  // Line breaks, tabs, control characters and Unicode separators all become one
  // space, and whitespace runs collapse: never a second line in the chrome.
  assert.equal(clampCanvasLabel('a\nb'), 'a b');
  assert.equal(clampCanvasLabel('  a\t\tb  '), 'a b');
  assert.equal(clampCanvasLabel('a\u0000b'), 'a b');
  assert.equal(clampCanvasLabel('a\u007fb'), 'a b');
  assert.equal(clampCanvasLabel('a\u00a0b'), 'a b');
  assert.equal(clampCanvasLabel('a\u2028b'), 'a b');
  assert.equal(clampCanvasLabel('a\r\nb'), 'a b');
  // A value that is not a string is no label at all.
  for (const value of [null, undefined, 7, true, {}, []]) assert.equal(clampCanvasLabel(value), null);
});

test('the label is clamped to the documented maximum in code points', () => {
  assert.equal(CANVAS_SESSION_LABEL_MAX, 120);
  const exact = 'x'.repeat(CANVAS_SESSION_LABEL_MAX);
  assert.equal(clampCanvasLabel(exact), exact);
  const long = clampCanvasLabel('x'.repeat(400));
  assert.equal(long.length, CANVAS_SESSION_LABEL_MAX);
  // Counting code points keeps an astral character whole instead of emitting a
  // lone surrogate half.
  const emoji = clampCanvasLabel('\u{1F600}'.repeat(130));
  assert.equal(Array.from(emoji).length, CANVAS_SESSION_LABEL_MAX);
  assert.ok(Array.from(emoji).every(point => point === '\u{1F600}'));
  // A cut that lands on the single space before the next word drops that space.
  assert.equal(clampCanvasLabel(`${'y'.repeat(119)} tail`).length, 119);
});

test('batches and draws are counts or nothing, never a rounded figure', () => {
  const counts = status => {
    const descriptor = canvasSessionReport(status);
    return descriptor === null ? null : { batches: descriptor.batches, draws: descriptor.draws };
  };
  assert.deepEqual(counts(liveStatus({ batches: 0, draws: 0 })), { batches: 0, draws: 0 });
  for (const bad of [-1, 1.5, '3', NaN, Infinity, null, undefined, true]) {
    assert.deepEqual(counts(liveStatus({ batches: bad, draws: bad })), { batches: null, draws: null }, String(bad));
  }
  assert.deepEqual(counts(liveStatus({ batches: undefined, draws: undefined })), { batches: null, draws: null });
});

test('a report is a healthy session status with its own identity', () => {
  assert.deepEqual(canvasSessionReport(liveStatus()), { label: LABEL, batches: 2, draws: 5 });
  // A failure is reported through the transport's one failure path, so it never
  // also publishes a document identity.
  for (const status of [
    null,
    undefined,
    'json',
    [],
    liveStatus({ ok: false, step: 'render', error: 'boom' }),
    liveStatus({ ok: 'true' }),
    liveStatus({ ok: undefined }),
    liveStatus({ label: '' }),
    liveStatus({ label: null }),
  ]) {
    assert.equal(canvasSessionReport(status), null, `status ${JSON.stringify(status)} must not publish`);
  }
});

test('identical consecutive reports reach the host once', () => {
  const sent = [];
  const reporter = createCanvasSessionReporter(report => sent.push(report));
  assert.equal(reporter.report(liveStatus()), true);
  assert.deepEqual(sent, [{ label: LABEL, batches: 2, draws: 5 }]);
  // The same payload again (a pan that changed nothing) is one report.
  assert.equal(reporter.report(liveStatus()), false);
  assert.equal(reporter.report(liveStatus({ presented: 99 })), false);
  assert.equal(sent.length, 1);
  // A changed count is a new report...
  assert.equal(reporter.report(liveStatus({ draws: 6 })), true);
  assert.equal(sent.length, 2);
  // ...and so is a different drawing.
  assert.equal(reporter.report(liveStatus({ label: 'other.dwg', draws: 6 })), true);
  assert.equal(sent.length, 3);
  // An invalid status sends nothing and does not reset the dedupe window: the
  // last real report still stands.
  assert.equal(reporter.report(liveStatus({ ok: false })), false);
  assert.equal(reporter.report(null), false);
  assert.equal(reporter.report(liveStatus({ label: 'other.dwg', draws: 6 })), false);
  assert.equal(sent.length, 3);
  assert.equal(canvasSessionKey({ label: 'a', batches: null, draws: null }), 'a\u0000null\u0000null');
  assert.throws(() => createCanvasSessionReporter(null), TypeError);
});

test('the wire body never carries a fabricated or null count', () => {
  // The host command takes `{label, batches, draws}` and defaults a missing key;
  // `null` would fail its integer deserialization, and 0 would be a fabricated
  // figure. A count that was not reported is therefore simply absent.
  assert.deepEqual(canvasSessionPayload({ label: LABEL, batches: 2, draws: 5 }), { label: LABEL, batches: 2, draws: 5 });
  assert.deepEqual(canvasSessionPayload({ label: LABEL, batches: 0, draws: 0 }), { label: LABEL, batches: 0, draws: 0 });
  assert.deepEqual(canvasSessionPayload({ label: LABEL, batches: null, draws: null }), { label: LABEL });
  assert.deepEqual(canvasSessionPayload({ label: LABEL, batches: 3, draws: null }), { label: LABEL, batches: 3 });
  // And the body a real status produces is exactly what the host reads.
  assert.deepEqual(canvasSessionPayload(canvasSessionReport(liveStatus())), { label: LABEL, batches: 2, draws: 5 });
});

test('the app publishes the session report from the dom-canvas effect only', async () => {
  const app = await readModule('App.tsx');
  // Exactly one IPC call site in the whole package: no second transport, no
  // second status channel, and no region component talking to this command.
  const all = await readAppSource();
  assert.equal(all.split("invoke('canvas_session_status'").length - 1, 1);
  assert.equal(app.split('createCanvasTransport(').length - 1, 1);
  assert.ok(app.includes('createCanvasSessionReporter('));
  assert.ok(app.includes('reporter.report(next);'));
  // The existing ref stays the one place the raw status is kept.
  assert.ok(app.includes('canvasStatusRef.current = next;'));
  // The `dom-canvas` guard precedes the report, and the effect returns before it
  // otherwise: a `native-below` mount never reaches the publish path.
  const guard = app.indexOf('if (surfaceMode !== SURFACE_MODE_DOM_CANVAS || !surfaceNode) return undefined;');
  assert.ok(guard > 0, 'the dom-canvas effect guard must exist');
  assert.ok(guard < app.indexOf('canvas_session_status'), 'the guard must precede the report');
  // The typed payload object is still the only body: TS satisfies the loose IPC
  // argument type by spreading it (or passing it directly).
  assert.match(app, /invoke\('canvas_session_status', (?:\{ \.\.\.canvasSessionPayload\(report\) \}|canvasSessionPayload\(report\))\)/);
});

test('a canvas session replaces the start panel in the existing label slot', async () => {
  const app = await readModule('App.tsx');
  // One document slot, two honest sources: the host's label or the session's.
  assert.ok(app.includes('const hostDocumentLabel = status?.documentLabel ?? null;'));
  assert.ok(app.includes('const documentLabel = hostDocumentLabel ?? canvasSession?.label ?? null;'));
  assert.ok(app.includes('const canvasSession = useMemo(() => canvasSessionFromShell({ status }), [status]);'));
  // Layout and capability stay two facts: the session makes a drawing *visible*,
  // never a host document that could admit document-scoped commands.
  assert.ok(app.includes('const documentOpen = hostDocumentLabel !== null;'));
  assert.ok(app.includes('const documentVisible = documentOpen || canvasSession !== null;'));

  const workspace = await readModule('components/Workspace.tsx');
  // The start panel is replaced by the CAD partition from the layout fact, not
  // from a carrier check in the component.
  assert.ok(workspace.includes('const showStart = !shell.documentVisible;'));

  // The label lands in the slot the open drawing already uses: no new node, no
  // new string, no second tab.
  const doctabs = await readModule('components/DocTabs.tsx');
  assert.ok(doctabs.includes('const label = shell.documentLabel;'));
  assert.ok(doctabs.includes('<span className="doc-tab__label">{label}</span>'));
  // The status bar keeps its existing state words for the same fact.
  const status = await readModule('components/StatusBar.tsx');
  assert.ok(status.includes("shell.documentVisible ? t('status.ready') : t('plot.no_document')"));
});

test('a visible canvas drawing never becomes a host document for admission', async () => {
  const app = await readModule('App.tsx');
  // The session must not feed the fact that gates document-scoped commands,
  // standard views or the host's palettes.
  assert.ok(!/const documentOpen = [^;]*canvasSession/.test(app));
  // Command admission keeps reading the host's own label.
  const commands = await readModule('commands/commands.ts');
  assert.ok(commands.includes('DOCUMENT_SCOPED_COMMANDS.includes(id) && status.documentLabel === null'));
  // The ribbon's no-document gate keeps reading the host fact too.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('!shell.documentOpen'));
  // And `documentVisible` is only consumed by layout / the status-bar state word.
  const consumers = [await readModule('components/Workspace.tsx'), await readModule('components/StatusBar.tsx')];
  for (const source of consumers) assert.ok(source.includes('shell.documentVisible'));
});

test('the palettes are what the carrier reports, not what it is called', async () => {
  const app = await readModule('App.tsx');
  // The flag is the carrier's own capability claim, read once through the helper -
  // never derived from the carrier's name (`dom-canvas` used to mean "no palettes",
  // which stopped being true when the page session became the real desktop session).
  assert.ok(app.includes('const palettesAvailable = palettesOffered(status);'));
  assert.ok(!app.includes('palettesAvailable = surfaceMode'));
  assert.equal(palettesOffered({ palettesAvailable: true }), true);
  assert.equal(palettesOffered({ palettesAvailable: false }), false);
  // An absent flag is today's behaviour: available. Only an explicit `false` hides
  // a panel, so an older host keeps every palette it always had.
  assert.equal(palettesOffered({}), true);
  assert.equal(palettesOffered(null), true);
  assert.equal(palettesOffered({ status: {} }), true);
  assert.equal(palettesOffered({ palettesAvailable: 'false' }), true);

  const workspace = await readModule('components/Workspace.tsx');
  // The carrier rule is one helper (`palettesOffered` in `surface.mjs`), asked by
  // the panels, the status bar and the nav rail alike.
  assert.ok(workspace.includes('const palettesAvailable = palettesOffered(shell);'));
  assert.ok(workspace.includes("import { palettesOffered, SURFACE_MODE_DOM_CANVAS, surfaceModeFromShell } from '../cad/surface.ts';"));
  assert.ok(workspace.includes('const showProperties = palettesAvailable && shell.propertiesOpen && !showStart;'));
  assert.ok(workspace.includes('const showDiagnostics = palettesAvailable && Boolean(shell.diagnosticsOpen);'));
  // Both panels render from the gated flags, so no empty panel is possible.
  assert.ok(workspace.includes('{showProperties ? <PropertiesPalette /> : null}'));
  assert.ok(workspace.includes('{showDiagnostics ? <DiagnosticsPalette /> : null}'));
  assert.ok(!workspace.includes('{shell.diagnosticsOpen ?'));

  const status = await readModule('components/StatusBar.tsx');
  const gate = status.indexOf('{palettesOffered(shell) ? (');
  assert.ok(gate > 0, 'the status bar must gate the palette toggles');
  assert.ok(!status.includes('shell.palettesAvailable !== false'), 'the rule is not re-spelled inline');
  const block = status.slice(gate, status.indexOf(') : null}', gate));
  assert.ok(block.includes("t('properties.panel')"), 'the properties toggle lives behind the gate');
  assert.ok(block.includes("t('diagnostics.title')"), 'the diagnostics toggle lives behind the gate');
  // The clean-screen / command-line toggles stay available on every carrier.
  assert.ok(status.slice(0, gate).includes("t('status.command_line')"));
  assert.ok(status.slice(0, gate).includes("t('status.clean_screen')"));

  // The ribbon uses the same flag, so no dead palette button is left behind.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('offeredItems(panel.items, shell.palettesAvailable)'));
  assert.ok(!ribbon.includes('for (const item of panel.items)'));
});

test('the ribbon does not offer a palette toggle either', () => {
  // A ribbon button that toggles a panel which is never rendered is a dead
  // affordance, so the same carrier flag drops those items there too.
  const items = [
    { id: 'diagnostics', action: 'diagnostics' },
    { id: 'PROPERTIES', action: 'properties' },
    { id: 'clean', action: 'cleanScreen' },
    { id: 'OPEN', command: 'OPEN' },
  ];
  assert.deepEqual(PALETTE_ACTIONS, ['diagnostics', 'properties']);
  // Without the carrier flag the very same array comes back: no re-render churn,
  // no changed order, nothing filtered.
  assert.equal(offeredItems(items, true), items);
  assert.equal(offeredItems(items, undefined), items);
  assert.deepEqual(offeredItems(items, false).map(item => item.id), ['clean', 'OPEN']);

  // The shipped model really contains palette entries, and dropping them never
  // leaves a panel with an empty body: a panel that held nothing but palette
  // toggles (the Home tab's "Layers" panel is exactly that now that `LAYER` opens
  // the docked layers palette) is dropped whole by the renderer instead of
  // rendering a titled box with no buttons.
  let paletteEntries = 0;
  let droppedPanels = 0;
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      paletteEntries += panel.items.filter(item => PALETTE_ACTIONS.includes(item.action)
        || PALETTE_COMMANDS.includes(item.command)).length;
      const offered = offeredItems(panel.items, false);
      assert.equal(offeredPanels([panel], false).length > 0, offered.length > 0, panel.id);
      if (offered.length === 0) droppedPanels += 1;
    }
    assert.ok(offeredPanels(tab.panels, false).length > 0, `tab ${tab.id} keeps at least one panel`);
    for (const panel of offeredPanels(tab.panels, false)) {
      assert.ok(offeredItems(panel.items, false).length > 0, `offered panel ${panel.id} keeps at least one item`);
    }
  }
  assert.ok(paletteEntries >= 4, `expected the ribbon to offer palette entries, saw ${paletteEntries}`);
  assert.ok(droppedPanels >= 1, 'the layers-only panel is dropped, not left empty');
  assert.equal(offeredPanels(RIBBON_TABS[0].panels, true), RIBBON_TABS[0].panels);
});

test('the identity adds no user-visible copy to either catalog', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  // The session's own label is printed as data; nothing about it was translated,
  // so no new key was introduced for it.
  for (const key of [...Object.keys(zh), ...Object.keys(en)]) {
    assert.ok(!/canvas.?session/i.test(key), `unexpected catalog key ${key}`);
  }
  // And the touched modules carry no hardcoded CJK of their own.
  for (const name of ['App.tsx', 'cad/surface.ts', 'components/Workspace.tsx', 'components/StatusBar.tsx', 'components/DocTabs.tsx']) {
    const source = await readModule(name);
    assert.ok(!/[\u3400-\u9fff]/.test(source), `${name} must not carry hardcoded CJK`);
  }
});
