// Contract test for the fixed-height ribbon band. The band must not change
// height when the active tab changes (or when a variant popover opens), because
// the host samples the CAD partition rect from this chrome. These assertions
// lock the CSS token, the horizontal small-item flow, the in-band variant
// popover and the dismissal wiring, so a future edit cannot silently reintroduce
// the tab-to-tab height jump.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCss, readModule } from './helpers.mjs';

function block(css, selector) {
  const match = css.match(new RegExp(`${selector}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `expected a rule for ${selector}`);
  return match[1];
}

test('the ribbon band height is one token, identical on every tab', async () => {
  const css = await readCss();
  assert.match(css, /--ribbon-h:\s*\d+px/, 'expected a --ribbon-h token');
  const panels = block(css, '\\.ribbon__panels');
  assert.match(panels, /height:\s*var\(--ribbon-h\)/, 'the band must take its height from the token');
  // Regression guard: the old content-driven floor is what let tiles grow the band.
  assert.doesNotMatch(panels, /min-height:\s*80px/);
});

test('small items flow horizontally instead of one tall column', async () => {
  const css = await readCss();
  const small = block(css, '\\.ribbon-panel__group\\.is-small');
  assert.match(small, /display:\s*grid/);
  assert.match(small, /grid-auto-flow:\s*column/);
  assert.match(small, /grid-template-rows:/);
  assert.doesNotMatch(small, /flex-direction:\s*column/);
  // A compact icon is required so a small row fits the fixed band.
  assert.match(css, /\.ribbon-panel__group\.is-small \.ribbon-button \.icon\s*\{[^}]*width:\s*16px/);
});

test('the variant menu is an in-band popover that cannot grow the band', async () => {
  const css = await readCss();
  const menu = block(css, '\\.ribbon-panel__menu');
  assert.match(menu, /position:\s*absolute/);
  assert.match(menu, /max-height:\s*100%/);
  assert.match(menu, /overflow:\s*auto/);
  // It is confined to the panel body, which is the positioning context.
  const body = block(css, '\\.ribbon-panel__body');
  assert.match(body, /position:\s*relative/);
});

test('Ribbon renders the menu inside the panel body and marks the open item', async () => {
  const source = await readModule('components/Ribbon.jsx');
  const bodyAt = source.indexOf('ribbon-panel__body');
  const menuAt = source.indexOf('<VariantMenu');
  const titleAt = source.indexOf('className="ribbon-panel__title"');
  assert.ok(bodyAt !== -1 && menuAt !== -1 && titleAt !== -1);
  assert.ok(menuAt > bodyAt && menuAt < titleAt, 'the menu must render inside the panel body');
  assert.ok(source.includes("menuOpen ? 'is-open' : ''"), 'the invoking item must be marked open');
});

test('the variant popover is dismissible and keeps its accessibility roles', async () => {
  const source = await readModule('components/Ribbon.jsx');
  assert.ok(source.includes("event.key === 'Escape'"), 'Escape must dismiss the popover');
  assert.ok(source.includes("closest?.('.ribbon-panel__menu')"), 'an outside click must dismiss the popover');
  for (const role of ['role="tab"', 'role="tabpanel"', 'role="menu"', 'role="menuitem"']) {
    assert.ok(source.includes(role), `expected ${role} to be preserved`);
  }
  assert.ok(source.includes('aria-expanded={menuOpen}'));
});
