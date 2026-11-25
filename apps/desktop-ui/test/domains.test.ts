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
  toggleSwitcher,
} from '../src/domains/store.ts';

test('a status token is read verbatim and anything else stays drafting', () => {
  for (const id of DOMAIN_IDS) assert.equal(normalizeDomain(id), id);
  assert.equal(normalizeDomain('revit'), null, 'reference names are not file tokens');
  assert.equal(normalizeDomain(undefined), null);
  assert.equal(normalizeDomain(7), null);
  assert.equal(domainOf({ applicationDomain: 'bim' }), 'bim');
  assert.equal(domainOf({}), DEFAULT_DOMAIN);
  assert.equal(domainOf(null), DEFAULT_DOMAIN);
});

test('drafting and concept are wired; bim and mechanical stay explicit placeholders', () => {
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
  for (const id of ['bim', 'mechanical']) {
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
  // Known but not wired: the profile is a placeholder, so the chrome renders the
  // default (drafting) interface while the document keeps its value.
  assert.equal(renderedDomain({ documentLabel: 'a.rvt', applicationDomain: 'bim' }), 'cad2d');
  assert.equal(renderedDomain({ documentLabel: 'a.sldprt', applicationDomain: 'mechanical' }), 'cad2d');
  assert.equal(renderedDomain({ documentLabel: 'a.skp', applicationDomain: 'concept' }), 'concept');
  // Unknown token: normalise to drafting and name the token for the notice.
  assert.equal(unknownDomainToken({ applicationDomain: 'manufacturing' }), 'manufacturing');
  assert.equal(unknownDomainToken({ applicationDomain: 'bim' }), null);
  assert.equal(unknownDomainToken({}), null);
  assert.equal(renderedDomain({ documentLabel: 'a.x', applicationDomain: 'manufacturing' }), 'cad2d');
  // A no-document preview of a placeholder also falls back.
  previewDomain('bim');
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
