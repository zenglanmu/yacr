// BIM browse panel: the response → view-model projection.
//
// These are the rules the rendered rows depend on: a null field is the catalog's
// "Unspecified" (never a blank cell and never the word `null`), evidence tokens
// become sentences or stay verbatim, facets are ordered for reading, and the
// empty state distinguishes "no BIM data" from "filter matched nothing".
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BIM_UNSET_KEY,
  bimEmptyCopy,
  bimEvidenceView,
  bimFacetGroups,
  bimFacetLabelKey,
  bimField,
  bimRowView,
  bimRowViews,
  bimTotalsView,
  sortBimFacets,
} from '../src/bim/bim-view.ts';
import { emptyBimFilter } from '../src/bim/bim-query.ts';

/** One parsed row with every optional field set. */
function row(overrides = {}) {
  return {
    object: { kind: 'source', connector: 'ifc', documentId: 'doc', id: 'g1' },
    entityIds: [7],
    category: 'IfcWall',
    name: 'Basic Wall',
    levelName: 'Level 1',
    typeName: 'Basic Wall',
    materialName: 'Concrete',
    source: { connector: 'ifc', documentId: 'doc', id: 'g1', nativeId: null },
    evidence: 'declared',
    propertySummary: [{ name: 'Pset_WallCommon', count: 3 }],
    ...overrides,
  };
}

test('an unset optional field is marked, never an empty string', () => {
  assert.deepEqual(bimField('Level 1'), { value: 'Level 1', unset: false });
  assert.deepEqual(bimField(null), { value: '', unset: true });
  assert.deepEqual(bimField('   '), { value: '', unset: true });
  assert.equal(BIM_UNSET_KEY, 'bim.value.unset');
});

test('evidence tokens map to catalog keys; an unknown token stays verbatim', () => {
  assert.deepEqual(bimEvidenceView('declared'), { key: 'bim.evidence.declared', raw: 'declared' });
  assert.deepEqual(bimEvidenceView('heuristic'), { key: 'bim.evidence.heuristic', raw: 'heuristic' });
  assert.deepEqual(bimEvidenceView('vendor-claim'), { key: null, raw: 'vendor-claim' });
  assert.deepEqual(bimEvidenceView(null), { key: null, raw: null });
});

test('one row projects every field, the component count and its property summary', () => {
  const view = bimRowView(row({ entityIds: [7, 8, 9] }));
  assert.equal(view.key, 'source:ifc:doc:g1');
  assert.deepEqual(view.category, { value: 'IfcWall', unset: false });
  assert.deepEqual(view.name, { value: 'Basic Wall', unset: false });
  assert.equal(view.entityCount, 3);
  assert.equal(view.selectable, true);
  assert.deepEqual(view.properties, ['Pset_WallCommon (3)']);
  assert.equal(view.evidence.key, 'bim.evidence.declared');
});

test('a component with no geometry ids is still shown, but not selectable', () => {
  const view = bimRowView(row({ entityIds: [] }));
  assert.equal(view.entityCount, 0);
  assert.equal(view.selectable, false);
});

test('null name/level/type/material become unset views, in the same order', () => {
  const view = bimRowView(row({ name: null, levelName: null, typeName: null, materialName: null }));
  for (const field of [view.name, view.level, view.type, view.material]) {
    assert.equal(field.unset, true);
    assert.equal(field.value, '');
  }
  assert.equal(view.category.unset, false);
});

test('a page projects row by row and an absent page is an empty list', () => {
  assert.equal(bimRowViews([row(), row({ object: { kind: 'entity', entityId: 3 } })]).length, 2);
  assert.deepEqual(bimRowViews(null), []);
});

test('facets sort by count, then name, then key; empty groups are omitted', () => {
  assert.deepEqual(
    sortBimFacets([
      { key: 'b', name: 'Beta', count: 1 },
      { key: 'a', name: 'Alpha', count: 3 },
      { key: 'c', name: null, count: 3 },
    ]).map(facet => facet.key),
    ['a', 'c', 'b'],
  );
  const groups = bimFacetGroups({
    total: 4,
    byCategory: [{ key: 'ifcwall', name: 'IfcWall', count: 4 }],
    byLevel: [],
    byType: [{ key: 'type:basic', name: 'Basic Wall', count: 2 }],
    byMaterial: [],
  });
  assert.deepEqual(groups.map(group => group.kind), ['category', 'type']);
  assert.equal(groups[0].labelKey, 'bim.facet.category');
  assert.equal(bimFacetLabelKey('material'), 'bim.facet.material');
  assert.deepEqual(bimFacetGroups(null), []);
});

test('the totals line shows the document total and the filtered total together', () => {
  assert.deepEqual(bimTotalsView(12, emptyBimFilter(), { total: 12 }), { total: 12, matched: null });
  const filter = { ...emptyBimFilter(), categories: ['ifcwall'] };
  assert.deepEqual(bimTotalsView(12, filter, { total: 3 }), { total: 12, matched: 3 });
  // A newer filter whose page has not answered yet must not borrow the old count.
  assert.deepEqual(bimTotalsView(12, filter, { total: 3 }, emptyBimFilter()), { total: 12, matched: null });
  // A failed summary still has the page's own total to show.
  assert.deepEqual(bimTotalsView(null, emptyBimFilter(), { total: 5 }), { total: 5, matched: null });
  assert.deepEqual(bimTotalsView(null, emptyBimFilter(), null), { total: 0, matched: null });
});

test('empty copy separates "no BIM data" from "the filter matched nothing"', () => {
  assert.deepEqual(bimEmptyCopy('empty', emptyBimFilter(), 0, 0), {
    titleKey: 'bim.empty',
    hintKey: 'bim.empty_hint',
  });
  const active = { ...emptyBimFilter(), categories: ['ifcwall'] };
  assert.deepEqual(bimEmptyCopy('filtered-empty', active, 12, 0), {
    titleKey: 'bim.no_match',
    hintKey: 'bim.no_match_hint',
  });
  // A host that answered an empty unfiltered page is "no data", not "no match".
  assert.equal(bimEmptyCopy('empty', active, 0, 0)?.titleKey, 'bim.empty');
  // Only the empty phases have copy.
  assert.equal(bimEmptyCopy('ready', active, 12, 4), null);
  assert.equal(bimEmptyCopy('error', active, null, 0), null);
  assert.equal(bimEmptyCopy('unsupported', active, null, 0), null);
});
