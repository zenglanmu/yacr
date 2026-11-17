// The 2026-11-09 Wave 14 batch: 3D solid primitives (ADR-0036 phase C1 creates
// exact bodies), the three booleans (phase C2 runs a background job), and the
// remaining named refusals.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { admitCommandText, isSupportedCommand, setCommandCatalog } from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';
// The seven primitive families create a body (explicit points, no prompt yet).
const PRIMITIVES = ['BOX','CONE','CYLINDER','SPHERE','TORUS','WEDGE','PYRAMID'];
// The booleans need at least two selected solid bodies and start a background job.
const BOOLEANS = ['INTERSECT','UNION','SUBTRACT'];
// CHAMFEREDGE (ADR-0036 P2.0) is a one-pick tool over exactly one selected body.
const CHAMFER_EDGE = 'CHAMFEREDGE';
// Everything else in the batch is still a stable named refusal.
const REFUSALS = ['POLYSOLID','EXTRUDE','REVOLVE','SWEEP','LOFT','SLICE','THICKEN','PRESSPULL','IMPRINT','OFFSETEDGE','FILLETEDGE','EDGE','MIRROR3D','ROTATE3D'];
const BATCH = [...PRIMITIVES, ...BOOLEANS, CHAMFER_EDGE, ...REFUSALS];
const DOC={documentLabel:'drawing.dwg',loading:false}; const NO_DOC={documentLabel:null,loading:false};
const SEL2={...DOC,selection:{count:2}};
setCommandCatalog(HOST_NAMES);
async function iconNames(){const map=(await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);assert.ok(map);return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(m=>m[1]));}
const slug=id=>`draw.kind.${id.toLowerCase()}`;
test('every Wave 14 command is catalogued once with real keys', async()=>{
  const zh=await readCatalog('zh-CN'), en=await readCatalog('en'), glyphs=await iconNames();
  for(const id of BATCH){const rows=COMMAND_CATALOG.filter(c=>c.id===id);assert.equal(rows.length,1,id);const [row]=rows;
    assert.equal(row.labelKey,slug(id),id);assert.ok(glyphs.has(row.icon),`${id} icon`);
    for(const [loc,cat] of [['zh-CN',zh],['en',en]]){assert.equal(typeof cat[row.labelKey],'string');assert.equal(typeof cat[`${row.labelKey}.hint`],'string');}
    assert.equal(HOST_NAMES.includes(id),true,id);
    // A document alone is not the whole requirement for a selection-gated row,
    // so CHAMFEREDGE answers the no-document case through
    // SELECTION_REQUIREMENTS instead of the document-scoped list.
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id),id !== CHAMFER_EDGE,id);
    assert.equal(CAPABILITY_GATES[id],undefined,id);
    if(PRIMITIVES.includes(id)){
      assert.doesNotMatch(en[`${slug(id)}.hint`],/Named refusal/i,id);
      assert.match(en[`${slug(id)}.hint`],/Explicit points/i,id);
    } else if(BOOLEANS.includes(id)){
      assert.doesNotMatch(en[`${slug(id)}.hint`],/Named refusal/i,id);
      assert.match(en[`${slug(id)}.hint`],/at least two selected solid bodies/i,id);
      assert.match(en[`${slug(id)}.hint`],/background job/i,id);
      assert.deepEqual({ ...SELECTION_REQUIREMENTS[id] },{count:2,reasonKey:`${slug(id)}.hint`},id);
    } else if(id === CHAMFER_EDGE){
      // One exact solid body is the target; the click is the kernel's seed.
      assert.doesNotMatch(en[`${slug(id)}.hint`],/Named refusal/i,id);
      assert.match(en[`${slug(id)}.hint`],/exact 3D solid body/i,id);
      assert.match(en[`${slug(id)}.hint`],/chamferDistanceA/i,id);
      assert.deepEqual({ ...SELECTION_REQUIREMENTS[id] },{count:1,reasonKey:`${slug(id)}.hint`},id);
      // The shared 0/0 channel default is legal for 2D CHAMFER but not for a 3D
      // set-back, so the distance hints and the step prompt must say so.
      for(const key of ['status.tool_parameter.chamfer_distance_a.hint','status.tool_parameter.chamfer_distance_b.hint']){
        assert.match(en[key],/CHAMFEREDGE/i,key);
        assert.match(en[key],/greater than 0/i,key);
      }
      assert.match(en['tool.prompt.chamferedge.pick_edge'],/greater than 0/i,'chamferedge prompt');
    } else {
      assert.match(en[`${slug(id)}.hint`],/Named refusal/i,id);
      assert.equal(SELECTION_REQUIREMENTS[id],undefined,id);
    }}
});
test('the Wave 14 names are admitted with a drawing and refused without one',()=>{
  for(const id of BATCH){assert.equal(isSupportedCommand(id),true,id);
    assert.equal(admitCommandText(id,{status:NO_DOC}).kind,'unavailable',id);
    if(BOOLEANS.includes(id)){
      // A document alone is not the whole requirement: two selected bodies are required.
      assert.deepEqual(admitCommandText(id,{status:DOC}),{kind:'unavailable',command:id,reasonKey:`${slug(id)}.hint`},id);
      assert.deepEqual(admitCommandText(id,{status:SEL2}),{kind:'admitted',command:id,reasonKey:null},id);
    } else if(id === CHAMFER_EDGE){
      // One selected body is the requirement (the hint is the refusal sentence).
      assert.deepEqual(admitCommandText(id,{status:DOC}),{kind:'unavailable',command:id,reasonKey:`${slug(id)}.hint`},id);
      assert.deepEqual(admitCommandText(id,{status:{...DOC,selection:{count:1}}}),{kind:'admitted',command:id,reasonKey:null},id);
      assert.equal(admitCommandText(id,{status:NO_DOC}).reasonKey,'plot.no_document',id);
    } else {
      assert.deepEqual(admitCommandText(id,{status:DOC}),{kind:'admitted',command:id,reasonKey:null},id);
      assert.equal(admitCommandText(id,{status:NO_DOC}).reasonKey,'plot.no_document',id);
    }}
});
