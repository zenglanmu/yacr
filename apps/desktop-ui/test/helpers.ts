// Shared test helpers: the app source (all shipped modules) and the catalogs.
//
// These suites are `.ts` files **by filename only**: the bodies are plain
// JavaScript, there is no type annotation anywhere, and no test is checked by
// `tsc` (`tsconfig.json` includes `src` and `vite-env.d.ts`, never `test`).
// What makes them run is Node's built-in type stripping (`node --test
// test/*.test.ts` on Node >= 22.18/24); TypeScript is *not* in the loop and
// these tests are therefore **not fully typed** — do not read the `.ts`
// extension as a typing guarantee.
//
// The source modules under `src/` are real TypeScript (`src/*.ts`,
// `src/**/*.tsx`). Tests read them as **text** and assert on their structure,
// so the assertions are string scans, not behavioural imports, unless a file
// imports the module directly. Every source path passed to `readModule()` must
// carry the real extension (`.ts` / `.tsx`): the helper does not guess, so a
// stale `x.mjs` / `X.jsx` request fails with ENOENT instead of silently reading
// the wrong file.
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const SRC = new URL('../src/', import.meta.url);

async function collect(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const url = new URL(`${entry.name}${entry.isDirectory() ? '/' : ''}`, dir);
    if (entry.isDirectory()) files.push(...await collect(url));
    else if (/\.(ts|tsx)$/.test(entry.name)) files.push(url);
  }
  return files;
}

/** Concatenated source of every shipped module under `src/` (`.ts` / `.tsx`). */
export async function readAppSource() {
  const files = await collect(SRC);
  const parts = await Promise.all(files.map(async url => `\n/* ${fileURLToPath(url)} */\n${await readFile(url, 'utf8')}`));
  return parts.join('');
}

/** Concatenated source of the rendered region components only. */
export async function readComponentsSource() {
  const files = await collect(new URL('components/', SRC));
  const parts = await Promise.all(files.map(async url => await readFile(url, 'utf8')));
  return parts.join('\n');
}

/**
 * Source text of one file under `src/`, by its src-relative path with the real
 * extension (`'App.tsx'`, `'components/Ribbon.tsx'`, `'cad/cad.ts'`). No extension
 * normalisation: a wrong name fails with ENOENT, which is the honest signal that
 * the file was renamed or never existed.
 */
export function readModule(name) {
  return readFile(new URL(name, SRC), 'utf8');
}

export function readCss() {
  return readFile(new URL('style.css', SRC), 'utf8');
}

export function catalogPath(locale) {
  return new URL(`../../../crates/cad-ui-core/i18n/${locale}.json`, import.meta.url);
}

export async function readCatalog(locale) {
  return JSON.parse(await readFile(catalogPath(locale), 'utf8'));
}

// The router vocabulary the desktop contract tests load.
//
// Mirrors `cad_ui_core::session::admissible_names()` (canonical `CommandId`
// selectors, interaction-only measurement/zoom selectors, then aliases).
// NEW/SAVE/PLOT are present because the router accepts them, and so are their
// aliases — the chrome no longer excludes them: the host's `canNew`/`canSave`/
// `canPlot` booleans decide. `SNAP` is **present**: it is AutoCAD's grid snap
// (F9), a real selector of its own (`CommandId::ToggleSnapGrid`) with the
// `SNAPUNIT` spacing sysvar. It is a different feature from object snap
// (`OSNAP`), which stays the F3 toggle.
export const HOST_NAMES = [
  'OPEN', 'NEW', 'PLOT', 'SAVE', 'CANCEL', 'PAN', 'ZOOM', 'FIT', 'RESETVIEW', 'TOGGLELAYER',
  'SETLAYERS', 'RESTORELAYERS', 'SPACE', 'SELECT', 'SELECTALL', 'MEASURE', 'CONFIRM',
  'CANCELMEASURE', 'UNDO', 'REDO', 'RESOURCES', 'DIAGNOSTICS', 'SWITCHBACKEND', 'SWITCH2D3D',
  'STANDARDVIEW', 'ORBIT', 'PROJECTION', 'MODE', 'LINE', 'CIRCLE', 'ARC', 'PLINE', 'RECTANG', 'ELLIPSE', 'POLYGON', 'POINT', 'SPLINE', 'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE', 'DIMBASELINE', 'DIMCONTINUE', 'MOVE', 'TRIM',
  'EXTEND', 'CHAMFER', 'FILLET', 'ARRAYRECT', 'ARRAYPOLAR', 'ALIGN', 'STRETCH', 'ERASE', 'EXPLODE', 'COPY', 'ROTATE', 'SCALE', 'MIRROR', 'COPYCLIP', 'CUTCLIP', 'PASTECLIP',
  'OFFSET', 'SETACTIVELAYER', 'HOVERPREVIEW', 'GRID', 'SNAP', 'SNAPUNIT', 'OSNAP', 'ORTHO', 'HATCH',
  // MATCHPROP: the selection-based property painter (`CommandId::MatchProp`,
  // `CommandPayload::None` — the selection's first entity is the source). No
  // dedicated IPC; the bare name is the whole input.
  'MATCHPROP',
  // TEXT (ADR 0019 P7a): the single-line text command whose text step travels on
  // `cad_set_tool_text`. Its own suite lives in `text-tool.test.mjs`.
  'TEXT',
  // MTEXT (2026-10-26): the two-corner multiline sibling, whose text step uses
  // the same channel. Its contract tests live beside TEXT's.
  'MTEXT',
  // MLEADER (ADR 0019 P7c): the two-point leader whose text step uses the same
  // channel. Its own suite lives in `mleader-tool.test.mjs`.
  'MLEADER',
  // Interaction-only selectors: one per measurement kind, plus fixed zoom steps.
  // ID (2026-10-26) joined the measurement kinds: one point pick, reported as
  // X/Y/Z plus the snap that produced it (`command.log.point`). `AREA OBJECT`
  // (2026-10-27) is the selection-based area variant: it is a **multi-word
  // canonical selector** (the host canonicalises the whole trimmed line, exactly
  // like the `MEASURE AREA` alias it mirrors), so it stays one entry here and is
  // never split into `AREA` + a typed argument.
  'DIST', 'MEASUREPOLYLINE', 'MEASUREANGLE', 'AREA', 'AREA OBJECT', 'ID', 'ZOOMIN', 'ZOOMOUT',
  // JOIN (2026-10-26): the modify selector that welds the current selection into
  // one polyline. `LAYISO`/`LAYUNISO` (2026-10-26) are the layer isolate pair —
  // router commands, not chrome-local selectors like `LAYER`. `LAYOFF`/`LAYON`
  // (2026-10-26) close that run: `LAYOFF` turns off every layer the selection sits
  // on, `LAYON` is the restore selector that turns them all back on.
  // `LAYERP` (2026-10-27) is a canonical selector of its own ("restore the previous
  // layer-override state"), **not** an alias of `RESTORELAYERS`: the UI therefore
  // writes no alias row for it (the alias table is the host's, and this mirror
  // lists it in the canonical run).
  'JOIN', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON', 'LAYERP',
  // POINT display style sysvars (2026-10-26): `PDMODE` and `PDSIZE` are typed
  // command-line settings with one numeric argument each (like `SNAPUNIT`), not
  // buttons. This mirror is instructed locally, exactly like `SNAPUNIT` before
  // it: the selectors change the open drawing's in-memory point display style
  // (no undo entry, and the exporter writes neither), so no copy may suggest
  // that the value is persisted. Admission tests read this list, never a Rust
  // import.
  'PDMODE', 'PDSIZE',
  // Full-word aliases.
  'EXPORT', 'PRINT', 'SAVEAS', 'QSAVE', 'ZOOM EXTENTS', 'ENTER', 'DBA', 'DCO',
  // AutoCAD's default `acad.pgp` abbreviations, restricted to implemented
  // commands (`cad-ui-core`'s `COMMAND_ALIASES`). `MA` (MATCHPROP) is present
  // because MATCHPROP itself is now implemented.
  'A', 'AL', 'AR', 'C', 'CHA', 'CO', 'CP', 'DAL', 'DAN', 'DCE', 'DDI', 'DLI', 'DRA',
  'E', 'EL', 'EX', 'F', 'GR', 'H', 'L', 'M', 'MA', 'MI', 'O', 'OR', 'OS', 'P', 'PL', 'PO',
  'POL', 'REC', 'RO', 'S', 'SC', 'SPL', 'T', 'TR', 'U', 'X', 'Z',
];
