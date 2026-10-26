// The colour chooser's contract: the palette mirror, the value grammar, what each
// palette may offer, and the interaction the popover promises.
//
// The pure model is imported and exercised directly (`color/color-value.ts`,
// `color/aci-palette.ts`); the components are read as **text** and asserted on
// their structure, the same way `properties.test.ts` and `layers.test.ts` check the
// palette they belong to. No test here mounts React: the suite runs under Node's
// type stripping with no DOM, so a source scan plus a pure model is the honest
// evidence, not a claim about rendered pixels.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  ACI_COLORS,
  ACI_INDEXES,
  aciHex,
} from '../src/color/aci-palette.ts';
import {
  COLOR_GRID,
  COLOR_GRID_COLUMNS,
  ENTITY_COLOR_SYMBOLS,
  LAYER_COLOR_SYMBOLS,
  aciChoice,
  colorSwatch,
  isSelectedSymbol,
  parseColorValue,
  selectedAciIndex,
  swatchCursor,
  swatchDelta,
  symbolLabelKey,
} from '../src/color/color-value.ts';
import { nextFocusIndex } from '../src/layers/layers.ts';
import { PROPERTY_ERRORS, propertyCommit, validatePropertyValue } from '../src/properties/properties.ts';
import { colorPayload } from '../src/layers/layer-attributes.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');

// --- the palette mirror -------------------------------------------------

test('the ACI table is the host table, triple for triple', async () => {
  // `crates/cad-app/src/property_value/aci.rs` is the one table the colour grammar
  // resolves (`aci:N`), so the picker may not carry a second opinion about what a
  // swatch looks like. Read the Rust source and compare every entry.
  const rust = await readFile(
    new URL('../../../crates/cad-app/src/property_value/aci.rs', import.meta.url),
    'utf8',
  );
  const tuples = [...rust.matchAll(/\((\d+),\s*(\d+),\s*(\d+)\)/g)]
    .map(match => match.slice(1).map(Number));
  assert.equal(tuples.length, 256, 'the host table has one triple per index');
  const hex = tuples.map(([r, g, b]) => `#${[r, g, b]
    .map(channel => channel.toString(16).padStart(2, '0'))
    .join('')}`);
  assert.equal(ACI_COLORS.length, 256);
  assert.deepEqual([...ACI_COLORS], hex, 'the picker mirrors the host palette exactly');
});

test('only real palette entries are colours; 0 and 256 are the symbolic variants', () => {
  assert.equal(aciHex(1), '#ff0000');
  assert.equal(aciHex(7), '#ffffff');
  assert.equal(aciHex(255), '#ffffff');
  // ByBlock/ByLayer placeholders and out-of-range input are not colours.
  for (const invalid of [0, 256, -1, 257, 1.5, '7', null, undefined]) {
    assert.equal(aciHex(invalid), null, `aciHex(${String(invalid)})`);
  }
  assert.deepEqual([...ACI_INDEXES], Array.from({ length: 255 }, (_, offset) => offset + 1));
});

test('the grid is the standard palette, in index order, painted by aciHex', () => {
  assert.equal(COLOR_GRID.length, 255);
  assert.equal(COLOR_GRID[0].index, 1);
  assert.equal(COLOR_GRID[254].index, 255);
  assert.equal(COLOR_GRID_COLUMNS, 15);
  assert.equal(COLOR_GRID.length % COLOR_GRID_COLUMNS, 0, 'the grid is whole rows');
  for (const cell of COLOR_GRID) assert.equal(cell.hex, aciHex(cell.index));
});

// --- the value grammar --------------------------------------------------

test('a value is read in the host grammar and never guessed', () => {
  assert.deepEqual(parseColorValue(''), { kind: 'empty' });
  assert.deepEqual(parseColorValue('   '), { kind: 'empty' });
  assert.deepEqual(parseColorValue(null), { kind: 'empty' });
  assert.deepEqual(parseColorValue('ByLayer'), { kind: 'symbolic', symbol: 'ByLayer' });
  assert.deepEqual(parseColorValue('bylayer'), { kind: 'symbolic', symbol: 'ByLayer' });
  assert.deepEqual(parseColorValue('BYBLOCK'), { kind: 'symbolic', symbol: 'ByBlock' });
  // The grammar's own symbolic indexes parse as the tokens they mean, so the chooser
  // marks ByLayer/ByBlock rather than a swatch.
  assert.deepEqual(parseColorValue('aci:0'), { kind: 'symbolic', symbol: 'ByBlock' });
  assert.deepEqual(parseColorValue('aci:256'), { kind: 'symbolic', symbol: 'ByLayer' });
  assert.deepEqual(parseColorValue('aci:1'), { kind: 'aci', index: 1, hex: '#ff0000' });
  assert.deepEqual(parseColorValue('ACI:7'), { kind: 'aci', index: 7, hex: '#ffffff' });
  assert.deepEqual(parseColorValue('#FF0000'), { kind: 'hex', hex: '#ff0000' });
  for (const other of ['#fff', 'red', 'aci:999', 'aci:', 'opacity:0.5', 'rgb(1,2,3)']) {
    assert.equal(parseColorValue(other).kind, 'unparsed', other);
  }
});

test('the trigger paints a real colour or a symbolic marker, never a guess', () => {
  assert.deepEqual(colorSwatch('aci:1'), { hex: '#ff0000', symbol: null });
  assert.deepEqual(colorSwatch('#00ff00'), { hex: '#00ff00', symbol: null });
  assert.deepEqual(colorSwatch('ByLayer'), { hex: null, symbol: 'ByLayer' });
  assert.deepEqual(colorSwatch(''), { hex: null, symbol: null });
  assert.deepEqual(colorSwatch('red'), { hex: null, symbol: null });
  // Only an indexed value selects a swatch: a true colour is not an index, and the
  // chooser must not claim the drawing stores one.
  assert.equal(selectedAciIndex('aci:5'), 5);
  assert.equal(selectedAciIndex('#ff0000'), null);
  assert.equal(selectedAciIndex('ByLayer'), null);
  assert.equal(isSelectedSymbol('ByLayer', 'ByLayer'), true);
  assert.equal(isSelectedSymbol('byblock', 'ByBlock'), true);
  assert.equal(isSelectedSymbol('#ff0000', 'ByLayer'), false);
});

// --- what each palette may offer ----------------------------------------

test('the entity chooser offers exactly the symbols the entity grammar accepts', async () => {
  assert.deepEqual([...ENTITY_COLOR_SYMBOLS], ['ByLayer', 'ByBlock']);
  for (const symbol of ENTITY_COLOR_SYMBOLS) {
    // The client mirror and the host both take it, so the entry can never be a
    // guaranteed refusal.
    assert.equal(validatePropertyValue('color', symbol), null, symbol);
  }
  // The host's own grammar table names both tokens, and it does **not** name
  // `ByObject` — AutoCAD's display label for ByBlock. Offering that literal string
  // would offer a guaranteed refusal, so it is not offered.
  const grammar = await readFile(
    new URL('../../../crates/cad-app/src/property_value/mod.rs', import.meta.url),
    'utf8',
  );
  assert.match(grammar, /`color` \| `ByLayer`, `ByBlock`/);
  assert.equal(ENTITY_COLOR_SYMBOLS.includes('ByObject'), false);
});

test('the layer chooser offers no symbol, because the layer grammar refuses both', async () => {
  assert.deepEqual([...LAYER_COLOR_SYMBOLS], []);
  // The layer write path refuses a symbolic colour by name (a layer has no layer
  // above it), so an entry for one would be a guaranteed refusal.
  const grammar = await readFile(
    new URL('../../../crates/cad-app/src/layer_property.rs', import.meta.url),
    'utf8',
  );
  assert.match(grammar, /refused \*\*by name\*\*/);
  for (const symbol of ENTITY_COLOR_SYMBOLS) {
    assert.equal(colorPayload(symbol), '', `${symbol} never reaches the layer wire`);
  }
});

// --- the write syntax is the existing one -------------------------------

test('a swatch writes the row grammar\'s own `aci:N`, through the existing commit', () => {
  assert.equal(aciChoice(7), 'aci:7');
  // The client mirror accepts it (so a pick is never refused locally)...
  assert.equal(validatePropertyValue('color', aciChoice(7)), null);
  // ...and the palette's existing commit turns it into the one `{key, value}` pair
  // the host already parses. No second value form is introduced.
  const colorRow = {
    key: 'color',
    editable: true,
    mixed: false,
    rawValue: 'ByLayer',
    options: [],
  };
  assert.deepEqual(propertyCommit(colorRow, aciChoice(7)), {
    kind: 'send',
    key: 'color',
    value: 'aci:7',
  });
  assert.deepEqual(propertyCommit(colorRow, 'ByBlock'), {
    kind: 'send',
    key: 'color',
    value: 'ByBlock',
  });
  // Re-picking the value the row already shows queues nothing at all.
  assert.deepEqual(propertyCommit({ ...colorRow, rawValue: 'aci:7' }, 'aci:7'), {
    kind: 'skip',
    reason: 'unchanged',
  });
  // A mixed row writes to every selected object, which is the palette's existing
  // rule, not a new one.
  assert.equal(propertyCommit({ ...colorRow, mixed: true, rawValue: null }, 'aci:7').kind, 'send');
  assert.equal(validatePropertyValue('color', 'aci:257'), PROPERTY_ERRORS.invalidValue);
});

test('the layer colour routing takes the index and the true colour, nothing else', () => {
  assert.equal(colorPayload('aci:7'), 'aci:7');
  assert.equal(colorPayload(' ACI:255 '), 'aci:255');
  assert.equal(colorPayload('#ff7f00'), '#FF7F00');
  // `aci:0`/`aci:256` are the layer grammar's symbolic refusals, and anything else
  // is not a colour at all: no write is produced.
  for (const value of ['aci:0', 'aci:256', 'aci:999', 'ByLayer', 'ByBlock', '', 'red', '#fff']) {
    assert.equal(colorPayload(value), '', value);
  }
});

// --- the properties palette offers it -----------------------------------

test('the colour row offers the chooser beside its existing text entry', async () => {
  const palette = await readModule('components/PropertiesPalette.tsx');
  assert.ok(palette.includes("import { ColorField } from './ColorChooser.tsx';"));
  assert.ok(palette.includes("import { ENTITY_COLOR_SYMBOLS } from '../color/color-value.ts';"));
  // The row keeps the text field: it stays the fallback for any syntax the grid
  // cannot express, and the chooser is added beside it.
  assert.ok(palette.includes('const textField = ('), 'the text entry is still the row editor');
  assert.ok(palette.includes('<ColorField'), 'the same cell wraps it');
  assert.ok(palette.includes('symbols={ENTITY_COLOR_SYMBOLS}'));
  assert.ok(palette.includes('onPick={value => onPick(row, value)}'));
  assert.ok(palette.includes('{textField}'), 'the field is rendered inside the cell');
  // Only the colour row gets it.
  assert.ok(palette.includes("if (row.key === 'color') {"));
  // The label is catalog copy, and the tooltip names what the trigger opens.
  assert.ok(palette.includes("label={t('properties.row.color')}"));
});

test('a pick is one write through the shell, never a second IPC', async () => {
  const palette = await readModule('components/PropertiesPalette.tsx');
  // One guarded write, exactly like Enter: a queued row drops the press, and the
  // value goes out through the one existing channel.
  assert.ok(palette.includes([
    '  function choose(row: PropertyRowView, value: string) {',
    '    const entry = editsRef.current[row.key];',
    '    if (entry && entry.sent !== null) return;',
    '    void send(row, value);',
    '  }',
  ].join('\n')));
  assert.ok(palette.includes('shell.applySelectedProperty(check.key, check.value)'));
  assert.ok(!palette.includes('invoke('), 'the palette never calls Tauri directly');
  assert.ok(!palette.includes('cad_set_selected_property'), 'the IPC name lives in properties.ts only');
  // A refusal keeps travelling the palette's own error line.
  assert.ok(palette.includes('title={message.code}'));
});

test('the layer details line uses the same cell through the store\'s write path', async () => {
  const details = await readModule('components/LayerDetails.tsx');
  assert.ok(details.includes("import { ColorField } from './ColorChooser.tsx';"));
  assert.ok(details.includes("import { LAYER_COLOR_SYMBOLS } from '../color/color-value.ts';"));
  assert.ok(details.includes('symbols={LAYER_COLOR_SYMBOLS}'), 'no entry the grammar refuses');
  assert.ok(details.includes('custom={colorPayload}'), 'the free-form entry is the layer grammar');
  assert.ok(details.includes('paint={color}'), 'the cell paints what the row dot paints');
  assert.ok(details.includes('onPick={onPickColor}'), 'one write per pick, through the store');
  assert.equal(details.includes('type="color"'), false, 'the webview colour dialog is gone');
  // The palette's write path is untouched: `colorPayload` → `store.setProperty`.
  const palette = await readModule('components/LayersPalette.tsx');
  assert.ok(palette.includes('const color = colorPayload(value);'));
  assert.ok(palette.includes("void store.setProperty(layer.name, key, value);"));
});

// --- the popover's interaction ------------------------------------------

test('the chooser closes on Escape, a press outside and Tab, and hands focus back', async () => {
  const code = await readModule('components/ColorChooser.tsx');
  // Escape: the chooser owns it and stops it, so the shell's global Escape (a tool
  // cancel, a field blur) never also fires for the same press.
  assert.ok(code.includes("if (event.key !== 'Escape') return;"));
  assert.ok(code.includes('event.preventDefault();'));
  assert.ok(code.includes('event.stopPropagation();'));
  assert.ok(code.includes('onClose({ restoreFocus: true });'));
  // A press outside dismisses without preventing it: the CAD partition still gets
  // the press, exactly like the shortcut menu.
  assert.ok(code.includes("document.addEventListener('pointerdown', onPointerDown, true);"));
  assert.ok(code.includes('if (!inside(event.target)) dismiss();'));
  // The anchor can never drift: blur, resize and any scroll of another node close it.
  assert.ok(code.includes("window.addEventListener('blur', dismiss);"));
  assert.ok(code.includes("window.addEventListener('resize', dismiss);"));
  assert.ok(code.includes("window.addEventListener('scroll', onScroll, true);"));
  // Focus returns to the trigger the cell owns.
  assert.ok(code.includes('triggerRef.current?.focus?.({ preventScroll: true });'));
  // Tab out closes rather than leaving a stranded popover.
  assert.ok(code.includes('const onBlur = (event: React.FocusEvent<HTMLDivElement>) => {'));
  assert.ok(code.includes('onClose({ restoreFocus: false });'));
});

test('the grid is one tab stop with an arrow-key cursor', async () => {
  const code = await readModule('components/ColorChooser.tsx');
  assert.ok(code.includes('tabIndex={cell.index - 1 === cursor ? 0 : -1}'), 'one tab stop');
  assert.ok(code.includes('onKeyDown={onGridKeyDown}'));
  assert.ok(code.includes('focusSwatch(next);'));
  // The pure step table: one cell sideways, one row up/down, first/last at the ends.
  assert.equal(swatchDelta('ArrowRight', 15), 1);
  assert.equal(swatchDelta('ArrowLeft', 15), -1);
  assert.equal(swatchDelta('ArrowDown', 15), 15);
  assert.equal(swatchDelta('ArrowUp', 15), -15);
  for (const key of ['Home', 'End', 'Enter', ' ', 'Escape', 'a', 'Tab']) {
    assert.equal(swatchDelta(key, 15), null, key);
  }
  // The cursor itself: arrows wrap at both ends with the app's existing list rule,
  // and Home/End are absolute (they are not a large step: a 255-step wrap from the
  // last swatch would stay on the last swatch).
  assert.equal(swatchCursor(254, 'ArrowRight', 15), 0);
  assert.equal(swatchCursor(0, 'ArrowLeft', 15), 254);
  assert.equal(swatchCursor(14, 'ArrowDown', 15), 29);
  assert.equal(swatchCursor(0, 'ArrowUp', 15), 240);
  assert.equal(swatchCursor(200, 'Home', 15), 0);
  assert.equal(swatchCursor(3, 'End', 15), 254);
  assert.equal(swatchCursor(-1, 'Enter', 15), null);
  assert.equal(nextFocusIndex(254, 1, 255), 0);
});

test('the chooser exposes the accessibility attributes a popover needs', async () => {
  const code = await readModule('components/ColorChooser.tsx');
  // Trigger: a named button that announces the popover and its own open state.
  assert.ok(code.includes('aria-label={label}'));
  assert.ok(code.includes('aria-haspopup="dialog"'));
  assert.ok(code.includes('aria-expanded={open}'));
  assert.ok(code.includes('aria-disabled={busy || undefined}'));
  // Popover: a non-modal dialog named in catalog copy.
  assert.ok(code.includes('role="dialog"'));
  assert.ok(code.includes("aria-label={t('color.title')}"));
  // Both option groups carry a visible heading that also names the group.
  assert.ok(code.includes("aria-labelledby={symbolicId}"));
  assert.ok(code.includes("aria-labelledby={paletteId}"));
  assert.ok(code.includes('<p className="color-chooser__label" id={symbolicId}>'));
  assert.ok(code.includes('<p className="color-chooser__label" id={paletteId}>'));
  // A swatch is a toggle button: pressed exactly when the drawing stores that index.
  assert.ok(code.includes('aria-label={t(\'color.index\', { index: cell.index })}'));
  assert.ok(code.includes('aria-pressed={cell.index === selected}'));
  assert.ok(code.includes('aria-pressed={isSelectedSymbol(value, symbol)}'));
  // The free-form entry is labelled and describes what it takes.
  assert.ok(code.includes('htmlFor={customId}'));
  assert.ok(code.includes('aria-describedby={customHintId}'));
  assert.ok(code.includes('aria-labelledby={paletteId}'));
});

test('the chooser paints no invented colour and shows no invented label', async () => {
  const code = await readModule('components/ColorChooser.tsx');
  // A symbolic value is a neutral marker, not a colour.
  assert.ok(code.includes("'color-field__trigger', painted ? '' : 'is-symbolic'"));
  assert.ok(code.includes('const painted = paint ?? swatch.hex;'));
  // An unknown token is shown verbatim rather than relabelled from a table.
  assert.ok(code.includes('{key ? t(key) : symbol}'));
  assert.equal(symbolLabelKey('ByLayer'), 'color.symbol.bylayer');
  assert.equal(symbolLabelKey('ByBlock'), 'color.symbol.byblock');
  assert.equal(symbolLabelKey('BySomething'), null);
  // The marker itself is a hatch, not a chosen colour.
  const css = await readModule('style.css');
  assert.ok(css.includes('.color-field__trigger.is-symbolic'));
  assert.ok(css.includes('repeating-linear-gradient'));
});

// --- copy ---------------------------------------------------------------

test('every new label, title and aria string exists in both catalogs', () => {
  const keys = [
    'color.title',
    'color.open',
    'color.close',
    'color.symbolic',
    'color.palette',
    'color.index',
    'color.symbol.bylayer',
    'color.symbol.byblock',
    'color.custom_label',
    'color.custom_hint',
    'color.apply',
  ];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0 && en[key].trim().length > 0, key);
    assert.notEqual(t(key), key, `${key} resolves`);
  }
  // Same placeholders on both sides, and the index is the only one with a slot.
  assert.equal(en['color.index'].includes('{index}'), true);
  assert.equal(zh['color.index'].includes('{index}'), true);
  for (const key of keys.filter(entry => entry !== 'color.index')) {
    assert.equal(/[{}]/.test(zh[key]), false, `${key} has no placeholder`);
    assert.equal(/[{}]/.test(en[key]), false, `${key} has no placeholder`);
  }
});

test('the new modules carry no hardcoded CJK and no direct IPC', async () => {
  for (const name of ['color/color-value.ts', 'color/aci-palette.ts', 'components/ColorChooser.tsx']) {
    const source = await readModule(name);
    assert.equal(/[\u3400-\u9fff\uf900-\ufaff]/.test(source), false, `${name}: no hardcoded CJK`);
  }
  const chooser = await readModule('components/ColorChooser.tsx');
  assert.equal(chooser.includes('invoke('), false, 'the chooser writes through its caller');
  assert.equal(chooser.includes('cad_set_'), false, 'no IPC name lives here');
});

// --- styles -------------------------------------------------------------

test('the popover reuses the chrome surface and is never clipped by the dock', async () => {
  const css = await readModule('style.css');
  const start = css.indexOf('/* Color chooser');
  assert.ok(start > 0, 'the chooser has its own block');
  const block = css.slice(start);
  // Fixed positioning is what lifts it out of the palette body's own scroll.
  assert.ok(block.includes('position: fixed'));
  assert.ok(block.includes('z-index: 50'), 'above the docks, like the shortcut menu');
  assert.ok(block.includes('.color-chooser.is-unplaced { visibility: hidden; }'), 'no unplaced flash');
  for (const token of ['var(--chrome-2)', 'var(--line-soft)', 'var(--focus)', 'var(--field)', 'var(--accent)']) {
    assert.ok(block.includes(token), `the chooser uses ${token}`);
  }
  // The two docked palettes keep their own blocks free of a floating surface.
  const properties = css.slice(css.indexOf('/* Properties palette'), css.indexOf('.start-panel'));
  assert.equal(properties.includes('position: fixed'), false);
  const layers = css.slice(css.indexOf('/* Layers palette'), css.indexOf('/* Navigation rail'));
  assert.equal(layers.includes('position: fixed'), false);
});
