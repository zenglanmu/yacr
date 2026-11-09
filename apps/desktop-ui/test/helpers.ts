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
// `canPlot` booleans decide. CLOSE is the document-scoped "end this drawing"
// selector the doc tab, File tab and command line share (the Rust lane adds it
// to the router table; this mirror never proves the host advertises it). `SNAP`
// is **present**: it is AutoCAD's grid snap
// (F9), a real selector of its own (`CommandId::ToggleSnapGrid`) with the
// `SNAPUNIT` spacing sysvar. It is a different feature from object snap
// (`OSNAP`), which stays the F3 toggle.
export const HOST_NAMES = [
  'OPEN', 'NEW', 'PLOT', 'SAVE', 'CLOSE', 'CANCEL', 'PAN', 'ZOOM', 'FIT', 'RESETVIEW', 'TOGGLELAYER',
  'SETLAYERS', 'RESTORELAYERS', 'SPACE', 'MODEL', 'SELECT', 'SELECTALL', 'SELECTSIMILAR', 'MEASURE', 'CONFIRM',
  'CANCELMEASURE', 'UNDO', 'REDO', 'RESOURCES', 'DIAGNOSTICS',
  // AUDIT (AutoCAD 2026): detect-only drawing audit, a payload-free canonical
  // selector of its own. The Rust lane adds `CommandId::AuditDrawing` to
  // `COMMAND_TABLE`; this mirror stands in for the loaded `cad_commands`
  // vocabulary and never proves the host advertises the name.
  'AUDIT',
  // Inquiry commands: read-only/Viewer-legal; FS is the Express direct-touch grower.
  'LIST',
  'STATUS',
  'FS',
  // MASSPROP (AutoCAD 2026): read-only mass properties of the current selection.
  // MEASUREGEOM (AutoCAD 2026): the inquiry router; the bare name is refused by
  // the host (`measuregeom.needs_option`) and its typed option lines are the
  // only admitted form. Both are canonical selectors, never aliases.
  'MASSPROP',
  'MEASUREGEOM',
  'SWITCHBACKEND', 'SWITCH2D3D',
  // PLAN (AutoCAD 2026): the payload-free "orthographic view of the XY plane"
  // selector. v1 is the top/plan 2D view only (this build has no UCS model, so
  // PLAN's UCS/World/Current options are NOT IMPLEMENTED and take no argument).
  // The Rust lane adds the same name (`CommandId::PlanView`); this mirror stands
  // in for the loaded `cad_commands` vocabulary and never proves the host
  // advertises it.
  'PLAN',
  // The 2026-11-07 layout/viewport batch (canonical selectors plus the
  // `-VPORTS` alias, which `isSupportedCommand` sees only through this mirror).
  'LAYOUT', 'LAYOUTMERGE', 'LAYOUTWIZARD', 'VPORTS', '-VPORTS', 'VPMAX', 'VPMIN', 'VPCLIP',
  'VPSCALE', 'LAYVPI',
  'STANDARDVIEW', 'ORBIT', 'PROJECTION', 'MODE', 'LINE', 'CIRCLE', 'ARC', 'PLINE', 'RECTANG', 'ELLIPSE', 'POLYGON', 'POINT', 'SPLINE', 'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE', 'DIMBASELINE', 'DIMCONTINUE',
  // DIMROTATED (AutoCAD 2026) is a canonical selector of its own (a linear
  // dimension at the session `dimRotation` angle); the Rust lane adds the same
  // name. This mirror stands in for the loaded `cad_commands` vocabulary and
  // never proves the host advertises it.
  'DIMROTATED', 'DIMSPACE', 'MOVE', 'TRIM',
  'EXTEND', 'CHAMFER', 'FILLET', 'ARRAYRECT', 'ARRAYPOLAR', 'ALIGN', 'STRETCH', 'ERASE', 'EXPLODE', 'COPY', 'ROTATE', 'SCALE', 'MIRROR', 'COPYCLIP', 'CUTCLIP', 'PASTECLIP',
  // XLINE / RAY (2026-11-01): the two construction captures. Each is a canonical
  // selector of its own — never an alias of `LINE`, because the capture records a
  // construction type key and the committed geometry is a bounded segment clipped
  // to the model bounds — so admitting one must not admit the other's capture.
  // This mirror stands in for the loaded `cad_commands` vocabulary; the Rust lane
  // adds the same names, and this list never proves the host advertises them.
  'XLINE', 'RAY',
  // Circle definition modes (2026-11-01): `CIRCLE 2P`, `CIRCLE 3P` and `CIRCLE D`
  // are canonical **multi-word** selectors of their own — never aliases of
  // `CIRCLE` — because each capture asks for different picks (two diameter
  // endpoints; three points on the circumference; a centre, then a point whose
  // distance from it is the diameter). The host canonicalises the whole trimmed
  // line, so each stays one entry here. The Rust lane adds the same names in
  // parallel; this mirror never proves the host advertises them.
  'CIRCLE 2P', 'CIRCLE 3P', 'CIRCLE D',
  // Clipboard-with-base-point family (2026-10-27): `COPYBASE`/`CUTBASE` put the
  // selection on the clipboard **with a base point**, so the following `PASTECLIP`
  // is placed relative to it, and `PASTEORIG` pastes at the coordinates the
  // clipboard came from.
  //
  // `PASTEBLOCK` (2026-11-01) is the clipboard's "paste as block" selector: one
  // insertion point, then the host creates an auto-named block plus a reference
  // there and keeps the clipboard. It is its own canonical selector, never an
  // alias of `PASTECLIP`, because the output is a block reference rather than
  // loose geometry. The Rust lane adds the same name (`CommandId::PasteBlock`);
  // this mirror stands in for the loaded `cad_commands` vocabulary because these
  // tests run without a host, and it never proves the host advertises the name.
  'COPYBASE', 'CUTBASE', 'PASTEORIG', 'PASTEBLOCK',
  'OFFSET', 'SETACTIVELAYER', 'HOVERPREVIEW', 'GRID', 'SNAP', 'SNAPUNIT', 'OSNAP', 'ORTHO', 'HATCH',
  // HATCHPOINT: HATCH's one-pick entry point (an internal point inside a closed
  // boundary). It is a canonical selector of its own, not an alias of `HATCH`:
  // the two take different input (a pick vs a pre-selection), so admitting one
  // must never admit the other's capture.
  'HATCHPOINT',
  // BOUNDARY: the other internal-point selector (one pick, then one closed
  // non-associative polyline per discovered loop). Its own canonical name, never
  // an alias of HATCH/HATCHPOINT; `BO`/`BPOLY` are deliberately not admitted.
  'BOUNDARY',
  // HATCHGENERATEBOUNDARY / HATCHTOBACK (AutoCAD 2026) are the hatch utilities:
  // boundary creation is selection-based, to-back is selection-free and
  // document-scoped. Both are canonical selectors of their own, never aliases of
  // HATCH. The Rust lane adds the same names (`CommandId::HatchGenerateBoundary`
  // / `CommandId::HatchToBack`); this mirror stands in for the loaded
  // `cad_commands` vocabulary and never proves the host advertises them.
  'HATCHGENERATEBOUNDARY', 'HATCHTOBACK',
  // TEXTTOFRONT (AutoCAD 2026): selection-based draw order; the current selection
  // is the input and only model-space text/leader/dimension rows are eligible.
  'TEXTTOFRONT',
  // 2026-11-07 selection/filter/find/visibility batch (canonical selectors).
  'QSELECT', 'FILTER', 'GETSEL', 'SSX', 'SELECTCOUNT', '-FIND', 'FIND',
  'HIDEOBJECTS', 'ISOLATEOBJECTS', 'UNISOLATEOBJECTS',
  // The 2026-11-07 view batch: canonical selectors of their own. REDRAW/REGEN and
  // their ALL forms are payload-free; VIEW/VIEWGO/VPOINT take typed arguments, so
  // only the base names are mirrored here (the grammars live in
  // `ARGUMENT_COMMANDS`). VIEWBACK/VIEWFORWARD walk the view stack.
  'REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL', 'VIEW', 'VIEWGO', 'VIEWBACK', 'VIEWFORWARD',
  'VPOINT',
  // GRADIENT / HATCHEDIT (AutoCAD 2026) are the two hatch-family selectors added
  // by this batch. GRADIENT is a payload-free router command over the current
  // selection; HATCHEDIT is one canonical selector whose option is typed
  // (`HATCHEDIT ANGLE|SCALE|SOLID|GRADIENT`), so only the base name is mirrored
  // here — the option words live in `ARGUMENT_COMMANDS.HATCHEDIT`. Neither is an
  // alias of HATCH.
  'GRADIENT', 'HATCHEDIT',
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
  // INSERT (2026-11-01): the point-then-name block reference. One pick sets the
  // insertion point, then the block name travels on the same terminal text step
  // (`cad_set_tool_text`). It is a canonical selector of its own — never an alias
  // of anything — so admitting it admits exactly this capture.
  'INSERT',
  // BLOCK (2026-11-01): the block **definition** command. It starts from the
  // current selection (the host begins it only with a non-empty selection), takes
  // one base-point pick, then the new block name on the same terminal text step
  // (`cad_set_tool_text`). It is a canonical selector of its own — never an alias
  // of INSERT, which consumes a block that already exists — so admitting one
  // admits exactly its own capture.
  'BLOCK',
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
  // The AutoCAD 2026 layer-selection run (LAYCUR/LAYMCUR/LAYFRZ/LAYTHW/
  // LAYLCK/LAYULK/SETBYLAYER). Each is a canonical, payload-free router
  // selector that reads the **current selection** (LAYTHW reads none) rather
  // than a draw capture; the fixed machine cannot ask "Select objects", so the
  // pre-selection is the whole input. This mirror stands in for the loaded
  // `cad_commands` vocabulary and never proves the host advertises them.
  'LAYCUR',
  'LAYMCUR',
  'LAYFRZ',
  'LAYTHW',
  'LAYLCK',
  'LAYULK',
  'SETBYLAYER',
  // The 2026-11-07 layer batch: canonical selectors of their own. LAYMCH/LAYMRG/
  // LAYDEL are selection-based; LAYERSTATE/LTSCALE/LAYWALK take typed arguments;
  // LAYERPMODE reports on a bare name. The `-` spellings are host aliases and are
  // deliberately not listed in this canonical mirror.
  'LAYMCH', 'LAYMRG', 'LAYDEL', 'LAYERSTATE', 'LAYERPMODE', 'LTSCALE', 'LAYWALK',
  // CHAMFER POLYLINE / FILLET POLYLINE (2026-10-27): the Polyline modes of the
  // two corner commands, canonical **multi-word** selectors of their own (like
  // `AREA OBJECT`, the space is part of one name and is never split into a typed
  // argument). They are not aliases of `CHAMFER`/`FILLET`: those keep their
  // two-pick capture, and an alias would route a polyline edit into it. Each
  // takes the current selection (`CommandPayload::None`), so the mirror lists
  // them in the canonical run, exactly as `cad_ui_core::session::COMMAND_TABLE`
  // now does.
  'CHAMFER POLYLINE', 'FILLET POLYLINE',
  // REVCLOUD (AutoCAD 2026) is the **selection-based Object mode**: the one
  // preselected closed straight-segment polyline is the whole input
  // (`CommandPayload::None`), so it is an ordinary canonical selector, not a
  // draw capture. There is no interactive rectangular/polygonal draw mode and no
  // `Arc length` option in v1. The Rust lane adds the same name
  // (`CommandId::RevisionCloud`); this mirror never proves the host advertises it.
  'REVCLOUD',
  // The three single-curve edits (2026-11-01). `BREAK` is a **two-pick** capture
  // (the first pick is the curve *and* the first break point); `DIVIDE` and
  // `LENGTHEN` take no pick at all and act on the current selection, which is why
  // they are pre-selection tools in `cad.ts` and own tool-parameter rows. All
  // three are canonical selectors of their own, never aliases: `BREAK` must not
  // route into `TRIM`'s capture and `LENGTHEN` must not route into `EXTEND`'s,
  // because the picks and the parameters differ.
  'BREAK', 'DIVIDE', 'LENGTHEN',
  // REVERSE (AutoCAD 2026) is the selection-based direction flip: at least one
  // LINE/LWPOLYLINE/SPLINE selected, no pick and no options.
  'REVERSE',
  // PEDIT (AutoCAD 2026) is the selection-based polyline option subset
  // (`PEDIT CLOSE|OPEN|REVERSE`) — an **argument form** of one canonical
  // selector, so only the base name is mirrored here; the three option words
  // live in `ARGUMENT_COMMANDS.PEDIT` and are never a second name list.
  'PEDIT',
  // OOPS (AutoCAD 2026) restores the last ERASE; selection-free, payload-free.
  'OOPS',
  // OVERKILL (AutoCAD 2026) removes exact duplicate selected entities (v1).
  'OVERKILL',
  // BREAKATPOINT (AutoCAD 2026) is the interactive single-point split; its first
  // click is only the object pick (unlike BREAK).
  'BREAKATPOINT',
  // SCALETEXT (2026-11-01) is the selection-based text scaler: one or more
  // Text/MTEXT objects plus one positive factor (`scaleTextFactor`), no picks. It
  // is a canonical selector of its own, never an alias of `SCALE` (which picks a
  // base point and acts on arbitrary geometry) — the two take different inputs
  // and different parameters.
  'SCALETEXT',
  // The text run: bare TORIENT is Most Readable, `TORIENT <angle>` its argument;
  // bare TCASE is Title, `TCASE <mode>` its argument; TXT2MTXT is name-only.
  'TORIENT', 'TCASE', 'TXT2MTXT',
  // The 2026-11-07 2D text batch: five argument-taking selectors and the two
  // selection-based mask commands. `TJUST`, `MTEDIT` and `-MTEXT` are the host's
  // aliases of `JUSTIFYTEXT`/`TEXTEDIT`/`MTEXT` (`COMMAND_ALIASES`), so the
  // mirrored `admissible_names()` includes them too. This list stands in for the
  // loaded `cad_commands` vocabulary and never proves the host advertises them.
  'JUSTIFYTEXT', 'TJUST', 'TSCALE', 'TEXTALIGN', 'TCOUNT', 'TCIRCLE',
  'TEXTMASK', 'TEXTUNMASK', 'MTEDIT', '-MTEXT',
  // The 2026-11-07 utility batch: PURGE/RENAME/SETVAR/MREDO/MULTIPLE are typed
  // argument selectors; TIME/DBLIST/TREESTAT are payload-free reports. The two
  // `-` spellings are host aliases of PURGE/RENAME.
  'PURGE', '-PURGE', 'RENAME', '-RENAME', 'SETVAR', 'TIME', 'DBLIST', 'TREESTAT',
  'MREDO', 'MULTIPLE',
  // The 2026-11-07 draw/modify batch: DONUT/SOLID/HELIX/MLINE/MLSTYLE are typed
  // creation selectors; TREX/EXTRIM/MPEDIT/DRAWORDER read the selection. The
  // `-OVERKILL` spelling is a host alias of OVERKILL.
  'DONUT', 'SOLID', 'HELIX', 'MLINE', 'MLSTYLE', 'TREX', 'EXTRIM', 'MPEDIT',
  'DRAWORDER', '-OVERKILL',
  // The 2026-11-07 dimension-editing batch: DIMSTYLE/-DIMSTYLE and the four
  // typed edit forms; DIMREGEN/DIMDISASSOCIATE/QDIM/DIMBREAK are canonical too.
  'DIMSTYLE', '-DIMSTYLE', 'DIMTEDIT', 'DIMEDIT', 'DIMOVERRIDE', 'DIMREGEN',
  'DIMDISASSOCIATE', 'QDIM', 'DIMJOGLINE', 'DIMBREAK',
  // The 2026-11-07 annotation/group batch. GROUP/UNGROUP/GROUPEDIT are
  // session-only command-line forms; QLEADER/LEADER create a MULTILEADER;
  // TEXTFIT and the hatch/xdata commands read the selection. `-GROUP` is the
  // host alias spelling.
  'GROUP', '-GROUP', 'UNGROUP', 'GROUPEDIT', 'QLEADER', 'LEADER', 'TEXTFIT',
  'HATCHSETBOUNDARY', 'HATCHSETORIGIN', 'XDLIST',
  // The 2026-11-07 extras batch: SKETCH/TRACE/MOCORO/MSTRETCH/VIEWRES/VSCURRENT/
  // LINETYPE are canonical selectors; `REGEN3`/`-PAN`/`-LINETYPE` are the host
  // alias spellings (the `-` forms normalize onto PAN/LINETYPE).
  'SKETCH', 'TRACE', 'MOCORO', 'MSTRETCH', 'VIEWRES', 'VSCURRENT', 'LINETYPE',
  'REGEN3', '-PAN', '-LINETYPE',
  // WIPEOUT is the open-ended polygon mask; POLYGON CIRCUMSCRIBED is the
  // tangent-ring variant of POLYGON (its own multi-word selector).
  'WIPEOUT', 'POLYGON CIRCUMSCRIBED',
  // POINT display style sysvars (2026-10-26): `PDMODE` and `PDSIZE` are typed
  // command-line settings with one numeric argument each (like `SNAPUNIT`), not
  // buttons. This mirror is instructed locally, exactly like `SNAPUNIT` before
  // it: the selectors change the open drawing's in-memory point display style
  // (no undo entry, and the exporter writes neither), so no copy may suggest
  // that the value is persisted. Admission tests read this list, never a Rust
  // import.
  'PDMODE', 'PDSIZE',
  // The 2026-11-09 pilot batch: five session-only display tokens with no renderer
  // effect (QTEXT/FILL/TFRAMES/REGENAUTO/ISODRAFT) and five read-only reports
  // (BCOUNT/COUNT/EDITTIME/JULIAN/XLIST). All are payload-free canonical
  // selectors, never aliases. This mirror stands in for the loaded
  // `cad_commands` vocabulary and never proves the host advertises them.
  'QTEXT', 'FILL', 'TFRAMES', 'REGENAUTO', 'ISODRAFT',
  'BCOUNT', 'COUNT', 'EDITTIME', 'JULIAN', 'XLIST',
  // The 2026-11-09 Wave 2 batch: thirty payload-free canonical selectors — ten
  // session tokens (no renderer effect), eight read-only reports and twelve
  // named refusals. `-INPUTSEARCHOPTIONS` is its own canonical selector here,
  // not an alias spelling.
  'DRAGMODE', 'APERTURE', 'ISOPLANE', 'HIGHLIGHTNEW', 'TRAYSETTINGS', 'TEXTSCR',
  'GRAPHSCR', 'FINDINCLUDE', 'FINDMATCH', 'REVCLOUDPROPERTIES',
  'SYSVARMONITOR', 'SYSVDLG', 'VTOPTIONS', 'ALIASEDIT', 'WHOHAS', 'ACADINFO',
  'PTYPE', 'SCALELISTEDIT',
  'HYPERLINKOPTIONS', 'INPUTSEARCHOPTIONS', '-INPUTSEARCHOPTIONS', 'SIGVALIDATE',
  'BASE', 'LIMITS', 'VIEWPLOTDETAILS', 'VPLAYER', 'MVIEW', 'PSBSCALE', 'PSTSCALE',
  'MVSETUP',
  // The 2026-11-09 Wave 3 batch: thirty payload-free canonical selectors — eight
  // reports/session commands (two space switches, four table reports, two
  // non-associative no-ops) and twenty-two named refusals. `-STYLE`, `-BEDIT`,
  // `-DIMINSPECT` and `-OBJECTSCALE` are their own canonical selectors here,
  // not aliases.
  'MSPACE', 'PSPACE', 'STYLE', '-STYLE', 'MLEADERSTYLE', 'BLOCK?',
  'DIMREASSOC', 'DIMREASSOCIATE', 'DIMJOGGED', 'DIMINSPECT', '-DIMINSPECT',
  'ANNORESET', 'ANNOUPDATE', 'OBJECTSCALE', '-OBJECTSCALE', 'QLATTACH',
  'QLATTACHSET', 'QLDETACHSET', 'ARRAYCLOSE', 'ARRAYEDIT', 'BEDIT', '-BEDIT',
  'BCLOSE', 'BCYCLEORDER', 'BGRIPSET', 'BESETTINGS', 'BTESTBLOCK',
  'LAYERPALETTE', 'CONSTRAINTBAR', 'CONSTRAINTSETTINGS',
  // The 2026-11-09 Wave 4 batch: twelve real selection edits (Work-only) plus
  // eighteen named refusals. All thirty are canonical selectors of their own;
  // `-COPYTOLAYER`, `-BREPLACE`, `-MLEDIT` and `-WBLOCK` are not aliases.
  'CENTERMARK', 'CENTERLINE', 'NCOPY', 'COPYTOLAYER', '-COPYTOLAYER',
  'CDORDER', 'FLATTEN', 'XPLODE', 'BTRIM', 'BEXTEND', 'CHANGE', 'CHPROP',
  'BSCALE', 'MINSERT', 'BLOCKREPLACE', 'BREPLACE', '-BREPLACE', 'SPLINEDIT',
  'MLEDIT', '-MLEDIT', 'BURST', 'CLIP', 'CLIPIT', 'REGION', 'ADDSELECTED',
  'WBLOCK', '-WBLOCK', 'BSAVE', 'BSAVEAS', 'CHSPACE',
  // The 2026-11-09 Wave 5 batch: the ATTDISP token, six read-only attribute/xref
  // reports and twenty-three named refusals. All thirty are canonical selectors
  // of their own; `-XREF`, `-ATTDEF`, `-ATTEDIT`, `-ATTEXT`, `-EATTEXT`,
  // `-XBIND` and `-ATTACH` are not aliases.
  'ATTDISP', 'BATTMAN', 'BATTORDER', 'XREF', '-XREF', 'EXTERNALREFERENCES',
  'EXTERNALREFERENCESCLOSE', 'ATTDEF', '-ATTDEF', 'ATTEDIT', '-ATTEDIT',
  'ATTIPEDIT', 'ATTIN', 'ATTOUT', 'ATTEXT', '-ATTEXT', 'EATTEDIT', 'EATTEXT',
  '-EATTEXT', 'ATTREDEF', 'ATTSYNC', 'GATTE', 'XDATA', 'XBIND', '-XBIND',
  'XATTACH', 'ATTACH', '-ATTACH', 'XOPEN', 'CLASSICXREF',
  // The 2026-11-09 Wave 6 batch: the drawing-log token, three read-only
  // file/plot/export reports and twenty-four named refusals. All thirty are
  // canonical selectors of their own; `-PLOTSTAMP`, `-PLOT`, `-EXPORT`,
  // `-ETRANSMIT`, `-ARCHIVE` and `-PUBLISH` are not aliases.
  'DWGLOG', 'LOGFILEON', 'LOGFILEOFF', 'DWGPROPS', 'DWFFORMAT', 'EXPORTPDF',
  'SAVEALL', 'QNEW', 'REVERT', 'CLOSEALL', 'CLOSEALLOTHER', 'QQUIT', 'QUIT',
  'PROPULATE', 'PLOTTERMANAGER', 'STYLESMANAGER', 'EXPORTSETTINGS', 'PREVIEW',
  'EXPORTLAYOUT', 'PLOTSTAMP', '-PLOTSTAMP', '-PLOT', '-EXPORT', 'ETRANSMIT',
  '-ETRANSMIT', 'ARCHIVE', '-ARCHIVE', 'PUBLISH', '-PUBLISH', 'AUTOPUBLISH',
  // The 2026-11-09 Wave 7 batch: two read-only dynamic-block visibility reports
  // and twenty-eight named refusals. All thirty are canonical selectors of their
  // own; `-BVSTATE` and `-BCPARAMETER` are not aliases.
  'BVSTATE', '-BVSTATE', 'BACTION', 'BACTIONBAR', 'BACTIONSET', 'BACTIONTOOL',
  'BASSOCIATE', 'BAUTHORPALETTE', 'BAUTHORPALETTECLOSE', 'BCONSTRUCTION',
  'BCPARAMETER', '-BCPARAMETER', 'BPARAMETER', 'BLOOKUPTABLE', 'BTABLE',
  'BVHIDE', 'BVSHOW', 'RESETBLOCK', 'AUTOCONSTRAIN', 'DELCONSTRAINT',
  'DIMCONSTRAINT', 'GEOMCONSTRAINT', 'DCALIGNED', 'DCANGULAR', 'DCCONVERT',
  'DCDIAMETER', 'DCDISPLAY', 'DCFORM', 'DCHORIZONTAL', 'DCLINEAR',
  // The 2026-11-09 Wave 8 batch: two session tokens and eight stable named
  // refusals. All ten are canonical selectors of their own.
  'ELEV', 'UCSICON', 'UCS', 'UCSMAN', 'TABLET', 'TASKBAR', 'REINIT',
  'SYSWINDOWS', 'MENU', 'RIBBON',
  'ARRAY',
  '-ARRAY',
  'ARRAYCLASSIC',
  'ARCTEXT',
  'BLEND',
  'BREAKLINE',
  'MLEADERALIGN',
  'MLEADEREDIT',
  'MLEADERCOLLECT',
  'TOLERANCE',
  'DIM',
  'DIMARC',
  'CAL',
  '-BLOCK',
  '-BOUNDARY',
  '-INSERT',
  '-HATCH',
  '-HATCHEDIT',
  'EXOFFSET',
  'DIMEX',
  'DIMIM',
  'SPACETRANS',
  'TXTEXP',
  'UPDATEFIELD',
  'UNITS',
  '-UNITS',
  '-LAYER',
  'LAYERSTATESAVE',
  'NEWVIEW',
  '-SCALELISTEDIT',
  'LWEIGHT',
  '-LWEIGHT',
  'COLOR',
  '-COLOR',
  'HELP',
  'CLASSICINSERT',
  'CLASSICGROUP',
  'FILETAB',
  'FILETABCLOSE',
  'HIDEPALETTES',
  'SHOWPALETTES',
  'RIBBONCLOSE',
  'GOTOSTART',
  'COPYHIST',
  'TRANSPARENCY',
  '3DALIGN',
  '3DARRAY',
  '3DCLIP',
  '3DCORBIT',
  '3DDISTANCE',
  '3DDWF',
  '3DEDITBAR',
  '3DFACE',
  '3DFLY',
  '3DFORBIT',
  '3DMESH',
  '3DMOVE',
  '3DORBIT',
  '3DORBITCTR',
  '-3DOSNAP',
  '3DOSNAP',
  '3DPAN',
  '3DPOLY',
  '3DPRINT',
  '3DPRINTSERVICE',
  '3DROTATE',
  '3DSCALE',
  '3DSWIVEL',
  '3DWALK',
  '3DZOOM',
  // Full-word aliases.
  'EXPORT', 'PRINT', 'SAVEAS', 'QSAVE', 'ZOOM EXTENTS', 'ENTER', 'DBA', 'DCO',
  // AutoCAD's default `acad.pgp` abbreviations, restricted to implemented
  // commands (`cad-ui-core`'s `COMMAND_ALIASES`). `MA` (MATCHPROP) is present
  // because MATCHPROP itself is now implemented.
  'A', 'AL', 'AR', 'C', 'CHA', 'CO', 'CP', 'DAL', 'DAN', 'DCE', 'DDI', 'DLI', 'DRA',
  'E', 'EL', 'EX', 'F', 'GR', 'H', 'L', 'M', 'MA', 'MI', 'O', 'OR', 'OS', 'P', 'PL', 'PO',
  'POL', 'REC', 'RO', 'S', 'SC', 'SPL', 'T', 'TR', 'U', 'X', 'Z',
];
