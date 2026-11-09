// Contract test for the 3D view affordances: the dedicated typed
// `cad_standard_view` channel, the View tab's standard-view items and the honest
// disable states (document-scoped toggles, gesture-only orbit).
//
// The point of this suite is the distinction the product makes: `{accepted:true}`
// is **queued**, never applied, and a click can never fabricate a camera move.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  NO_DOCUMENT_KEY,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { DOCUMENT_SCOPED_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { createTranslator } from '../src/i18n.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import {
  requestStandardView,
  STANDARD_VIEWS,
  STANDARD_VIEW_UNAVAILABLE_KEY,
  standardViewAvailable,
  standardViewFailure,
  standardViewName,
  standardViewUnavailableReasonKey,
  submitStandardView,
} from '../src/cad/view3d.ts';
import { panelRefusalKey } from '../src/layout/mobile-ribbon.ts';
import { readCatalog, readModule } from './helpers.ts';

// Only the names this suite exercises. `ORBIT` is here because the router does
// know it — the chrome still refuses a click, which is the point of the gesture
// list, not a missing vocabulary entry.
setCommandCatalog(['CLOSE', 'SWITCH2D3D', 'PROJECTION', 'MODEL', 'PLAN', 'AUDIT', 'STATUS', 'LAYTHW', 'HATCHTOBACK', 'LAYERSTATE', 'LTSCALE', 'LAYWALK', 'QSELECT', 'FILTER', 'GETSEL', 'SSX', 'SELECTCOUNT', '-FIND', 'FIND', 'HIDEOBJECTS', 'ISOLATEOBJECTS', 'UNISOLATEOBJECTS', 'PURGE', 'RENAME', 'SETVAR', 'TIME', 'DBLIST', 'TREESTAT', 'MREDO', 'MULTIPLE', 'REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL', 'VIEW', 'VIEWGO', 'VIEWBACK', 'VIEWFORWARD', 'VPOINT', 'LAYOUT', 'LAYOUTMERGE', 'LAYOUTWIZARD', 'VPORTS', 'VPMAX', 'VPMIN', 'VPCLIP', 'VPSCALE', 'LAYVPI', 'DIMSTYLE', '-DIMSTYLE', 'DIMREGEN', 'DIMDISASSOCIATE', 'GROUP', '-GROUP', 'UNGROUP', 'GROUPEDIT', 'QLEADER', 'LEADER', 'TEXTFIT', 'HATCHSETBOUNDARY', 'HATCHSETORIGIN', 'XDLIST', 'SKETCH', 'TRACE', 'MOCORO', 'MSTRETCH', 'VIEWRES', 'VSCURRENT', 'LINETYPE', '-LINETYPE', '-PAN', 'REGEN3', 'QTEXT', 'FILL', 'TFRAMES', 'REGENAUTO', 'ISODRAFT', 'BCOUNT', 'COUNT', 'EDITTIME', 'JULIAN', 'XLIST', 'DRAGMODE', 'APERTURE', 'ISOPLANE', 'HIGHLIGHTNEW', 'TRAYSETTINGS', 'TEXTSCR', 'GRAPHSCR', 'FINDINCLUDE', 'FINDMATCH', 'REVCLOUDPROPERTIES', 'SYSVARMONITOR', 'SYSVDLG', 'VTOPTIONS', 'ALIASEDIT', 'WHOHAS', 'ACADINFO', 'PTYPE', 'SCALELISTEDIT', 'HYPERLINKOPTIONS', 'INPUTSEARCHOPTIONS', '-INPUTSEARCHOPTIONS', 'SIGVALIDATE', 'BASE', 'LIMITS', 'VIEWPLOTDETAILS', 'VPLAYER', 'MVIEW', 'PSBSCALE', 'PSTSCALE', 'MVSETUP', 'MSPACE', 'PSPACE', 'STYLE', '-STYLE', 'MLEADERSTYLE', 'BLOCK?', 'DIMREASSOC', 'DIMREASSOCIATE', 'DIMJOGGED', 'DIMINSPECT', '-DIMINSPECT', 'ANNORESET', 'ANNOUPDATE', 'OBJECTSCALE', '-OBJECTSCALE', 'QLATTACH', 'QLATTACHSET', 'QLDETACHSET', 'ARRAYCLOSE', 'ARRAYEDIT', 'BEDIT', '-BEDIT', 'BCLOSE', 'BCYCLEORDER', 'BGRIPSET', 'BESETTINGS', 'BTESTBLOCK', 'LAYERPALETTE', 'CONSTRAINTBAR', 'CONSTRAINTSETTINGS', 'CENTERMARK', 'CENTERLINE', 'NCOPY', 'COPYTOLAYER', '-COPYTOLAYER', 'CDORDER', 'FLATTEN', 'XPLODE', 'BTRIM', 'BEXTEND', 'CHANGE', 'CHPROP', 'BSCALE', 'MINSERT', 'BLOCKREPLACE', 'BREPLACE', '-BREPLACE', 'SPLINEDIT', 'MLEDIT', '-MLEDIT', 'BURST', 'CLIP', 'CLIPIT', 'REGION', 'ADDSELECTED', 'WBLOCK', '-WBLOCK', 'BSAVE', 'BSAVEAS', 'CHSPACE', 'ATTDISP', 'BATTMAN', 'BATTORDER', 'XREF', '-XREF', 'EXTERNALREFERENCES', 'EXTERNALREFERENCESCLOSE', 'ATTDEF', '-ATTDEF', 'ATTEDIT', '-ATTEDIT', 'ATTIPEDIT', 'ATTIN', 'ATTOUT', 'ATTEXT', '-ATTEXT', 'EATTEDIT', 'EATTEXT', '-EATTEXT', 'ATTREDEF', 'ATTSYNC', 'GATTE', 'XDATA', 'XBIND', '-XBIND', 'XATTACH', 'ATTACH', '-ATTACH', 'XOPEN', 'CLASSICXREF', 'DWGLOG', 'LOGFILEON', 'LOGFILEOFF', 'DWGPROPS', 'DWFFORMAT', 'EXPORTPDF', 'SAVEALL', 'QNEW', 'REVERT', 'CLOSEALL', 'CLOSEALLOTHER', 'QQUIT', 'QUIT', 'PROPULATE', 'PLOTTERMANAGER', 'STYLESMANAGER', 'EXPORTSETTINGS', 'PREVIEW', 'EXPORTLAYOUT', 'PLOTSTAMP', '-PLOTSTAMP', '-PLOT', '-EXPORT', 'ETRANSMIT', '-ETRANSMIT', 'ARCHIVE', '-ARCHIVE', 'PUBLISH', '-PUBLISH', 'AUTOPUBLISH', 'BVSTATE', '-BVSTATE', 'BACTION', 'BACTIONBAR', 'BACTIONSET', 'BACTIONTOOL', 'BASSOCIATE', 'BAUTHORPALETTE', 'BAUTHORPALETTECLOSE', 'BCONSTRUCTION', 'BCPARAMETER', '-BCPARAMETER', 'BPARAMETER', 'BLOOKUPTABLE', 'BTABLE', 'BVHIDE', 'BVSHOW', 'RESETBLOCK', 'AUTOCONSTRAIN', 'DELCONSTRAINT', 'DIMCONSTRAINT', 'GEOMCONSTRAINT', 'DCALIGNED', 'DCANGULAR', 'DCCONVERT', 'DCDIAMETER', 'DCDISPLAY', 'DCFORM', 'DCHORIZONTAL', 'DCLINEAR', 'ELEV', 'UCSICON', 'UCS', 'UCSMAN', 'TABLET', 'TASKBAR', 'REINIT', 'SYSWINDOWS', 'MENU', 'RIBBON', 'ORBIT', 'FIT', 'PAN', 'ZOOMIN', 'ZOOMOUT', 'ARRAY', '-ARRAY', 'ARRAYCLASSIC', 'ARCTEXT', 'BLEND', 'BREAKLINE', 'MLEADERALIGN', 'MLEADEREDIT', 'MLEADERCOLLECT', 'TOLERANCE', 'DIM', 'DIMARC', 'CAL', '-BLOCK', '-BOUNDARY', '-INSERT', '-HATCH', '-HATCHEDIT', 'EXOFFSET', 'DIMEX', 'DIMIM', 'SPACETRANS', 'TXTEXP', 'UPDATEFIELD', 'UNITS', '-UNITS', '-LAYER', 'LAYERSTATESAVE', 'NEWVIEW', '-SCALELISTEDIT', 'LWEIGHT', '-LWEIGHT', 'COLOR', '-COLOR', 'HELP', 'CLASSICINSERT', 'CLASSICGROUP', 'FILETAB', 'FILETABCLOSE', 'HIDEPALETTES', 'SHOWPALETTES', 'RIBBONCLOSE', 'GOTOSTART', 'COPYHIST', 'TRANSPARENCY', '3DALIGN', '3DARRAY', '3DCLIP', '3DCORBIT', '3DDISTANCE', '3DDWF', '3DEDITBAR', '3DFACE', '3DFLY', '3DFORBIT', '3DMESH', '3DMOVE', '3DORBIT', '3DORBITCTR', '-3DOSNAP', '3DOSNAP', '3DPAN', '3DPOLY', '3DPRINT', '3DPRINTSERVICE', '3DROTATE', '3DSCALE', '3DSWIVEL', '3DWALK', '3DZOOM', '3DSIN', 'ACISIN', 'ACISOUT', 'BMPOUT', 'JPGOUT', 'PNGOUT', 'TIFOUT', 'WMFIN', 'WMFOUT', 'IGESIMPORT', 'IGESEXPORT', 'STLOUT', 'IMPORT', 'EXPORTTOAUTOCAD', 'DWGCONVERT', 'PLT2DWG', 'DXBIN', 'APPLOAD', 'ARX', 'NETLOAD', 'VBAIDE', 'VBALOAD', 'VBARUN', 'VBAUNLOAD', 'SCRIPT']);


const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');
const loaded = { documentLabel: 'fixture.dxf', entityCount: 12, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false };
const EXPECTED_VIEWS = ['top', 'bottom', 'front', 'back', 'left', 'right', 'isometric'];

function viewTab() {
  const tab = RIBBON_TABS.find(candidate => candidate.id === 'view');
  assert.ok(tab, 'the View tab must exist');
  return tab;
}

test('the View tab exposes the 3D panel, the orbit hint and all seven standard views', async () => {
  const tab = viewTab();
  const mode = tab.panels.find(panel => panel.id === 'view3d');
  assert.ok(mode, 'the View tab must carry the 3D panel');
  // The 3D affordances lead the tab: mode toggle, projection toggle, orbit hint.
  assert.deepEqual(mode.items.map(item => item.id), ['SWITCH2D3D', 'PROJECTION', 'ORBIT']);
  const toggle = mode.items.find(item => item.id === 'SWITCH2D3D');
  assert.equal(toggle.command, 'SWITCH2D3D');
  assert.equal(toggle.labelKey, 'ribbon.command.view_switch2d3d');
  assert.equal(toggle.large, true);
  assert.equal(toggle.standardView, undefined, 'the 2D/3D toggle carries no payload');
  const projection = mode.items.find(item => item.id === 'PROJECTION');
  assert.equal(projection.command, 'PROJECTION');
  assert.equal(projection.labelKey, 'view.projection');
  assert.equal(projection.standardView, undefined, 'PROJECTION is payload-free');
  assert.equal(typeof zh[mode.titleKey], 'string', mode.titleKey);
  assert.equal(typeof en[mode.titleKey], 'string', mode.titleKey);

  const standard = tab.panels.find(panel => panel.id === 'standard');
  assert.ok(standard, 'the View tab must carry the standard-view panel');
  // The seven payload-carrying views come first and keep their order. `PLAN` is
  // appended after them as a payload-free `cad_command` row (its own contract is
  // `test/plan-command.test.ts`), so the standard-view assertions filter on the
  // `standardView` token rather than assuming the panel holds only views.
  const standardViewItems = standard.items.filter(item => item.standardView !== undefined);
  assert.deepEqual(standardViewItems.map(item => item.standardView), EXPECTED_VIEWS);
  // The panel follows the shared data table, so a renamed host key cannot drift.
  assert.deepEqual(standardViewItems, STANDARD_VIEWS.map(entry => ({ ...entry })));
  const icons = await readModule('icons.tsx');
  for (const item of standardViewItems) {
    // A payload-carrying view is never also a `cad_command` name: that funnel
    // refuses a payload variant, so offering it as one would be a broken button.
    assert.equal(item.command, undefined, `${item.id} must not carry a command`);
    assert.equal(item.action, undefined, `${item.id} is not a shell action`);
    assert.equal(item.large, undefined, `${item.id} stays a small view button`);
    // Both catalogs word every label, and every glyph really exists (a missing
    // name would silently fall back to the `dot` icon).
    for (const key of [item.labelKey]) {
      assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
      assert.equal(typeof en[key], 'string', `en missing ${key}`);
    }
    assert.ok(icons.includes(`${item.icon}:`), `icons.jsx has no ${item.icon} glyph`);
  }
  assert.equal(typeof zh[standard.titleKey], 'string', standard.titleKey);
  assert.equal(typeof en[standard.titleKey], 'string', standard.titleKey);
  // The seven view labels plus the appended PLAN label are distinct, never one
  // word repeated.
  assert.equal(new Set(standard.items.map(item => item.labelKey)).size, 8);
});

test('a standard-view click sends exactly one cad_standard_view with its own name', async () => {
  const standard = viewTab().panels.find(panel => panel.id === 'standard');
  // `PLAN` rides `cad_command`, not this typed channel, so only the seven
  // payload-carrying rows are exercised here.
  for (const item of standard.items.filter(entry => entry.standardView !== undefined)) {
    const calls = [];
    const reported = [];
    const result = await submitStandardView({
      invoke: async (name, args) => { calls.push([name, args]); return { accepted: true }; },
      view: item.standardView,
      t,
      report: message => reported.push(message),
    });
    assert.equal(result.kind, 'accepted', item.standardView);
    assert.equal(result.view, item.standardView);
    // One IPC, the typed channel, and the host's own machine key as the payload.
    assert.deepEqual(calls, [['cad_standard_view', { view: item.standardView }]], item.standardView);
    // Admission is not completion: the ribbon reports nothing on a queued view.
    assert.deepEqual(reported, [], `${item.standardView} must not be reported as applied`);
  }
  // The payload name itself is never offered to the name-only command funnel.
  assert.equal(COMMAND_CATALOG.some(command => command.id === 'STANDARDVIEW'), false);
});

test('a refused or throwing cad_standard_view is surfaced, never swallowed', async () => {
  // The host answers, but not with an admission.
  const calls = [];
  const reported = [];
  const refused = await submitStandardView({
    invoke: async (name, args) => { calls.push(name); return { accepted: false }; },
    view: 'isometric',
    t,
    report: message => reported.push(message),
  });
  assert.equal(refused.kind, 'refused');
  assert.equal(calls.length, 1);
  assert.equal(reported.length, 1, 'a non-admission must reach the banner');
  assert.ok(reported[0].includes('CAD_STANDARD_VIEW_NOT_ACCEPTED'), reported[0]);
  assert.notEqual(reported[0], 'CAD_STANDARD_VIEW_NOT_ACCEPTED', 'the banner shows catalog copy, not the bare code');

  // A rejected invoke surfaces the host's own reason verbatim.
  reported.length = 0;
  const thrown = await submitStandardView({
    invoke: async () => { throw Error('view.standard_unknown: nope'); },
    view: 'top',
    t,
    report: message => reported.push(message),
  });
  assert.equal(thrown.kind, 'refused');
  assert.equal(reported.length, 1);
  assert.ok(reported[0].includes('view.standard_unknown'), reported[0]);

  // A name outside the seven never reaches IPC at all.
  let invoked = false;
  const unknown = await requestStandardView(async () => { invoked = true; return { accepted: true }; }, 'sideways');
  assert.equal(unknown.kind, 'refused');
  assert.equal(unknown.error, 'CAD_STANDARD_VIEW_UNKNOWN');
  assert.equal(invoked, false, 'an unknown view must not be sent to the host');
  assert.ok(standardViewFailure(unknown, t).includes('CAD_STANDARD_VIEW_UNKNOWN'));
  // The admitted case yields no line, so the banner cannot show a false failure.
  assert.equal(standardViewFailure({ kind: 'accepted', view: 'top' }, t), '');
  assert.equal(standardViewFailure(null, t).includes('CAD_STANDARD_VIEW_'), true, 'a missing result is a named refusal');
});

test('the wire name is matched like the host does: trimmed and case-insensitive', () => {
  assert.equal(standardViewName(' Isometric '), 'isometric');
  assert.equal(standardViewName('TOP'), 'top');
  assert.equal(standardViewName('sideways'), null);
  assert.equal(standardViewName(''), null);
  assert.equal(standardViewName(null), null);
  assert.deepEqual([...EXPECTED_VIEWS], STANDARD_VIEWS.map(entry => entry.standardView));
});

test('standard views and the 3D toggles need an open drawing, with a real reason', () => {
  // No status yet, an import in flight, and the start page all stay disabled.
  assert.equal(standardViewAvailable({ status: null, documentOpen: false }), false);
  assert.equal(standardViewAvailable({ status: { ...loaded, loading: true }, documentOpen: true }), false);
  assert.equal(standardViewAvailable({ status: empty, documentOpen: false }), false);
  assert.equal(standardViewAvailable({ status: loaded, documentOpen: true }), true);
  // The reason is the catalog's existing "no drawing is open" sentence, in both
  // languages, and it is not the generic unsupported line.
  assert.equal(standardViewUnavailableReasonKey({ status: empty, documentOpen: false }), NO_DOCUMENT_KEY);
  assert.notEqual(NO_DOCUMENT_KEY, 'command.unsupported');
  assert.equal(typeof zh[NO_DOCUMENT_KEY], 'string');
  assert.equal(typeof en[NO_DOCUMENT_KEY], 'string');
  // Before the host has answered, the honest answer is the generic key.
  assert.equal(standardViewUnavailableReasonKey({ status: null, documentOpen: false }), 'command.unsupported');
  assert.equal(standardViewUnavailableReasonKey({ status: loaded, documentOpen: true }), 'command.unsupported');

  // The toggles that are payload-free router commands are gated on a document
  // the same way (disabled, not "unsupported", on the start page). `PLAN` is the
  // 2D-plan selector in the same group.
  assert.deepEqual([...DOCUMENT_SCOPED_COMMANDS], ['CLOSE', 'SWITCH2D3D', 'PROJECTION', 'MODEL', 'PLAN', 'AUDIT', 'STATUS', 'LAYTHW', 'HATCHTOBACK', 'LAYERSTATE', 'LTSCALE', 'LAYWALK', 'QSELECT', 'FILTER', 'GETSEL', 'SSX', 'SELECTCOUNT', '-FIND', 'FIND', 'HIDEOBJECTS', 'ISOLATEOBJECTS', 'UNISOLATEOBJECTS', 'PURGE', 'RENAME', 'SETVAR', 'TIME', 'DBLIST', 'TREESTAT', 'MREDO', 'MULTIPLE', 'REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL', 'VIEW', 'VIEWGO', 'VIEWBACK', 'VIEWFORWARD', 'VPOINT', 'LAYOUT', 'LAYOUTMERGE', 'LAYOUTWIZARD', 'VPORTS', 'VPMAX', 'VPMIN', 'VPCLIP', 'VPSCALE', 'LAYVPI', 'DIMSTYLE', '-DIMSTYLE', 'DIMREGEN', 'DIMDISASSOCIATE', 'GROUP', '-GROUP', 'UNGROUP', 'GROUPEDIT', 'QLEADER', 'LEADER', 'TEXTFIT', 'HATCHSETBOUNDARY', 'HATCHSETORIGIN', 'XDLIST', 'SKETCH', 'TRACE', 'MOCORO', 'MSTRETCH', 'VIEWRES', 'VSCURRENT', 'LINETYPE', '-LINETYPE', '-PAN', 'REGEN3', 'QTEXT', 'FILL', 'TFRAMES', 'REGENAUTO', 'ISODRAFT', 'BCOUNT', 'COUNT', 'EDITTIME', 'JULIAN', 'XLIST', 'DRAGMODE', 'APERTURE', 'ISOPLANE', 'HIGHLIGHTNEW', 'TRAYSETTINGS', 'TEXTSCR', 'GRAPHSCR', 'FINDINCLUDE', 'FINDMATCH', 'REVCLOUDPROPERTIES', 'SYSVARMONITOR', 'SYSVDLG', 'VTOPTIONS', 'ALIASEDIT', 'WHOHAS', 'ACADINFO', 'PTYPE', 'SCALELISTEDIT', 'HYPERLINKOPTIONS', 'INPUTSEARCHOPTIONS', '-INPUTSEARCHOPTIONS', 'SIGVALIDATE', 'BASE', 'LIMITS', 'VIEWPLOTDETAILS', 'VPLAYER', 'MVIEW', 'PSBSCALE', 'PSTSCALE', 'MVSETUP', 'MSPACE', 'PSPACE', 'STYLE', '-STYLE', 'MLEADERSTYLE', 'BLOCK?', 'DIMREASSOC', 'DIMREASSOCIATE', 'DIMJOGGED', 'DIMINSPECT', '-DIMINSPECT', 'ANNORESET', 'ANNOUPDATE', 'OBJECTSCALE', '-OBJECTSCALE', 'QLATTACH', 'QLATTACHSET', 'QLDETACHSET', 'ARRAYCLOSE', 'ARRAYEDIT', 'BEDIT', '-BEDIT', 'BCLOSE', 'BCYCLEORDER', 'BGRIPSET', 'BESETTINGS', 'BTESTBLOCK', 'LAYERPALETTE', 'CONSTRAINTBAR', 'CONSTRAINTSETTINGS', 'CENTERMARK', 'CENTERLINE', 'NCOPY', 'COPYTOLAYER', '-COPYTOLAYER', 'CDORDER', 'FLATTEN', 'XPLODE', 'BTRIM', 'BEXTEND', 'CHANGE', 'CHPROP', 'BSCALE', 'MINSERT', 'BLOCKREPLACE', 'BREPLACE', '-BREPLACE', 'SPLINEDIT', 'MLEDIT', '-MLEDIT', 'BURST', 'CLIP', 'CLIPIT', 'REGION', 'ADDSELECTED', 'WBLOCK', '-WBLOCK', 'BSAVE', 'BSAVEAS', 'CHSPACE', 'ATTDISP', 'BATTMAN', 'BATTORDER', 'XREF', '-XREF', 'EXTERNALREFERENCES', 'EXTERNALREFERENCESCLOSE', 'ATTDEF', '-ATTDEF', 'ATTEDIT', '-ATTEDIT', 'ATTIPEDIT', 'ATTIN', 'ATTOUT', 'ATTEXT', '-ATTEXT', 'EATTEDIT', 'EATTEXT', '-EATTEXT', 'ATTREDEF', 'ATTSYNC', 'GATTE', 'XDATA', 'XBIND', '-XBIND', 'XATTACH', 'ATTACH', '-ATTACH', 'XOPEN', 'CLASSICXREF', 'DWGLOG', 'LOGFILEON', 'LOGFILEOFF', 'DWGPROPS', 'DWFFORMAT', 'EXPORTPDF', 'SAVEALL', 'QNEW', 'REVERT', 'CLOSEALL', 'CLOSEALLOTHER', 'QQUIT', 'QUIT', 'PROPULATE', 'PLOTTERMANAGER', 'STYLESMANAGER', 'EXPORTSETTINGS', 'PREVIEW', 'EXPORTLAYOUT', 'PLOTSTAMP', '-PLOTSTAMP', '-PLOT', '-EXPORT', 'ETRANSMIT', '-ETRANSMIT', 'ARCHIVE', '-ARCHIVE', 'PUBLISH', '-PUBLISH', 'AUTOPUBLISH', 'BVSTATE', '-BVSTATE', 'BACTION', 'BACTIONBAR', 'BACTIONSET', 'BACTIONTOOL', 'BASSOCIATE', 'BAUTHORPALETTE', 'BAUTHORPALETTECLOSE', 'BCONSTRUCTION', 'BCPARAMETER', '-BCPARAMETER', 'BPARAMETER', 'BLOOKUPTABLE', 'BTABLE', 'BVHIDE', 'BVSHOW', 'RESETBLOCK', 'AUTOCONSTRAIN', 'DELCONSTRAINT', 'DIMCONSTRAINT', 'GEOMCONSTRAINT', 'DCALIGNED', 'DCANGULAR', 'DCCONVERT', 'DCDIAMETER', 'DCDISPLAY', 'DCFORM', 'DCHORIZONTAL', 'DCLINEAR', 'ELEV', 'UCSICON', 'UCS', 'UCSMAN', 'TABLET', 'TASKBAR', 'REINIT', 'SYSWINDOWS', 'MENU', 'RIBBON', 'ARRAY', '-ARRAY', 'ARRAYCLASSIC', 'ARCTEXT', 'BLEND', 'BREAKLINE', 'MLEADERALIGN', 'MLEADEREDIT', 'MLEADERCOLLECT', 'TOLERANCE', 'DIM', 'DIMARC', 'CAL', '-BLOCK', '-BOUNDARY', '-INSERT', '-HATCH', '-HATCHEDIT', 'EXOFFSET', 'DIMEX', 'DIMIM', 'SPACETRANS', 'TXTEXP', 'UPDATEFIELD', 'UNITS', '-UNITS', '-LAYER', 'LAYERSTATESAVE', 'NEWVIEW', '-SCALELISTEDIT', 'LWEIGHT', '-LWEIGHT', 'COLOR', '-COLOR', 'HELP', 'CLASSICINSERT', 'CLASSICGROUP', 'FILETAB', 'FILETABCLOSE', 'HIDEPALETTES', 'SHOWPALETTES', 'RIBBONCLOSE', 'GOTOSTART', 'COPYHIST', 'TRANSPARENCY', '3DALIGN', '3DARRAY', '3DCLIP', '3DCORBIT', '3DDISTANCE', '3DDWF', '3DEDITBAR', '3DFACE', '3DFLY', '3DFORBIT', '3DMESH', '3DMOVE', '3DORBIT', '3DORBITCTR', '-3DOSNAP', '3DOSNAP', '3DPAN', '3DPOLY', '3DPRINT', '3DPRINTSERVICE', '3DROTATE', '3DSCALE', '3DSWIVEL', '3DWALK', '3DZOOM', '3DSIN', 'ACISIN', 'ACISOUT', 'BMPOUT', 'JPGOUT', 'PNGOUT', 'TIFOUT', 'WMFIN', 'WMFOUT', 'IGESIMPORT', 'IGESEXPORT', 'STLOUT', 'IMPORT', 'EXPORTTOAUTOCAD', 'DWGCONVERT', 'PLT2DWG', 'DXBIN', 'APPLOAD', 'ARX', 'NETLOAD', 'VBAIDE', 'VBALOAD', 'VBARUN', 'VBAUNLOAD', 'SCRIPT']);


  for (const id of DOCUMENT_SCOPED_COMMANDS) {
    // A document-scoped command with a selection precondition is admitted only
    // once a document **and** enough objects are selected; `loaded` has no
    // selection, so those rows answer `unavailable` with their own hint. Every
    // other row is admitted as soon as a document is open.
    const requirement = SELECTION_REQUIREMENTS[id];
    const expected = requirement && requirement.count > 0 ? 'unavailable' : 'admitted';
    assert.equal(admitCommandText(id, { status: loaded }).kind, expected, id);
    assert.equal(admitCommandText(id, { status: empty }).kind, 'unavailable', id);
    assert.equal(admitCommandText(id, { status: null }).kind, 'unavailable', id);
  }
});

test('a declared carrier capability list decides which standard views are live', () => {
  // Absent (`undefined`) is the older-host / desktop `native-below` case: today's
  // exact behaviour, no capability question asked.
  assert.equal(standardViewAvailable({ status: loaded, documentOpen: true }, 'top'), true);
  assert.equal(standardViewAvailable({ status: loaded, documentOpen: true }), true);
  assert.equal(standardViewUnavailableReasonKey({ status: loaded, documentOpen: true }, 'top'), 'command.unsupported');
  // `[]` is an affirmative "this carrier has no standard views" (the dom-canvas
  // wasm session): every view is refused, and with its own reason — the carrier
  // limitation, not the "no drawing is open" sentence (a drawing *is* open there).
  const noViews = { ...loaded, hostProfile: { standardViews: [] } };
  for (const view of EXPECTED_VIEWS) {
    assert.equal(standardViewAvailable({ status: noViews, documentOpen: true }, view), false, view);
    assert.equal(standardViewUnavailableReasonKey({ status: noViews, documentOpen: true }, view), STANDARD_VIEW_UNAVAILABLE_KEY, view);
  }
  assert.notEqual(STANDARD_VIEW_UNAVAILABLE_KEY, NO_DOCUMENT_KEY);
  assert.notEqual(STANDARD_VIEW_UNAVAILABLE_KEY, 'command.unsupported');
  // A `null` list is the declared "nothing was declared" answer and keeps the
  // historic allow-all rule, exactly like an absent profile.
  const nullViews = { ...loaded, hostProfile: { standardViews: null } };
  assert.equal(standardViewAvailable({ status: nullViews, documentOpen: true }, 'top'), true);
  assert.equal(standardViewUnavailableReasonKey({ status: nullViews, documentOpen: true }, 'top'), 'command.unsupported');
  // A list that names the view keeps it live; a key outside it is refused.
  const someViews = { ...loaded, hostProfile: { standardViews: ['top', 'isometric'] } };
  assert.equal(standardViewAvailable({ status: someViews, documentOpen: true }, 'top'), true);
  assert.equal(standardViewAvailable({ status: someViews, documentOpen: true }, 'isometric'), true);
  assert.equal(standardViewAvailable({ status: someViews, documentOpen: true }, 'front'), false);
  assert.equal(standardViewUnavailableReasonKey({ status: someViews, documentOpen: true }, 'front'), STANDARD_VIEW_UNAVAILABLE_KEY);
  // An unnamed request on a declaring carrier cannot be confirmed by a bare list,
  // so it is refused rather than guessed — and the unknown-name path is unchanged.
  assert.equal(standardViewAvailable({ status: noViews, documentOpen: true }), false);
  assert.equal(standardViewAvailable({ status: someViews, documentOpen: true }, 'sideways'), false);
  // The capability never overrides the document gate: a carrier that *does* support
  // the view still needs an open drawing.
  assert.equal(standardViewAvailable({ status: someViews, documentOpen: false }, 'top'), false);
  assert.equal(standardViewUnavailableReasonKey({ status: someViews, documentOpen: false }, 'top'), NO_DOCUMENT_KEY);
  assert.equal(standardViewAvailable({ status: { ...someViews, loading: true }, documentOpen: true }, 'top'), false);
  // The catalog words the new reason in both languages, without a `{command}`
  // placeholder: a whole panel shares it (see `panelRefusalKey`).
  assert.equal(typeof zh[STANDARD_VIEW_UNAVAILABLE_KEY], 'string');
  assert.equal(typeof en[STANDARD_VIEW_UNAVAILABLE_KEY], 'string');
  assert.equal(en[STANDARD_VIEW_UNAVAILABLE_KEY].includes('{command}'), false);
  assert.equal(zh[STANDARD_VIEW_UNAVAILABLE_KEY].includes('{command}'), false);
  assert.equal(panelRefusalKey(STANDARD_VIEW_UNAVAILABLE_KEY), STANDARD_VIEW_UNAVAILABLE_KEY);
  // The ribbon passes the item's own view, so a capability list is consulted per
  // button rather than panel-wide.
  return readModule('components/Ribbon.tsx').then(ribbon => {
    assert.ok(ribbon.includes('item.standardView ? standardViewAvailable(shell, item.standardView)'));
    assert.ok(ribbon.includes('standardViewUnavailableReasonKey(shell, item.standardView)'));
  });
});

test('the orbit affordance stays visible, gesture-disabled, and emits no command', async () => {
  const orbit = viewTab().panels.find(panel => panel.id === 'view3d').items
    .find(item => item.id === 'ORBIT');
  assert.ok(orbit, 'the orbit hint must stay visible in the 3D panel');
  assert.equal(orbit.command, 'ORBIT');
  assert.equal(orbit.labelKey, 'view.orbit_label');
  assert.equal(orbit.standardView, undefined, 'orbit is a gesture, not a view payload');
  assert.equal(orbit.action, undefined, 'orbit is never a local shell action');
  // The router knows ORBIT, yet a click cannot supply the drag payload.
  assert.equal(isSupportedCommand('ORBIT'), false);
  assert.equal(admitCommandText('ORBIT', { status: loaded }).kind, 'unsupported');
  // Both catalogs carry the gesture reason the disabled button shows.
  assert.equal(typeof zh['view.orbit.gesture'], 'string');
  assert.equal(typeof en['view.orbit.gesture'], 'string');
  // A disabled ribbon button drops its click handler: no IPC can be emitted.
  const primitives = await readModule('components/primitives.tsx');
  assert.ok(
    primitives.includes('onClick={disabled ? undefined : onClick}'),
    'a disabled ribbon button must not carry a click handler',
  );
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('unavailableReasonKey(item.command, { status: shell.status, pending: shell.pending })'),
    'the gesture hint comes from the shared reason key',
  );
});

test('the renderer routes a standard view through the typed IPC, never through runCommand', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  const typed = ribbon.indexOf('if (item.standardView) return shell.runStandardView(item.standardView);');
  const command = ribbon.indexOf('if (item.command) return shell.runCommand(item.command);');
  assert.ok(typed !== -1, 'ribbon items must dispatch their standardView token');
  assert.ok(command !== -1, 'ribbon items keep dispatching their command token');
  assert.ok(typed < command, 'the payload branch must precede the name-only command branch');
  const app = await readModule('App.tsx');
  assert.ok(
    app.includes('submitStandardView({ invoke, view, t, report: setError })'),
    'the shell must surface a refused view through the error banner',
  );
});
