// The chrome's declarative admission tables, split verbatim out of `commands.ts`
// (which owns the functions that read them) so neither module approaches the
// 1000-line code gate. Plain sibling module, not a barrel: importers name it
// directly.
//
// Nothing here decides anything by itself — these are the frozen data rows the
// admission functions consult: the typed-argument grammar, the host capability
// gates, the gesture-only and document-scoped sets, the selection preconditions
// and the selectors the chrome answers without the router. Empty or wrong data
// here can only ever refuse, never silently admit.
import {
  ABOUT_COMMAND,
  CLEANSCREENOFF_COMMAND,
  CLEANSCREENON_COMMAND,
  LAYERCLOSE_COMMAND,
  PROPERTIESCLOSE_COMMAND,
} from './chrome-commands.ts';

/** The `cad_status` slice the admission rules read (all fields are the host's). */
export interface CommandStatusView {
  loading?: boolean;
  documentLabel?: string | null;
  canOpen?: boolean;
  canNew?: boolean;
  canSave?: boolean;
  canPlot?: boolean;
  canFit?: boolean;
  entityCount?: number | null;
  /** The `cad_status.selection` slice the selection rules count (`{count}`). */
  selection?: { readonly count?: unknown } | null;
}

/** A host capability boolean that gates a file-level selector. */
export type CapabilityGate = 'canNew' | 'canSave' | 'canPlot';

// The typed-argument grammar now lives in a sibling module so this table file
// stays under the 1000-line gate. Importers keep reading it from here.
export { ARGUMENT_COMMANDS } from './argument-commands.ts';
export type { ArgumentSpec } from './argument-commands.ts';

/**
 * Host capability gates for the file-level selectors.
 *
 * `NEW`/`SAVE`/`PLOT` and their router aliases are real router commands, not
 * "host-owned" names to be hidden behind a local exclusion list: the host
 * publishes whether it can service them right now in `cad_status`
 * (`canNew`/`canSave`/`canPlot`) and those booleans decide admission. This table
 * maps each name onto the field that gates it; a name absent here has no
 * capability gate. `canNew` is about creating a blank drawing (no document
 * needed); `canSave`/`canPlot` additionally require an open document.
 */
export const CAPABILITY_GATES: Readonly<Record<string, CapabilityGate>> = Object.freeze({
  NEW: 'canNew',
  SAVE: 'canSave',
  SAVEAS: 'canSave',
  QSAVE: 'canSave',
  PLOT: 'canPlot',
  EXPORT: 'canPlot',
  PRINT: 'canPlot',
});

/**
 * Commands the Rust router accepts but a **click cannot perform** because they
 * are canvas drag gestures: `ORBIT` is the 3D primary drag, and its `yaw`/
 * `pitch` come from pointer motion, not from a button payload. Inventing an
 * orbit here would fabricate a camera move, so the affordance stays visible but
 * disabled with a gesture reason that tells the user how to orbit. This is an
 * explicit disable, never a local enable.
 */
export const GESTURE_ONLY_COMMANDS: readonly string[] = Object.freeze(['ORBIT']);

/**
 * Router-admitted selectors that only mean something with a drawing open:
 * `SWITCH2D3D` frames the document's 3D bounds and `PROJECTION` flips the
 * projection of that same camera, so on the start page there is nothing to
 * frame. `MODEL` returns to the Model tab of the open drawing, and `PLAN` returns
 * the open drawing's view to the 2D plan (there is nothing to view without one).
 * `AUDIT` validates the open drawing's stored entities; with no drawing there is
 * nothing to check.
 * They are disabled there with the catalog's own "no drawing is open" sentence
 * instead of a misleading "not supported yet".
 *
 * `MATCHPROP` needs a drawing *and* a selection; it is deliberately **not** in
 * this list — a document is not its only precondition, and the set means exactly
 * "a drawing is the whole requirement" (see `SELECTION_REQUIREMENTS`, which
 * answers the no-document case for selection-gated commands itself).
 */
export const DOCUMENT_SCOPED_COMMANDS: readonly string[] = Object.freeze([
  // CLOSE ends the open drawing; the start page has nothing to close.
  'CLOSE', 'SWITCH2D3D', 'PROJECTION', 'MODEL', 'PLAN', 'AUDIT', 'STATUS', 'LAYTHW', 'HATCHTOBACK',
  'LAYERSTATE', 'LTSCALE', 'LAYWALK',
  // The 2026-11-07 selection/filter/find/visibility batch queries or hides the
  // open drawing's entities, so the start page disables all ten with the
  // catalog's own "no drawing is open" sentence.
  'QSELECT', 'FILTER', 'GETSEL', 'SSX', 'SELECTCOUNT', '-FIND', 'FIND',
  'HIDEOBJECTS', 'ISOLATEOBJECTS', 'UNISOLATEOBJECTS',
  // The 2026-11-07 utility batch reports or edits the open document (or its
  // history), so the start page disables all eight with the catalog's own
  // "no drawing is open" sentence.
  'PURGE', 'RENAME', 'SETVAR', 'TIME', 'DBLIST', 'TREESTAT', 'MREDO', 'MULTIPLE',
  // The 2026-11-07 view batch refreshes, regenerates or restores a view of the
  // open drawing, so the start page disables all of it with the catalog's own
  // "no drawing is open" sentence.
  'REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL', 'VIEW', 'VIEWGO', 'VIEWBACK', 'VIEWFORWARD',
  'VPOINT',
  // The 2026-11-07 layout/viewport batch reads or writes the open drawing's
  // layout table / session space, so the start page disables all nine.
  'LAYOUT', 'LAYOUTMERGE', 'LAYOUTWIZARD', 'VPORTS', 'VPMAX', 'VPMIN', 'VPCLIP',
  'VPSCALE', 'LAYVPI',

  // The 2026-11-07 dimension batch: the style reports and the two no-op reports
  // read the open drawing, so the start page disables them with the same
  // "no drawing is open" sentence. The edit commands are selection-gated and
  // answer the no-document case through their own hint row.
  'DIMSTYLE', '-DIMSTYLE', 'DIMREGEN', 'DIMDISASSOCIATE',

  // The 2026-11-07 annotation/group batch reads or writes the open drawing (the
  // group commands read its entity refs; the rest act on entities), so the start
  // page disables all of them with the same "no drawing is open" sentence.
  'GROUP', '-GROUP', 'UNGROUP', 'GROUPEDIT', 'QLEADER', 'LEADER', 'TEXTFIT',
  'HATCHSETBOUNDARY', 'HATCHSETORIGIN', 'XDLIST',
  // The 2026-11-07 extras batch reads or writes the open drawing / its session
  // view tokens. `REGEN3`, `-PAN` and `-LINETYPE` are alias spellings.
  'SKETCH', 'TRACE', 'MOCORO', 'MSTRETCH', 'VIEWRES', 'VSCURRENT', 'LINETYPE',
  '-LINETYPE', '-PAN', 'REGEN3',

  // The 2026-11-09 pilot batch: the five session-only display tokens
  // (QTEXT/FILL/TFRAMES/REGENAUTO/ISODRAFT) and the five read-only reports
  // (BCOUNT/COUNT/EDITTIME/JULIAN/XLIST) all read or depend on the open drawing,
  // so the start page disables them with the catalog's own "no drawing is open"
  // sentence. None takes a capability gate: no file I/O is involved.
  'QTEXT', 'FILL', 'TFRAMES', 'REGENAUTO', 'ISODRAFT',
  'BCOUNT', 'COUNT', 'EDITTIME', 'JULIAN', 'XLIST',

  // The 2026-11-09 Wave 2 batch: ten session tokens, eight read-only reports and
  // twelve named refusals. All thirty need the open drawing (the tokens are
  // per-drawing session state; the reports read it; the refusals answer from the
  // build's model, which is only meaningful with a document open), so the start
  // page disables them with the catalog's own "no drawing is open" sentence.
  // None takes a capability gate: no file I/O is involved.
  'DRAGMODE', 'APERTURE', 'ISOPLANE', 'HIGHLIGHTNEW', 'TRAYSETTINGS', 'TEXTSCR',
  'GRAPHSCR', 'FINDINCLUDE', 'FINDMATCH', 'REVCLOUDPROPERTIES',
  'SYSVARMONITOR', 'SYSVDLG', 'VTOPTIONS', 'ALIASEDIT', 'WHOHAS', 'ACADINFO',
  'PTYPE', 'SCALELISTEDIT',
  'HYPERLINKOPTIONS', 'INPUTSEARCHOPTIONS', '-INPUTSEARCHOPTIONS', 'SIGVALIDATE',
  'BASE', 'LIMITS', 'VIEWPLOTDETAILS', 'VPLAYER', 'MVIEW', 'PSBSCALE', 'PSTSCALE',
  'MVSETUP',

  // The 2026-11-09 Wave 3 batch: eight reports/session commands and twenty-two
  // named refusals. All thirty need the open drawing (the space switches and
  // reports read its tables; the refusals answer from the build's model, which
  // is only meaningful with a document open), so the start page disables them
  // with the catalog's own "no drawing is open" sentence. None takes a
  // capability gate: no file I/O is involved.
  'MSPACE', 'PSPACE', 'STYLE', '-STYLE', 'MLEADERSTYLE', 'BLOCK?',
  'DIMREASSOC', 'DIMREASSOCIATE',
  'DIMJOGGED', 'DIMINSPECT', '-DIMINSPECT', 'ANNORESET', 'ANNOUPDATE',
  'OBJECTSCALE', '-OBJECTSCALE', 'QLATTACH', 'QLATTACHSET', 'QLDETACHSET',
  'ARRAYCLOSE', 'ARRAYEDIT', 'BEDIT', '-BEDIT', 'BCLOSE', 'BCYCLEORDER',
  'BGRIPSET', 'BESETTINGS', 'BTESTBLOCK', 'LAYERPALETTE', 'CONSTRAINTBAR',
  'CONSTRAINTSETTINGS',

  // The 2026-11-09 Wave 4 batch: the twelve real selection edits and the
  // eighteen named refusals all read the open drawing (the edits read its
  // entities, the refusals answer from the build's model), so the start page
  // disables them with the catalog's own "no drawing is open" sentence. None
  // takes a capability gate: no file I/O is involved.
  'CENTERMARK', 'CENTERLINE', 'NCOPY', 'COPYTOLAYER', '-COPYTOLAYER',
  'CDORDER', 'FLATTEN', 'XPLODE', 'BTRIM', 'BEXTEND', 'CHANGE', 'CHPROP',
  'BSCALE', 'MINSERT', 'BLOCKREPLACE', 'BREPLACE', '-BREPLACE', 'SPLINEDIT',
  'MLEDIT', '-MLEDIT', 'BURST', 'CLIP', 'CLIPIT', 'REGION', 'ADDSELECTED',
  'WBLOCK', '-WBLOCK', 'BSAVE', 'BSAVEAS', 'CHSPACE',

  // The 2026-11-09 Wave 5 batch: the ATTDISP token, the six read-only
  // attribute/xref reports and the twenty-three named refusals all read the
  // open drawing, so the start page disables them with the catalog's own "no
  // drawing is open" sentence. None takes a capability gate: no file I/O is
  // performed (the xref report reads import provenance already in the database).
  'ATTDISP', 'BATTMAN', 'BATTORDER', 'XREF', '-XREF', 'EXTERNALREFERENCES',
  'EXTERNALREFERENCESCLOSE', 'ATTDEF', '-ATTDEF', 'ATTEDIT', '-ATTEDIT',
  'ATTIPEDIT', 'ATTIN', 'ATTOUT', 'ATTEXT', '-ATTEXT', 'EATTEDIT', 'EATTEXT',
  '-EATTEXT', 'ATTREDEF', 'ATTSYNC', 'GATTE', 'XDATA', 'XBIND', '-XBIND',
  'XATTACH', 'ATTACH', '-ATTACH', 'XOPEN', 'CLASSICXREF',

  // The 2026-11-09 Wave 6 batch: the drawing-log token, the three read-only
  // file/plot/export reports and the twenty-four named refusals all read the
  // open drawing, so the start page disables them with the catalog's own "no
  // drawing is open" sentence. None takes a capability gate: no file I/O is
  // performed in cad-app (plot/export remain host-owned).
  'DWGLOG', 'LOGFILEON', 'LOGFILEOFF', 'DWGPROPS', 'DWFFORMAT', 'EXPORTPDF',
  'SAVEALL', 'QNEW', 'REVERT', 'CLOSEALL', 'CLOSEALLOTHER', 'QQUIT', 'QUIT',
  'PROPULATE', 'PLOTTERMANAGER', 'STYLESMANAGER', 'EXPORTSETTINGS', 'PREVIEW',
  'EXPORTLAYOUT', 'PLOTSTAMP', '-PLOTSTAMP', '-PLOT', '-EXPORT', 'ETRANSMIT',
  '-ETRANSMIT', 'ARCHIVE', '-ARCHIVE', 'PUBLISH', '-PUBLISH', 'AUTOPUBLISH',

  // The 2026-11-09 Wave 7 batch: the two read-only visibility-state reports and
  // the twenty-eight dynamic-block/constraint refusals all read the open
  // drawing, so the start page disables them with the catalog's own "no drawing
  // is open" sentence. None takes a capability gate: the reports only read.
  'BVSTATE', '-BVSTATE', 'BACTION', 'BACTIONBAR', 'BACTIONSET', 'BACTIONTOOL',
  'BASSOCIATE', 'BAUTHORPALETTE', 'BAUTHORPALETTECLOSE', 'BCONSTRUCTION',
  'BCPARAMETER', '-BCPARAMETER', 'BPARAMETER', 'BLOOKUPTABLE', 'BTABLE',
  'BVHIDE', 'BVSHOW', 'RESETBLOCK', 'AUTOCONSTRAIN', 'DELCONSTRAINT',
  'DIMCONSTRAINT', 'GEOMCONSTRAINT', 'DCALIGNED', 'DCANGULAR', 'DCCONVERT',
  'DCDIAMETER', 'DCDISPLAY', 'DCFORM', 'DCHORIZONTAL', 'DCLINEAR',

  // The 2026-11-09 Wave 8 batch: two session tokens (ELEV/UCSICON) and eight
  // stable refusals (UCS through RIBBON). They are payload-free and read only
  // the session or answer, so the start page disables them with the catalog's
  // own "no drawing is open" sentence; none takes a capability gate.
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
  '3DSIN',
  'ACISIN',
  'ACISOUT',
  'BMPOUT',
  'JPGOUT',
  'PNGOUT',
  'TIFOUT',
  'WMFIN',
  'WMFOUT',
  'IGESIMPORT',
  'IGESEXPORT',
  'STLOUT',
  'IMPORT',
  'EXPORTTOAUTOCAD',
  'DWGCONVERT',
  'PLT2DWG',
  'DXBIN',
  'APPLOAD',
  'ARX',
  'NETLOAD',
  'VBAIDE',
  'VBALOAD',
  'VBARUN',
  'VBAUNLOAD',
  'SCRIPT',
  'PAGESETUP',
  '-PAGESETUP',
  'PSETUPIN',
  '-PSETUPIN',
  'PLOTSTYLE',
  '-PLOTSTYLE',
  'CONVERTPSTYLES',
  'CONVERTCTB',
  'CONVERTPOLY',
  'CONVERT',
  'TABLE',
  '-TABLE',
  'TABLEDIT',
  'TABLEEXPORT',
  'TABLESTYLE',
  'TINSERT',
  'MATCHCELL',
  'FIELD',
  'RTEXT',
  'RTEDIT',
  'SPELL',
  'SUPERHATCH',
  'ALIGNSPACE',
  'VPSYNC',
  'LAYTRANS',
  'BOX',
  'CONE',
  'CYLINDER',
  'SPHERE',
  'TORUS',
  'WEDGE',
  'PYRAMID',
  'POLYSOLID',
  'EXTRUDE',
  'REVOLVE',
  'SWEEP',
  'LOFT',
  'INTERSECT',
  'UNION',
  'SUBTRACT',
  'SLICE',
  'THICKEN',
  'PRESSPULL',
  'IMPRINT',
  'OFFSETEDGE',
  'CHAMFEREDGE',
  'FILLETEDGE',
  'EDGE',
  'MIRROR3D',
  'ROTATE3D',
  'SURFBLEND',
  'SURFEXTEND',
  'SURFEXTRACTCURVE',
  'SURFFILLET',
  'SURFNETWORK',
  'SURFOFFSET',
  'SURFPATCH',
  'SURFSCULPT',
  'SURFTRIM',
  'SURFUNTRIM',
  'MESH',
  'MESHCAP',
  'MESHCOLLAPSE',
  'MESHCREASE',
  'MESHEXTRUDE',
  'MESHMERGE',
  'MESHOPTIONS',
  'MESHPRIMITIVEOPTIONS',
  'MESHREFINE',
  'MESHSMOOTH',
  'MESHSMOOTHLESS',
  'MESHSMOOTHMORE',
  'MESHSPIN',
  'MESHSPLIT',
  'MESHUNCREASE',
  'IMAGE',
  '-IMAGE',
  'IMAGEATTACH',
  'IMAGECLIP',
  'IMAGEADJUST',
  '-IMAGEADJUST',
  'IMAGEQUALITY',
  'IMAGEAPP',
  'IMAGEASYNCWAIT',
  'IMAGEEDIT',
  'CLASSICIMAGE',
  'PDFATTACH',
  '-PDFATTACH',
  'PDFCLIP',
  'PDFLAYERS',
  'PDFADJUST',
  'PDFIMPORT',
  '-PDFIMPORT',
  'PDFSHXTEXT',
  '-PDFSHXTEXT',
  'DGNATTACH',
  '-DGNATTACH',
  'DGNCLIP',
  'DGNLAYERS',
  'DGNADJUST',
]);

/** One command's precondition on the **current selection**. */
export interface SelectionRequirement {
  /** Minimum selected objects that can still produce a real result. */
  readonly count: number;
  /**
   * The catalog sentence that words the requirement. It is the desktop's own
   * copy of the host rule (`matchprop.needs_two`), so the disabled ribbon hint
   * and the command line's refusal say the same thing — and it is the *only*
   * place the source→targets rule is spelled out for the reader.
   */
  readonly reasonKey: string;
}

/**
 * Commands whose bare name acts on the **current selection**, with the smallest
 * selection that is not a guaranteed refusal.
 *
 * `MATCHPROP` is the selection-based property painter: the first selected entity
 * is the **source**, every other selected entity is a **target**, so one object
 * or none is never a partial success — the host refuses it outright
 * (`matchprop.needs_two`). Offering a live button in that state would be an
 * affordance that can only fail, so the same table disables it with a sentence
 * that says what is missing (`draw.kind.matchprop.hint`) *and* stops the command
 * line from sending a line that cannot succeed.
 *
 * Only *count* preconditions belong here. A command that merely needs "something"
 * selected (ERASE, MOVE, COPY) keeps the neighbours' contract: it stays enabled
 * and the host answers with an explicit refusal, because its own copy
 * (`draw.error.selection_required`) already says so at run time.
 *
 * `JOIN` (2026-10-26) is the second real count precondition: it welds the
 * selection into a single entity (one LINE or one ARC), so one object or none can
 * never be a partial success and the host refuses it outright. Its sentence is the
 * catalog's own `draw.kind.join.hint`, which is also what the disabled ribbon
 * button shows, so the button and the refusal word the same rule once.
 *
 * `LAYISO` needs at least one object — isolating "nothing" would leave every
 * layer hidden and no way back from the canvas — and it reuses the catalog's
 * existing "Select objects first" line (`draw.error.selection_required`): that is
 * already the sentence the host refuses a bare isolate with, so no second copy of
 * the same fact is invented. `LAYUNISO` takes **no** selection (`count: 0`): its
 * entry exists for the "no drawing is open" answer only — the requirement can
 * never be unmet, because nothing is what it restores.
 *
 * `LAYOFF`/`LAYON` (2026-10-26) follow the isolate pair exactly. `LAYOFF` turns
 * **off** every layer the current selection sits on (the current layer excepted),
 * so with nothing selected there is no layer to turn off — the same one-object
 * minimum as `LAYISO`, worded with the same catalog sentence the host refuses a
 * bare `LAYOFF` with (`draw.error.selection_required`). `LAYON` turns every layer
 * back on and takes **no** selection (`count: 0`), so its entry is the
 * "no drawing is open" answer only, exactly like `LAYUNISO`'s.
 */
export const SELECTION_REQUIREMENTS: Readonly<Record<string, SelectionRequirement>> = Object.freeze({
  MATCHPROP: Object.freeze({ count: 2, reasonKey: 'draw.kind.matchprop.hint' }),
  JOIN: Object.freeze({ count: 2, reasonKey: 'draw.kind.join.hint' }),
  // SELECTSIMILAR (AutoCAD 2026) grows the selection from its **geometry kind**,
  // so there must already be one object to call the source. The host refuses an
  // empty selection by name (`selectsimilar.selection_required`); this row makes
  // the disabled button and the command line say the same thing, and it is also
  // the "no drawing is open" answer. Attribute matching is NOT implemented in
  // v1, so the hint must not promise one.
  SELECTSIMILAR: Object.freeze({ count: 1, reasonKey: 'draw.kind.selectsimilar.hint' }),
  // CHAMFER POLYLINE / FILLET POLYLINE (2026-10-27) are the Polyline modes of the
  // two corner commands: each rewrites **exactly one** preselected open polyline
  // in place (`CommandPayload::None`, so nothing is picked interactively). The
  // table states a *minimum* of one — the host is what refuses a selection of two
  // or a non-polyline, by name — and the sentence is this command's own hint, so
  // the disabled button and the refusal describe the same rule once. Zero
  // selected objects can never be a partial success, which is the whole reason
  // the row exists instead of leaving the host to answer at run time.
  'CHAMFER POLYLINE': Object.freeze({ count: 1, reasonKey: 'draw.kind.chamfer_polyline.hint' }),
  'FILLET POLYLINE': Object.freeze({ count: 1, reasonKey: 'draw.kind.fillet_polyline.hint' }),
  // REVCLOUD (AutoCAD 2026) Object mode v1 rewrites **exactly one** preselected
  // closed straight-segment polyline in place (`CommandPayload::None`, so nothing
  // is picked interactively). The table states a minimum of one — the host is
  // what refuses a selection of two or a non-polyline by name — and the sentence
  // is the command's own hint, so the disabled button and the refusal describe
  // the same rule once.
  REVCLOUD: Object.freeze({ count: 1, reasonKey: 'draw.kind.revcloud.hint' }),
  // REVERSE (AutoCAD 2026) reverses at least one selected object; the host
  // refuses an empty selection by name (`reverse.selection`). The table minimum
  // is one and the sentence is the command's own hint.
  REVERSE: Object.freeze({ count: 1, reasonKey: 'draw.kind.reverse.hint' }),
  // PEDIT (AutoCAD 2026) v1 edits **exactly one** preselected model-space
  // polyline (CLOSE / OPEN / REVERSE; no Select polyline prompt, no Multiple).
  // The table states a minimum of one — the host is what refuses a selection of
  // two or a non-polyline by name — and the sentence is the command's own hint,
  // so the disabled button/popover row and the refusal describe the same rule.
  PEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.pedit.hint' }),
  // OOPS (AutoCAD 2026) restores the last ERASE and never reads the selection,
  // so it has no selection precondition. The row exists for the "no drawing is
  // open" answer only, like LAYERP.
  OOPS: Object.freeze({ count: 0, reasonKey: 'draw.kind.oops.hint' }),
  // OVERKILL (AutoCAD 2026) can only remove a duplicate when there are at least
  // two objects to compare, so the local minimum is two. The host refuses an
  // empty/one-object selection by name (`overkill.selection`).
  OVERKILL: Object.freeze({ count: 2, reasonKey: 'draw.kind.overkill.hint' }),
  // HATCHGENERATEBOUNDARY (AutoCAD 2026) needs at least one selected hatch; the
  // host refuses an empty selection and any non-hatch by name
  // (`hatchgenerateboundary.selection` / `.not_hatch`). The table minimum is one
  // and the sentence is the command's own hint, so the disabled button and the
  // refusal describe the same rule once.
  HATCHGENERATEBOUNDARY: Object.freeze({ count: 1, reasonKey: 'draw.kind.hatchgenerateboundary.hint' }),
  // GRADIENT (AutoCAD 2026) fills the current selection, so at least one closed
  // boundary must be selected. The table is a *minimum*: the host refuses a
  // non-closed/non-+Z boundary by name. The sentence is the command's own hint.
  GRADIENT: Object.freeze({ count: 1, reasonKey: 'draw.kind.gradient.hint' }),
  // HATCHEDIT edits exactly one selected hatch; the host refuses other counts,
  // non-hatches and non-addressable refs by name. The table states the minimum
  // one and uses the command's own hint, so the disabled button and the refusal
  // describe the same rule.
  HATCHEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.hatchedit.hint' }),
  // BREAKATPOINT is an **interactive tool** (pick the object, then the break
  // point), not a pre-selection command, so it has no selection precondition.
  // The row exists for the "no drawing is open" answer only.
  BREAKATPOINT: Object.freeze({ count: 0, reasonKey: 'draw.kind.breakatpoint.hint' }),
  LAYISO: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYUNISO: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  LAYOFF: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYON: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  // AREA OBJECT (2026-10-27) measures **exactly one** closed object, so its rule is
  // a count of one: `SelectionRequirement.count` is a *minimum* here (the table's
  // shape does not express an upper bound), which means a selection of two is let
  // through locally and refused by the host **by name** — the same division of
  // labour every other selection rule uses. The sentence is this command's own
  // `measure.kind.area_object.hint`, which states the one-object rule and that the
  // command does not pick objects interactively, so the refusal and the catalogue
  // row cannot describe two different behaviours.
  'AREA OBJECT': Object.freeze({ count: 1, reasonKey: 'measure.kind.area_object.hint' }),
  // LAYERP restores the previous layer-override state, so no selection is ever the
  // reason it cannot run. Like LAYON/LAYUNISO its entry exists for the "no drawing
  // is open" answer only.
  LAYERP: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  // LIST reports the current selection; zero objects is `list.selection`.
  LIST: Object.freeze({ count: 1, reasonKey: 'draw.kind.list.hint' }),
  // MASSPROP (2026-11-07) measures the selected closed planar regions, so at
  // least one object is required. The table is a *minimum* (two objects are let
  // through locally and the host decides coplanarity/kind by name); the sentence
  // is the command's own hint, so the disabled button and the refusal agree.
  MASSPROP: Object.freeze({ count: 1, reasonKey: 'draw.kind.massprop.hint' }),
  // MEASUREGEOM itself takes no selection up front: its tool options capture
  // their own points and only RADIUS reads the selection, so pre-gating the
  // command would disable the tool entries with an unrelated sentence. The host
  // answers the RADIUS case by name (`measuregeom.radius.selection_empty`).
  // FS needs exactly one seed; two or more is refused by the host by name.
  FS: Object.freeze({ count: 1, reasonKey: 'draw.kind.fs.hint' }),
  // The layer-selection run is pre-selection based; LAYTHW needs no selection.
  LAYCUR: Object.freeze({ count: 1, reasonKey: 'draw.kind.laycur.hint' }),
  LAYMCUR: Object.freeze({ count: 1, reasonKey: 'draw.kind.laymcur.hint' }),
  LAYFRZ: Object.freeze({ count: 1, reasonKey: 'draw.kind.layfrz.hint' }),
  LAYTHW: Object.freeze({ count: 0, reasonKey: 'draw.kind.laythw.hint' }),
  LAYLCK: Object.freeze({ count: 1, reasonKey: 'draw.kind.laylck.hint' }),
  LAYULK: Object.freeze({ count: 1, reasonKey: 'draw.kind.layulk.hint' }),
  SETBYLAYER: Object.freeze({ count: 1, reasonKey: 'draw.kind.setbylayer.hint' }),
  // The 2026-11-07 merge/delete run is pre-selection based like the rest of the
  // layer run. LAYMCH needs a destination pick plus at least one object to move,
  // so its minimum is two; LAYMRG/LAYDEL need one object to name the source.
  LAYMCH: Object.freeze({ count: 2, reasonKey: 'draw.kind.laymch.hint' }),
  LAYMRG: Object.freeze({ count: 1, reasonKey: 'draw.kind.laymrg.hint' }),
  LAYDEL: Object.freeze({ count: 1, reasonKey: 'draw.kind.laydel.hint' }),
  // The text run is selection-based (one object minimum); DIMSPACE needs a base
  // dimension plus at least one target.
  TORIENT: Object.freeze({ count: 1, reasonKey: 'draw.kind.torient.hint' }),
  TCASE: Object.freeze({ count: 1, reasonKey: 'draw.kind.tcase.hint' }),
  TXT2MTXT: Object.freeze({ count: 1, reasonKey: 'draw.kind.txt2mtxt.hint' }),
  DIMSPACE: Object.freeze({ count: 2, reasonKey: 'draw.kind.dimspace.hint' }),
  // TEXTTOFRONT (AutoCAD 2026) reads the **current selection** (a declared
  // divergence from AutoCAD's Select objects prompt). With nothing selected the
  // host refuses it by name (`texttofront.selection`), so the table disables the
  // button/line with its own hint: one object is the minimum that can produce a
  // real result. The host independently skips non-text selections with
  // per-entity diagnostics, so the local rule is only a minimum.
  TEXTTOFRONT: Object.freeze({ count: 1, reasonKey: 'draw.kind.texttofront.hint' }),
  // The 2026-11-07 2D text batch. Every one reads the current selection; the
  // host independently refuses a non-text target (whole command or per-entity,
  // as documented) and the empty-selection case by name, so these rows are
  // minimums. TEXTALIGN needs **two** objects: aligning one to itself can never
  // produce a real result. TJUST shares JUSTIFYTEXT's rule (it is the same
  // `CommandId`).
  JUSTIFYTEXT: Object.freeze({ count: 1, reasonKey: 'draw.kind.justifytext.hint' }),
  TJUST: Object.freeze({ count: 1, reasonKey: 'draw.kind.justifytext.hint' }),
  TSCALE: Object.freeze({ count: 1, reasonKey: 'draw.kind.tscale.hint' }),
  TEXTALIGN: Object.freeze({ count: 2, reasonKey: 'draw.kind.textalign.hint' }),
  TCOUNT: Object.freeze({ count: 1, reasonKey: 'draw.kind.tcount.hint' }),
  TCIRCLE: Object.freeze({ count: 1, reasonKey: 'draw.kind.tcircle.hint' }),
  TEXTMASK: Object.freeze({ count: 1, reasonKey: 'draw.kind.textmask.hint' }),
  TEXTUNMASK: Object.freeze({ count: 1, reasonKey: 'draw.kind.textunmask.hint' }),
  // SSX reads the **first selected object** as the template and SELECTCOUNT does
  // the same, so both need one selected object; the host refuses the empty case
  // by name (`ssx.selection_required` / `selectcount.selection_required`).
  SSX: Object.freeze({ count: 1, reasonKey: 'draw.kind.ssx.hint' }),
  SELECTCOUNT: Object.freeze({ count: 1, reasonKey: 'draw.kind.selectcount.hint' }),
  // HIDEOBJECTS hides the selection; ISOLATEOBJECTS keeps it and hides the rest.
  // Both refuse an empty selection, so the row disables the button with its own
  // sentence. UNISOLATEOBJECTS takes no selection (it only clears the hidden
  // set), so its `count: 0` row exists for the no-drawing answer only.
  HIDEOBJECTS: Object.freeze({ count: 1, reasonKey: 'draw.kind.hideobjects.hint' }),
  ISOLATEOBJECTS: Object.freeze({ count: 1, reasonKey: 'draw.kind.isolateobjects.hint' }),
  UNISOLATEOBJECTS: Object.freeze({ count: 0, reasonKey: 'draw.kind.unisolateobjects.hint' }),
  // The 2026-11-07 selection batch. TREX needs a target **and** a boundary, so
  // two objects is the minimum; EXTRIM needs a boundary plus a target; MPEDIT
  // edits one or more polylines; DRAWORDER FRONT/BACK needs one, while
  // ABOVE/BELOW additionally require a reference object (the host refuses the
  // one-object form with `draworder.reference_required`, so the local minimum
  // stays one and the sentence names the rule).
  TREX: Object.freeze({ count: 2, reasonKey: 'draw.kind.trex.hint' }),
  EXTRIM: Object.freeze({ count: 2, reasonKey: 'draw.kind.extrim.hint' }),
  MPEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.mpedit.hint' }),
  DRAWORDER: Object.freeze({ count: 1, reasonKey: 'draw.kind.draworder.hint' }),
  // The 2026-11-07 dimension-editing batch. DIMTEDIT needs **exactly** one
  // dimension (the host refuses anything else); the rest need at least one.
  // DIMBREAK's declared input is one dimension plus crossing objects, so its
  // local minimum is two even though the host refuses the capability by name.
  DIMTEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimtedit.hint' }),
  DIMEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimedit.hint' }),
  DIMOVERRIDE: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimoverride.hint' }),
  QDIM: Object.freeze({ count: 1, reasonKey: 'draw.kind.qdim.hint' }),
  DIMJOGLINE: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimjogline.hint' }),
  DIMBREAK: Object.freeze({ count: 2, reasonKey: 'draw.kind.dimbreak.hint' }),
  // The 2026-11-07 annotation/group batch. GROUP's bare form is a read-only
  // list, so it has no selection row; UNGROUP/GROUPEDIT/QLEADER/LEADER take
  // names/points instead of a selection. The three selection-based commands are
  // minimums (the host refuses the exact counts by name).
  TEXTFIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.textfit.hint' }),
  HATCHSETBOUNDARY: Object.freeze({ count: 2, reasonKey: 'draw.kind.hatchsetboundary.hint' }),
  HATCHSETORIGIN: Object.freeze({ count: 1, reasonKey: 'draw.kind.hatchsetorigin.hint' }),
  XDLIST: Object.freeze({ count: 1, reasonKey: 'draw.kind.xdlist.hint' }),
  // MOCORO transforms the current selection in every mode; the host refuses an
  // empty one by name (`mocoro.selection_required`), and this row reuses the
  // shared "Select objects first" sentence. LINETYPE's `LIST` needs no
  // selection, so it is deliberately absent (its `SET` half is host-refused).
  MOCORO: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
});

/**
 * Selectors the **chrome** answers itself, before the router. They have a real
 * command-line entry (typing the word works) but no reason to reach a router
 * selector: `PROPERTIES` opens the properties window, `LAYER` opens the layers
 * palette, `FONTS` opens the read-only Font Manager, `RESOURCES` opens
 * the References panel, `ABOUT` opens the existing About dialog (the same
 * `requestOpenPopup('about')` path as the File tab's action) and
 * `CLEANSCREENON` sets the chrome's existing clean-screen state (the same state
 * the File tab's `cleanScreen` toggle flips; it sets, never toggles).
 * `CLEANSCREENOFF` clears that same state and is likewise an explicit set, never
 * a toggle. `PROPERTIESCLOSE` closes the properties window (an idempotent
 * success when it is already closed), so it is not gated by layout either.
 * `LAYERCLOSE` does the same for the layers palette: the palette's own close
 * control and the typed word close the one window, and a second close is still
 * a success, so no layout gate applies.
 *
 * They are deliberately admitted regardless of the loaded host vocabulary (and
 * of whether a drawing is open): they touch no CAD state, exactly like the
 * `OPTIONS` dialog the command line already intercepts locally. `LAYER` is not a
 * router name at all — the router knows the layer *operations*
 * (`TOGGLELAYER`/`SETLAYERS`/`RESTORELAYERS`/`SETACTIVELAYER`), which the palette
 * drives through its own dedicated commands (`cad_layers` and friends).
 * `RESOURCES` is a router name (it builds a host-side report), but the References
 * panel reads the references the status already carries, so the UI selector
 * opens that panel and sends nothing: no second source and no fabricated load.
 * `FONTS` owns its own report command (`cad_fonts`), which the panel reads
 * directly; the selector only opens the panel. `ABOUT` sends nothing at all: the
 * host-owned dialog already exists, and the selector only opens it.
 * `CLEANSCREENON` likewise sends nothing: it only sets the chrome's own
 * clean-screen state. `CLEANSCREENOFF` sends nothing either: it only clears that
 * same state. `PROPERTIESCLOSE` sends nothing: it only closes the chrome's own
 * properties window. `LAYERCLOSE` sends nothing either: it only closes the
 * chrome's own layers palette.
 */
export const LOCAL_CHROME_COMMANDS: readonly string[] = Object.freeze([
  'PROPERTIES', 'LAYER', 'FONTS', 'RESOURCES', ABOUT_COMMAND, CLEANSCREENON_COMMAND,
  CLEANSCREENOFF_COMMAND, PROPERTIESCLOSE_COMMAND, LAYERCLOSE_COMMAND,
  'DSETTINGS',
  'QUICKPROPERTIES',
  'CLASSICLAYER',
  'COMMANDLINE',
  'COMMANDLINEHIDE',
]);
