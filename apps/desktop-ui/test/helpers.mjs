// Shared test helpers: the app source (all shipped modules) and the catalogs.
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const SRC = new URL('../src/', import.meta.url);

async function collect(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const url = new URL(`${entry.name}${entry.isDirectory() ? '/' : ''}`, dir);
    if (entry.isDirectory()) files.push(...await collect(url));
    else if (/\.(jsx|mjs|js)$/.test(entry.name)) files.push(url);
  }
  return files;
}

/** Concatenated source of every shipped module under `src/`. */
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
// `canPlot` booleans decide. `SNAP` is **absent** because the router refuses it
// (AutoCAD's `SNAP` is grid snap, a different feature); `OSNAP` is the
// object-snap toggle the router does accept.
export const HOST_NAMES = [
  'OPEN', 'NEW', 'PLOT', 'SAVE', 'CANCEL', 'PAN', 'ZOOM', 'FIT', 'RESETVIEW', 'TOGGLELAYER',
  'SETLAYERS', 'RESTORELAYERS', 'SPACE', 'SELECT', 'SELECTALL', 'MEASURE', 'CONFIRM',
  'CANCELMEASURE', 'UNDO', 'REDO', 'RESOURCES', 'DIAGNOSTICS', 'SWITCHBACKEND', 'SWITCH2D3D',
  'STANDARDVIEW', 'ORBIT', 'PROJECTION', 'MODE', 'LINE', 'CIRCLE', 'ARC', 'PLINE', 'RECTANG', 'ELLIPSE', 'POLYGON', 'POINT', 'SPLINE', 'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE', 'DIMBASELINE', 'DIMCONTINUE', 'MOVE', 'TRIM',
  'EXTEND', 'CHAMFER', 'FILLET', 'ARRAYRECT', 'ARRAYPOLAR', 'ALIGN', 'STRETCH', 'ERASE', 'EXPLODE', 'COPY', 'ROTATE', 'SCALE', 'MIRROR', 'COPYCLIP', 'CUTCLIP', 'PASTECLIP',
  'OFFSET', 'SETACTIVELAYER', 'HOVERPREVIEW', 'GRID', 'OSNAP', 'ORTHO', 'HATCH',
  // TEXT (ADR 0019 P7a): the single-line text command whose text step travels on
  // `cad_set_tool_text`. Its own suite lives in `text-tool.test.mjs`.
  'TEXT',
  // MLEADER (ADR 0019 P7c): the two-point leader whose text step uses the same
  // channel. Its own suite lives in `mleader-tool.test.mjs`.
  'MLEADER',
  // Interaction-only selectors: one per measurement kind, plus fixed zoom steps.
  'DIST', 'MEASUREPOLYLINE', 'MEASUREANGLE', 'AREA', 'ZOOMIN', 'ZOOMOUT',
  // Full-word aliases.
  'EXPORT', 'PRINT', 'SAVEAS', 'QSAVE', 'ZOOM EXTENTS', 'ENTER', 'DBA', 'DCO',
  // AutoCAD's default `acad.pgp` abbreviations, restricted to implemented
  // commands (`cad-ui-core`'s `COMMAND_ALIASES`). `MA` (MATCHPROP) is absent
  // because the command itself is absent.
  'A', 'AL', 'AR', 'C', 'CHA', 'CO', 'CP', 'DAL', 'DAN', 'DCE', 'DDI', 'DLI', 'DRA',
  'E', 'EL', 'EX', 'F', 'GR', 'H', 'L', 'M', 'MI', 'O', 'OR', 'OS', 'P', 'PL', 'PO',
  'POL', 'REC', 'RO', 'S', 'SC', 'SPL', 'T', 'TR', 'U', 'X', 'Z',
];
