// Contract tests for the theme switch.
//
// The one rule that matters: the host is the authority. The component writes the
// opposite value through `cad_set_setting` and never flips a local copy; the
// document attribute changes only when the host's next `cad_status.theme`
// arrives. The rest locks the first-paint cache, the one `data-theme` writer,
// and the light token block the stylesheet keys on.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import {
  DEFAULT_THEME,
  THEME_ATTRIBUTE,
  THEME_SETTING_KEY,
  THEME_STORAGE_KEY,
  normalizeTheme,
  oppositeTheme,
  shouldReplayStoredTheme,
} from '../src/theme/theme.ts';

test('only the two documented names are themes; the toggle is a pure inversion', () => {
  assert.equal(DEFAULT_THEME, 'dark');
  assert.equal(THEME_STORAGE_KEY, 'yacr.theme');
  assert.equal(THEME_SETTING_KEY, 'theme');
  assert.equal(THEME_ATTRIBUTE, 'data-theme');
  assert.equal(normalizeTheme('dark'), 'dark');
  assert.equal(normalizeTheme('light'), 'light');
  for (const value of ['Dark', '', null, undefined, 0, true, {}, 'blue']) {
    assert.equal(normalizeTheme(value), null, String(value));
  }
  assert.equal(oppositeTheme('dark'), 'light');
  assert.equal(oppositeTheme('light'), 'dark');
});

test('a dom-canvas boot replays the cached theme; a native host is never overridden', () => {
  // Only the in-memory wasm carrier loses its settings on reload.
  assert.equal(shouldReplayStoredTheme('dark', 'dom-canvas', 'light'), true);
  assert.equal(shouldReplayStoredTheme('light', 'dom-canvas', 'dark'), true);
  // A native host persisted the value itself: a stale mirror must not win.
  assert.equal(shouldReplayStoredTheme('dark', 'native-below', 'light'), false);
  assert.equal(shouldReplayStoredTheme('dark', undefined, 'light'), false);
  // Agreement, no cache, or a malformed host value is not a replay.
  assert.equal(shouldReplayStoredTheme('light', 'dom-canvas', 'light'), false);
  assert.equal(shouldReplayStoredTheme('dark', 'dom-canvas', null), false);
  assert.equal(shouldReplayStoredTheme('blue', 'dom-canvas', 'light'), false);
});

test('the theme module writes the document attribute, the guarded cache and the one IPC', async () => {
  const theme = await readModule('theme/theme.ts');
  assert.ok(theme.includes('document.documentElement.dataset.theme = theme'));
  assert.ok(theme.includes('writeSetting(invoke, THEME_SETTING_KEY, theme)'), 'the Options write seam is reused');
  assert.ok(theme.includes('localStorage'), 'the cache is localStorage');
  assert.ok(theme.includes('try {'), 'a blocked storage is caught, not thrown');
  // The host value wins; an absent one keeps the cached first paint.
  assert.ok(theme.includes('const host = normalizeTheme(hostTheme);'));
  assert.ok(theme.includes('const theme = host ?? initialTheme();'));
  assert.ok(theme.includes('useLayoutEffect'), 'a newly reported theme applies before paint');
  // A write refusal is reported by the caller; the module never applies the
  // requested value itself.
  const setStart = theme.indexOf('export function setTheme');
  const setBody = theme.slice(setStart, theme.indexOf('\n}', setStart));
  assert.equal(setBody.includes('applyTheme('), false, 'no optimistic flip');
  assert.equal(setBody.includes('storeTheme('), false, 'no optimistic cache write');
});

test('main applies the cached theme before React mounts', async () => {
  const main = await readModule('main.tsx');
  const apply = main.indexOf('applyTheme(initialTheme())');
  const mount = main.indexOf('createRoot(');
  assert.ok(apply > 0, 'the first paint is themed');
  assert.ok(mount > apply, 'before the first render');
});

test('the toggle never applies a theme locally, and both title rows carry it', async () => {
  const toggle = await readModule('components/ThemeToggle.tsx');
  assert.equal(toggle.includes('applyTheme('), false, 'the component does not write the attribute');
  assert.equal(toggle.includes('storeTheme('), false, 'the component does not write the cache');
  assert.ok(toggle.includes('setTheme(target)'));
  assert.ok(toggle.includes('oppositeTheme(theme)'), 'the target is the opposite of host truth');
  assert.ok(toggle.includes("t('shell.theme.toggle'"), 'the accessible name comes from the catalog');
  assert.ok(toggle.includes("t('options.error.generic')"), 'a refused write is not silent');

  const title = await readModule('components/TitleBar.tsx');
  assert.ok(title.includes("import { ThemeToggle } from './ThemeToggle.tsx';"));
  assert.ok(title.includes('<ThemeToggle />'));
  const mobile = await readModule('components/MobileTopBar.tsx');
  assert.ok(mobile.includes("import { ThemeToggle } from './ThemeToggle.tsx';"));
  assert.ok(mobile.includes('<ThemeToggle />'));
});

test('the light theme overrides the whole token set and the hardcoded colours are tokenised', async () => {
  const css = await readModule('style.css');
  const start = css.indexOf(':root[data-theme="light"]');
  assert.ok(start > 0, 'the light block exists');
  const block = css.slice(start, css.indexOf('\n}', start));
  assert.ok(block.includes('color-scheme: light'));
  for (const token of [
    '--chrome:', '--chrome-2:', '--chrome-3:', '--panel:', '--field:', '--hover:', '--pressed:',
    '--stripe:', '--row-hover:', '--well:', '--well-hi:', '--line:', '--line-soft:',
    '--text:', '--text-dim:', '--text-muted:', '--text-disabled:',
    '--accent:', '--accent-text:', '--accent-strong:', '--on:', '--focus:', '--danger:', '--ok:',
    '--shadow-rgb:', '--scrim-overlay:', '--scrim-drawer:', '--scrim-sheet:',
  ]) {
    assert.ok(block.includes(token), `light block carries ${token}`);
  }
  // The dark chrome stays the historical one: no `data-theme="dark"` block and
  // the base token values are untouched.
  assert.equal(css.includes(':root[data-theme="dark"]'), false);
  assert.ok(css.includes('--chrome: #3b4453;'));
  // Every former raw shadow/white-on-accent literal now reads a token. The only
  // raw `rgba(...)` left outside the token blocks belongs to the token values.
  const rules = css.slice(css.indexOf('* { box-sizing'));
  assert.equal(/#fff\b/.test(rules), false, 'white-on-accent is --on-text');
  assert.equal(/rgba\(0, 0, 0,/.test(rules), false, 'shadows use --shadow-rgb');
  assert.equal(/rgba\(59, 68, 83, 0\)/.test(rules), false, 'the chrome fade is --chrome-0');
  // Compact chrome follows the same tokens, and the nav rail is no longer hidden.
  const mobile = await readModule('mobile.css');
  assert.equal(/#fff\b/.test(mobile), false);
  assert.equal(/rgba\(0, 0, 0,/.test(mobile), false);
  assert.ok(mobile.includes('var(--scrim-drawer)'));
  assert.ok(mobile.includes('.mobile-topbar .ui-icon-button'));
});
