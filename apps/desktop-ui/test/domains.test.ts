// Application-domain contracts (ADR-0053): the token reader, the four profiles,
// the document-less preview, the write seam and the colours the stylesheet keys
// on. Pure Node tests — no DOM, no React.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readModule } from './helpers.ts';
import {
  DEFAULT_DOMAIN,
  DOMAIN_IDS,
  DOMAIN_PROFILES,
  domainCommand,
  domainFromArgument,
  domainOf,
  domainProfile,
  normalizeDomain,
  unknownDomainToken,
  writeDomain,
} from '../src/domains/domains.ts';
import {
  clearPreviewDomain,
  currentDomain,
  domainStoreSnapshot,
  previewDomain,
  renderedDomain,
  resetDomainStore,
  setSwitcherOpen,
  setTrayHidden,
  toggleSwitcher,
  toggleTray,
  trayHidden,
} from '../src/domains/store.ts';
import { PANEL_ACTIONS } from '../src/chrome/panel-actions.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { SHELL_ACTIONS } from '../src/ribbon/actions.ts';
import { BIM_RIBBON_TABS } from '../src/domains/bim/ribbon.ts';
import { CONCEPT_MENUS } from '../src/domains/concept/menus.ts';

test('a status token is read verbatim and anything else stays drafting', () => {
  for (const id of DOMAIN_IDS) assert.equal(normalizeDomain(id), id);
  assert.equal(normalizeDomain('revit'), null, 'reference names are not file tokens');
  assert.equal(normalizeDomain(undefined), null);
  assert.equal(normalizeDomain(7), null);
  assert.equal(domainOf({ applicationDomain: 'bim' }), 'bim');
  assert.equal(domainOf({}), DEFAULT_DOMAIN);
  assert.equal(domainOf(null), DEFAULT_DOMAIN);
});

test('drafting, BIM and concept are wired; mechanical stays an explicit placeholder', () => {
  assert.deepEqual([...DOMAIN_IDS], ['cad2d', 'bim', 'concept', 'mechanical']);
  for (const id of DOMAIN_IDS) assert.equal(DOMAIN_PROFILES[id].id, id);
  assert.deepEqual(
    {
      ribbon: DOMAIN_PROFILES.cad2d.ribbon,
      surface: DOMAIN_PROFILES.cad2d.surface,
      commandLine: DOMAIN_PROFILES.cad2d.commandLine,
      placeholder: DOMAIN_PROFILES.cad2d.placeholder,
    },
    { ribbon: true, surface: 'ribbon', commandLine: true, placeholder: false },
  );
  assert.equal(DOMAIN_PROFILES.cad2d.traySide, 'right', 'drafting registers no tray slot');
  // Concept is a SketchUp-style menubar + tool column + tray, not a ribbon and
  // not a command-line profile; its value entry is the Measurements surface.
  assert.deepEqual(
    {
      ribbon: DOMAIN_PROFILES.concept.ribbon,
      surface: DOMAIN_PROFILES.concept.surface,
      commandLine: DOMAIN_PROFILES.concept.commandLine,
      placeholder: DOMAIN_PROFILES.concept.placeholder,
    },
    { ribbon: true, surface: 'menu', commandLine: false, placeholder: false },
  );
  // SketchUp docks its default tray on the right.
  assert.equal(DOMAIN_PROFILES.concept.traySide, 'right');
  // BIM is a Revit-style ribbon band from the domain `ribbon` slot plus its
  // left-hand tray (Revit docks Properties/Project Browser on the left); it is
  // not the drafting ribbon and has no command line.
  assert.deepEqual(
    {
      ribbon: DOMAIN_PROFILES.bim.ribbon,
      surface: DOMAIN_PROFILES.bim.surface,
      commandLine: DOMAIN_PROFILES.bim.commandLine,
      placeholder: DOMAIN_PROFILES.bim.placeholder,
    },
    { ribbon: true, surface: 'domain-ribbon', commandLine: false, placeholder: false },
  );
  assert.equal(DOMAIN_PROFILES.bim.traySide, 'left');
  for (const id of ['mechanical']) {
    const profile = DOMAIN_PROFILES[id];
    assert.equal(profile.ribbon, false, `${id} must not show the drafting ribbon`);
    assert.equal(profile.surface, 'ribbon');
    assert.equal(profile.commandLine, false, `${id} must not show the drafting command line`);
    assert.equal(profile.placeholder, true);
  }
  assert.equal(domainProfile({ applicationDomain: 'bim' }).id, 'bim');
});

test('the DOMAIN argument accepts reference names as input aliases only', () => {
  assert.equal(domainFromArgument('autocad'), 'cad2d');
  assert.equal(domainFromArgument('DWG'), 'cad2d');
  assert.equal(domainFromArgument(' revit '), 'bim');
  assert.equal(domainFromArgument('skp'), 'concept');
  assert.equal(domainFromArgument('SolidWorks'), 'mechanical');
  assert.equal(domainFromArgument('mechanical'), 'mechanical');
  assert.equal(domainFromArgument('revit2024'), null);
  assert.equal(domainFromArgument(''), null);
  assert.equal(domainFromArgument(undefined), null);
});

test('the DOMAIN word opens the switcher or carries one domain token', () => {
  assert.deepEqual(domainCommand('DOMAIN'), { kind: 'open' });
  assert.deepEqual(domainCommand('  domain  '), { kind: 'open' });
  assert.deepEqual(domainCommand('DOMAIN revit'), { kind: 'set', domain: 'bim' });
  assert.deepEqual(domainCommand('domain sketchup'), { kind: 'set', domain: 'concept' });
  assert.deepEqual(domainCommand('DOMAIN mechanical'), { kind: 'set', domain: 'mechanical' });
  assert.deepEqual(domainCommand('DOMAIN cad2d'), { kind: 'set', domain: 'cad2d' });
  assert.deepEqual(domainCommand('DOMAIN revit2024'), { kind: 'unknown', token: 'revit2024' });
  // A longer word is a different command, never `DOMAIN` with an argument.
  assert.equal(domainCommand('DOMAINX'), null);
  assert.equal(domainCommand('DOMAINS revit'), null);
  assert.equal(domainCommand(''), null);
  assert.equal(domainCommand(null), null);
});

test('an unwired or unknown domain falls back to the drafting interface', () => {
  resetDomainStore();
  // BIM and concept are wired: the chrome renders their own profiles.
  assert.equal(renderedDomain({ documentLabel: 'a.rvt', applicationDomain: 'bim' }), 'bim');
  assert.equal(renderedDomain({ documentLabel: 'a.skp', applicationDomain: 'concept' }), 'concept');
  // Known but not wired: mechanical is a placeholder, so the chrome renders the
  // default (drafting) interface while the document keeps its value.
  assert.equal(renderedDomain({ documentLabel: 'a.sldprt', applicationDomain: 'mechanical' }), 'cad2d');
  // Unknown token: normalise to drafting and name the token for the notice.
  assert.equal(unknownDomainToken({ applicationDomain: 'manufacturing' }), 'manufacturing');
  assert.equal(unknownDomainToken({ applicationDomain: 'bim' }), null);
  assert.equal(unknownDomainToken({}), null);
  assert.equal(renderedDomain({ documentLabel: 'a.x', applicationDomain: 'manufacturing' }), 'cad2d');
  // A no-document preview of a wired domain renders its own profile; a preview
  // of the remaining placeholder still falls back.
  previewDomain('bim');
  assert.equal(renderedDomain(null), 'bim');
  previewDomain('mechanical');
  assert.equal(renderedDomain(null), 'cad2d');
  clearPreviewDomain();
});

test('a document wins over the session preview', () => {
  resetDomainStore();
  assert.equal(currentDomain(null), 'cad2d');
  previewDomain('bim');
  assert.equal(domainStoreSnapshot().preview, 'bim');
  assert.equal(currentDomain(null), 'bim');
  assert.equal(
    currentDomain({ documentLabel: 'a.dwg', applicationDomain: 'mechanical' }),
    'mechanical',
    'a document always reports its own domain',
  );
  clearPreviewDomain();
  assert.equal(currentDomain(null), 'cad2d');
});

test('the switcher flag is one shared surface', () => {
  resetDomainStore();
  assert.equal(domainStoreSnapshot().switcherOpen, false);
  toggleSwitcher();
  assert.equal(domainStoreSnapshot().switcherOpen, true);
  setSwitcherOpen(false);
  assert.equal(domainStoreSnapshot().switcherOpen, false);
});

test('the tray flag is one shared surface and resets with the store', () => {
  resetDomainStore();
  assert.equal(domainStoreSnapshot().trayHidden, false);
  assert.equal(trayHidden(), false);
  toggleTray();
  assert.equal(trayHidden(), true);
  toggleTray();
  assert.equal(trayHidden(), false);
  setTrayHidden(true);
  assert.equal(trayHidden(), true);
  // Idempotent: setting the same value publishes nothing new.
  setTrayHidden(true);
  assert.equal(domainStoreSnapshot().trayHidden, true);
  resetDomainStore();
  assert.equal(trayHidden(), false);
  // The ribbon/menu action table dispatches the same transition (no branch was
  // added to `App.tsx`).
  assert.equal(PANEL_ACTIONS.domainTray, toggleTray);
});

test('the write seam reports acceptance and the host refusal verbatim', async () => {
  const accepted = await writeDomain(async () => ({ accepted: true }), 'bim');
  assert.deepEqual(accepted, { kind: 'accepted' });

  const refused = await writeDomain(async () => ({ accepted: false }), 'bim');
  assert.equal(refused.kind, 'failed');

  const rejected = await writeDomain(async () => {
    throw new Error('domain.unknown: revit');
  }, 'bim');
  assert.deepEqual(rejected, { kind: 'failed', code: 'domain.unknown: revit' });
});

test('the chrome regions and theme read the rendered (fallback) domain', async () => {
  const chrome = await readModule('domains/chrome.tsx');
  assert.ok(chrome.includes('renderedDomain(shell.status)'), 'ribbon/command line must use the fallback');
  const switcher = await readModule('components/DomainSwitcher.tsx');
  assert.ok(switcher.includes('applyDomainAttribute(rendered)'), 'the theme follows the rendered domain');
  const surface = await readModule('components/DomainSurface.tsx');
  assert.ok(surface.includes("'domain.fallback_title'"), 'the fallback is disclosed, not silent');
  assert.ok(surface.includes('unknownDomainToken'), 'an unknown token is named');
});

test('the workspace places the tray on the profile-side of the canvas', async () => {
  const workspace = await readModule('components/Workspace.tsx');
  assert.match(workspace, /DOMAIN_PROFILES\[renderedDomain\(shell\.status\)\]\.traySide/);
  const left = workspace.indexOf("traySide === 'left'");
  const canvas = workspace.indexOf('className="workspace__canvas"');
  const right = workspace.indexOf("traySide === 'right'");
  assert.ok(left > 0 && left < canvas, 'the left tray renders before the canvas (Revit)');
  assert.ok(right > canvas, 'the right tray renders after the canvas (SketchUp)');
  assert.ok(workspace.includes('<DomainTray side="left" />'));
  assert.ok(workspace.includes('<DomainTray side="right" />'));
});

test('every profile puts the domain switcher under File, one row each', () => {
  const profiles = [
    { name: 'cad2d', tabs: RIBBON_TABS },
    { name: 'bim', tabs: BIM_RIBBON_TABS },
    { name: 'concept', tabs: CONCEPT_MENUS },
  ];
  for (const { name, tabs } of profiles) {
    const file = tabs.find(tab => tab.panels.some(panel => panel.id === 'domain'));
    assert.ok(file, `${name} has the shared domain panel`);
    assert.equal(file, tabs[0], `${name} keeps it on the File surface`);
    const rows = file.panels
      .flatMap(panel => [...panel.items])
      .filter(item => item.action === SHELL_ACTIONS.domainSwitcher);
    assert.equal(rows.length, 1, `${name} has exactly one switcher row`);
    assert.equal(rows[0].labelKey, 'domain.switcher_label');
  }
});

test('the stylesheet keys a token block on every non-drafting domain', async () => {
  const css = await readCss();
  for (const id of ['bim', 'concept', 'mechanical']) {
    assert.match(css, new RegExp(`\\[data-domain="${id}"\\]`), `${id} needs a token block`);
  }
  // The renderer's canvas pair is not overridden by a chrome preset.
  assert.doesNotMatch(
    css,
    /\[data-domain="(bim|concept|mechanical)"\][^{]*\{[^}]*--canvas:/,
    'the drawing backdrop stays the host renderer setting',
  );
});

test('every domain string the chrome renders exists in both catalogs', async () => {
  const keys = [
    'domain.switcher',
    'domain.switcher_label',
    'domain.close',
    'domain.preview_hint',
    'domain.placeholder_tag',
    'domain.placeholder_title',
    'domain.placeholder_body',
    'domain.fallback_title',
    'domain.fallback_body',
    'domain.fallback_unknown',
    'domain.fallback_dismiss',
    'domain.return',
    'domain.switch_failed',
    'domain.unknown',
  ];
  for (const id of DOMAIN_IDS) {
    keys.push(DOMAIN_PROFILES[id].labelKey, DOMAIN_PROFILES[id].referenceKey);
  }
  for (const locale of ['zh-CN', 'en']) {
    const catalog = await readCatalog(locale);
    for (const key of keys) assert.ok(catalog[key], `${locale} is missing ${key}`);
  }
});
