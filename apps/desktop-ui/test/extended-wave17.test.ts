// The 2026-11-10 Wave 17-24 batch: 200 honest named refusals.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { admitCommandText, isSupportedCommand, setCommandCatalog } from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';
const BATCH = ['ACTBASEPOINT','ACTIVITYINSIGHTSCLOSE','ACTIVITYINSIGHTSOPEN','ACTMANAGER','ACTRECORD','-ACTSTOP','ACTSTOP','ACTUSERINPUT','-ACTUSERMESSAGE','ACTUSERMESSAGE','ADCCLOSE','ADCENTER','ADCNAVIGATE','ADJUST','AMECONVERT','ANALYSISCURVATURE','ANALYSISDRAFT','ANALYSISOPTIONS','ANALYSISZEBRA','ANIPATH','APPAUTOLOADER','APPSTORE','ARRAYPATH','ASSISTANTCLOSE','ASSISTANTOPEN','ATTACHURL','BDETECT','BLOCKICON','BLOCKSDATAOPTION','BLOCKSPALETTE','BLOCKSPALETTECLOSE','BLOCKTOXREF','BREP','BROWSER','BSEARCH','CAMERA','CENTERDISASSOCIATE','CENTERREASSOCIATE','CENTERRESET','CHECKSTANDARDS','CHURLS','COMMANDMACROS','COMMANDMACROSCLOSE','-COMPARE','COMPARE','COMPARECLOSE','COMPAREEXPORT','COMPAREIMPORT','COMPAREINFO','COMPILE','CONVERTOLDLIGHTS','CONVERTOLDMATERIALS','CONVTOMESH','CONVTONURBS','CONVTOSOLID','CONVTOSURFACE','COORDINATIONMODELATTACH','COPYFROMTRACE','COPYLINK','COPYM','COUNTAREA','COUNTAREACLOSE','COUNTCLOSE','COUNTFIELD','COUNTLIST','COUNTLISTCLOSE','COUNTNAVNEXT','COUNTNAVPREV','COUNTTABLE','CUI','CUIEXPORT','CUIIMPORT','CUILOAD','CUIUNLOAD','CUSTOMIZE','CVADD','CVHIDE','CVREBUILD','CVREMOVE','CVSHOW','-DATAEXTRACTION','DATAEXTRACTION','DATALINK','DATALINKUPDATE','DBCCLOSE','DBCCONFIGURE','DBCDEFINELLT','DBCDEFINELT','DBCONNECT','DCRADIUS','DCVERTICAL','DELAY','DETACHURL','DETECTCLOSE','DETECTCONVERT','DETECTPRIMARY','DETECTREMOVE','DETECTREVIEWNEXT','DETECTREVIEWPREV','-DGNEXPORT','DGNEXPORT','-DGNIMPORT','DGNIMPORT','DGNMAPPING','DIGITALSIGN','DISTANTLIGHT','DOWNLOADMANAGER','DRAWINGRECOVERY','DRAWINGRECOVERYHIDE','DVIEW','DWFADJUST','-DWFATTACH','DWFATTACH','DWFCLIP','DWFLAYERS','EDGESURF','EDITSHOT','EXPLAN','EXPORTDWF','EXPORTDWFX','EXPRESSMENU','EXPRESSTOOLS','FADEMARKUP','FLATSHOT','FREESPOT','FREEWEB','GCCOINCIDENT','GCCOLLINEAR','GCCONCENTRIC','GCEQUAL','GCFIX','GCHORIZONTAL','GCPARALLEL','GCPERPENDICULAR','GCSMOOTH','GCSYMMETRIC','GCTANGENT','GCVERTICAL','GEOGRAPHICLOCATION','GEOMAP','GEOMAPIMAGE','GEOMAPIMAGEUPDATE','GEOMARKLATLONG','GEOMARKME','GEOMARKPOINT','GEOMARKPOSITION','GEOREMOVE','GEOREORIENTMARKER','GOTOURL','-GRAPHICSCONFIG','GRAPHICSCONFIG','HIDE','HLSETTINGS','-HYPERLINK','HYPERLINK','-INSERTCONTENT','INSERTOBJ','-INTERFERE','INTERFERE','LIGHT','LIGHTLIST','LIGHTLISTCLOSE','LIVESECTION','LOAD','LSP','LSPSURF','MAKELISPAPP','MARKUP','-MARKUPASSIST','MARKUPASSIST','MARKUPCLOSE','-MARKUPIMPORT','MARKUPIMPORT','MATBROWSERCLOSE','MATBROWSEROPEN','MATEDITORCLOSE','MATEDITOROPEN','MATERIALASSIGN','MATERIALATTACH','MATERIALMAP','MATERIALS','MATERIALSCLOSE','MIGRATEMATERIALS','MKLTYPE','MKSHAPE','MOVEBAK','MSLIDE','NAVBAR','NAVSMOTION','NAVSMOTIONCLOSE','NAVSWHEEL','NAVVCUBE','NEWSHEETSET','NEWSHOT','OLECONVERT','OLELINKS','OLEOPEN','OLERESET','OLESCALE','OPENDWFMARKUP'];
const DOC={documentLabel:'drawing.dwg',loading:false}; const NO_DOC={documentLabel:null,loading:false};
setCommandCatalog(HOST_NAMES);
async function iconNames(){const map=(await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);assert.ok(map);return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(m=>m[1]));}
const slug=id=>`draw.kind.${id.startsWith('-')?`dash${id.slice(1)}`:id}`.toLowerCase();
test('every Wave 17-24 command is catalogued once with real keys', async()=>{
  const zh=await readCatalog('zh-CN'), en=await readCatalog('en'), glyphs=await iconNames();
  assert.equal(BATCH.length,200);
  for(const id of BATCH){const rows=COMMAND_CATALOG.filter(c=>c.id===id);assert.equal(rows.length,1,id);const [row]=rows;
    assert.equal(row.labelKey,slug(id),id);assert.ok(glyphs.has(row.icon),`${id} icon`);
    for(const [loc,cat] of [['zh-CN',zh],['en',en]]){assert.equal(typeof cat[row.labelKey],'string',`${loc} ${id}`);assert.equal(typeof cat[`${row.labelKey}.hint`],'string',`${loc} ${id}`);}
    assert.equal(HOST_NAMES.includes(id),true,id);assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id),true,id);
    assert.equal(CAPABILITY_GATES[id],undefined,id);assert.match(en[`${slug(id)}.hint`],/\.unsupported:/,id);}
});
test('the Wave 17-24 names are admitted with a drawing and refused without one',()=>{
  for(const id of BATCH){assert.equal(isSupportedCommand(id),true,id);
    assert.deepEqual(admitCommandText(id,{status:DOC}),{kind:'admitted',command:id,reasonKey:null},id);
    const noDoc=admitCommandText(id,{status:NO_DOC});assert.equal(noDoc.kind,'unavailable',id);assert.equal(noDoc.reasonKey,'plot.no_document',id);}
});
