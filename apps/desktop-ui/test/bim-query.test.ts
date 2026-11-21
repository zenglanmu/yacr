// BIM browse panel: the one `cad_bim_query` wire model.
//
// Pure contract tests: the payload mapping (including the value-kind inference),
// the response normalisation (null fields stay null; they are not turned into
// empty strings), the nested `summary` block (one round trip), the page
// arithmetic edges, and the two stable refusals the panel must not confuse with
// an empty result.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BIM_PAGE_SIZE,
  BIM_PAYLOAD_INVALID,
  emptyBimFilter,
  bimFacetActive,
  bimDraftValueKind,
  bimFilterActive,
  bimFilterCount,
  bimPageInfo,
  bimPropertyDraft,
  bimRefusal,
  bimRowKey,
  bimSelectionIds,
  buildBimPropertyFilter,
  buildBimQuery,
  facetTerms,
  inferBimValue,
  isBimStaleRefusal,
  nextPageOffset,
  parseBimQueryAnswer,
  parseBimSummary,
  previousPageOffset,
  sameBimFilter,
  toggleBimFacet,
} from '../src/bim/bim-query.ts';

/** One wire row with every optional field present by default. */
function wireRow(overrides = {}) {
  return {
    object: { kind: 'source', connector: 'ifc', documentId: 'doc-1', id: 'global-1' },
    entityIds: [7],
    category: 'IfcWall',
    name: 'Basic Wall:Generic 200mm',
    levelName: 'Level 1',
    typeName: 'Basic Wall',
    materialName: 'Concrete',
    source: { connector: 'ifc', documentId: 'doc-1', id: 'global-1', nativeId: '22' },
    evidence: 'declared',
    propertySummary: [{ name: 'Pset_WallCommon', count: 3 }],
    ...overrides,
  };
}

/** One wire summary block (the nested part of a query answer). */
function wireSummaryBlock(overrides = {}) {
  return {
    total: 12,
    byCategory: [{ key: 'ifcwall', name: 'IfcWall', count: 12 }],
    byLevel: [{ key: 'level:ifc:doc:1', name: 'Level 1', count: 12 }],
    byType: [],
    byMaterial: [],
    ...overrides,
  };
}

/** One wire page: one round trip carries the rows and the summary block. */
function wirePage(overrides = {}) {
  return {
    databaseId: 1,
    revision: 42,
    total: 1,
    offset: 0,
    limit: 100,
    stale: false,
    rows: [wireRow()],
    summary: wireSummaryBlock({ total: 1, byCategory: [], byLevel: [] }),
    ...overrides,
  };
}

test('the empty filter maps to the frozen wire shape, every key present', () => {
  assert.deepEqual(buildBimQuery(emptyBimFilter(), 0, BIM_PAGE_SIZE), {
    filter: {
      categories: [],
      levels: [],
      types: [],
      materials: [],
      property: null,
      evidence: null,
    },
    offset: 0,
    limit: 100,
  });
  assert.equal(BIM_PAGE_SIZE, 100);
});

test('facet terms are trimmed, de-duplicated and keep their order', () => {
  assert.deepEqual(facetTerms([' IfcWall ', 'ifcwall', '', 'IfcSlab', 'IfcWall']), ['IfcWall', 'ifcwall', 'IfcSlab']);
  assert.deepEqual(facetTerms(null), []);
  assert.deepEqual(facetTerms([1, null, 'x']), ['x']);
  const filter = { ...emptyBimFilter(), categories: [' IfcWall '], levels: ['Level 1', 'Level 1'] };
  assert.deepEqual(buildBimQuery(filter).filter.categories, ['IfcWall']);
  assert.deepEqual(buildBimQuery(filter).filter.levels, ['Level 1']);
});

test('a property draft becomes one typed wire predicate', () => {
  assert.deepEqual(
    buildBimPropertyFilter({ pset: ' Pset_WallCommon ', property: ' IsExternal ', op: 'eq', value: 'true' }),
    { pset: 'Pset_WallCommon', property: 'IsExternal', op: 'eq', value: { kind: 'bool', value: true } },
  );
  assert.deepEqual(
    buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'ne', value: '-3' })?.value,
    { kind: 'integer', value: -3 },
  );
  assert.deepEqual(
    buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'contains', value: '2.5' })?.value,
    { kind: 'text', value: '2.5' },
  );
  assert.deepEqual(
    buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'contains', value: 'true' })?.value,
    { kind: 'text', value: 'true' },
  );
  // The live kind tag and the wire kind are the same decision.
  assert.equal(bimDraftValueKind({ pset: 'P', property: 'Q', op: 'contains', value: '2.5' }), 'text');
  assert.equal(bimDraftValueKind({ pset: 'P', property: 'Q', op: 'eq', value: '2.5' }), 'real');
  assert.equal(bimDraftValueKind({ pset: 'P', property: 'Q', op: 'eq', value: '' }), null);
  assert.deepEqual(
    buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'eq', value: ' Wall ' })?.value,
    { kind: 'text', value: 'Wall' },
  );
  // A blank field is refused locally: no request whose only answer is a refusal.
  assert.equal(buildBimPropertyFilter({ pset: '', property: 'Q', op: 'eq', value: '1' }), null);
  assert.equal(buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'eq', value: '   ' }), null);
  assert.equal(buildBimPropertyFilter(null), null);
});

test('the value kind is inferred in one place: bool, integer, real, text', () => {
  assert.deepEqual(inferBimValue('true'), { kind: 'bool', value: true });
  assert.deepEqual(inferBimValue(' FALSE '), { kind: 'bool', value: false });
  assert.deepEqual(inferBimValue('42'), { kind: 'integer', value: 42 });
  assert.deepEqual(inferBimValue('-0'), { kind: 'integer', value: 0 });
  assert.deepEqual(inferBimValue('3.5'), { kind: 'real', value: 3.5 });
  assert.deepEqual(inferBimValue('1e3'), { kind: 'real', value: 1000 });
  assert.deepEqual(inferBimValue('.5'), { kind: 'real', value: 0.5 });
  // Too large for a safe integer: a real keeps the magnitude, never a truncation.
  assert.deepEqual(inferBimValue('99999999999999999999'), { kind: 'real', value: 1e20 });
  assert.deepEqual(inferBimValue('Pset_WallCommon'), { kind: 'text', value: 'Pset_WallCommon' });
  assert.deepEqual(inferBimValue('12abc'), { kind: 'text', value: '12abc' });
  assert.equal(inferBimValue(''), null);
  assert.equal(inferBimValue('   '), null);
  assert.equal(inferBimValue(null), null);
});

test('a property filter round-trips into the form draft', () => {
  const filter = buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'ne', value: '7' });
  assert.deepEqual(bimPropertyDraft(filter), { pset: 'P', property: 'Q', op: 'ne', value: '7' });
  assert.deepEqual(bimPropertyDraft(null), { pset: '', property: '', op: 'eq', value: '' });
});

test('facet toggles add and remove exactly one key, and leave the rest alone', () => {
  const start = { ...emptyBimFilter(), categories: ['IfcWall'] };
  const added = toggleBimFacet(start, 'level', 'Level 1');
  assert.deepEqual(added.categories, ['IfcWall']);
  assert.deepEqual(added.levels, ['Level 1']);
  assert.equal(bimFacetActive(added, 'level', 'Level 1'), true);
  const removed = toggleBimFacet(added, 'category', 'IfcWall');
  assert.deepEqual(removed.categories, []);
  assert.equal(toggleBimFacet(start, 'type', ''), start);
  assert.equal(toggleBimFacet(start, 'type', null), start);
  assert.equal(bimFacetActive(start, 'category', ' IfcWall '), false);
});

test('filter activity and count read every group plus the property term', () => {
  assert.equal(bimFilterActive(emptyBimFilter()), false);
  const filter = {
    ...emptyBimFilter(),
    categories: ['a', 'b'],
    types: ['t'],
    property: buildBimPropertyFilter({ pset: 'P', property: 'Q', op: 'eq', value: '1' }),
  };
  assert.equal(bimFilterActive(filter), true);
  assert.equal(bimFilterCount(filter), 4);
  assert.equal(bimFilterCount(emptyBimFilter()), 0);
  assert.equal(sameBimFilter(filter, { ...filter, categories: ['a', 'b'] }), true);
  assert.equal(sameBimFilter(filter, { ...filter, categories: ['b', 'a'] }), false);
});

test('page arithmetic names every edge, including an empty total and limit 0', () => {
  assert.deepEqual(bimPageInfo(0, 0, 100), {
    page: 0, pages: 0, first: 0, last: 0, total: 0, hasPrev: false, hasNext: false,
  });
  assert.deepEqual(bimPageInfo(12, 0, 100), {
    page: 1, pages: 1, first: 1, last: 12, total: 12, hasPrev: false, hasNext: false,
  });
  assert.deepEqual(bimPageInfo(250, 100, 100), {
    page: 2, pages: 3, first: 101, last: 200, total: 250, hasPrev: true, hasNext: true,
  });
  assert.deepEqual(bimPageInfo(250, 200, 100), {
    page: 3, pages: 3, first: 201, last: 250, total: 250, hasPrev: true, hasNext: false,
  });
  // A stale offset after the document shrank: an empty range, Previous still works.
  assert.deepEqual(bimPageInfo(12, 200, 100), {
    page: 1, pages: 1, first: 0, last: 0, total: 12, hasPrev: true, hasNext: false,
  });
  // A host that answered with no page capacity: the total is still reported,
  // nothing is visible and there is no next page.
  assert.deepEqual(bimPageInfo(12, 0, 0), {
    page: 1, pages: 1, first: 0, last: 0, total: 12, hasPrev: false, hasNext: false,
  });
  assert.equal(previousPageOffset(100, 100), 0);
  assert.equal(previousPageOffset(0, 100), 0);
  assert.equal(nextPageOffset(250, 100, 100), 200);
  assert.equal(nextPageOffset(250, 200, 100), 200);
});

test('only the two stable markers are classified; everything else is an error', () => {
  assert.equal(bimRefusal('unsupported-runtime: bimQuery'), 'unsupported');
  assert.equal(bimRefusal('unsupported-runtime: bimQuery: the canvas session has no BIM lane'), 'unsupported');
  assert.equal(bimRefusal(' unsupported-runtime:  bimQuery '), 'unsupported');
  assert.equal(bimRefusal('unsupported-runtime: bimQueryX'), 'error');
  assert.equal(bimRefusal('cad_waiting'), 'waiting');
  assert.equal(bimRefusal('cad_waiting: no session'), 'waiting');
  assert.equal(bimRefusal('unsupported_on_surface_mode: cad_bim_query'), 'error');
  assert.equal(bimRefusal(''), 'error');
  assert.equal(isBimStaleRefusal('bim.stale: the query document is not the open document'), true);
  assert.equal(isBimStaleRefusal('bim.stale'), true);
  assert.equal(isBimStaleRefusal('bim.staleX'), false);
});

test('a query answer keeps null optional fields null and normalises the rest', () => {
  const answer = parseBimQueryAnswer(wirePage({
    rows: [
      wireRow({
        category: null,
        name: '',
        levelName: '  ',
        typeName: null,
        materialName: null,
        entityIds: [7, 7, 8, -1, '9', 1.5],
        propertySummary: [{ name: 'Pset_A', count: 2 }, { name: '  ', count: 9 }, { count: 1 }],
        source: { connector: 'ifc', documentId: null, id: null, nativeId: null },
      }),
      // A second component keyed by the entity itself.
      wireRow({ object: { kind: 'entity', entityId: 3 }, entityIds: [3] }),
      // A row with no addressable object cannot be selected or keyed: skipped.
      wireRow({ object: { kind: 'source', connector: 'ifc' } }),
      'not a row',
    ],
  }));
  assert.ok(answer);
  assert.equal(answer.rows.length, 2);
  const [first, second] = answer.rows;
  assert.equal(first.category, null);
  assert.equal(first.name, null);
  assert.equal(first.levelName, null);
  assert.equal(first.typeName, null);
  assert.equal(first.materialName, null);
  assert.deepEqual(first.entityIds, [7, 8]);
  assert.deepEqual(first.propertySummary, [{ name: 'Pset_A', count: 2 }]);
  assert.deepEqual(first.source, { connector: 'ifc', documentId: null, id: null, nativeId: null });
  assert.equal(first.evidence, 'declared');
  assert.equal(second.category, 'IfcWall');
  assert.equal(bimRowKey(second), 'entity:3');
  assert.equal(bimRowKey(first), 'source:ifc:doc-1:global-1');
});

test('an unreadable query payload is null, never an empty page', () => {
  assert.equal(parseBimQueryAnswer(null), null);
  assert.equal(parseBimQueryAnswer({}), null);
  assert.equal(parseBimQueryAnswer({ total: 0 }), null); // no rows array
  assert.equal(parseBimQueryAnswer({ rows: [] }), null); // no total
  assert.equal(parseBimQueryAnswer({ total: -1, rows: [] }), null);
  // The summary block is part of the frozen one-command shape: an answer without
  // it is a contract failure, not a document with no BIM data.
  assert.equal(parseBimQueryAnswer({ total: 3, rows: [] }), null);
  assert.equal(parseBimQueryAnswer({ total: 3, rows: [], summary: {} }), null);
  assert.equal(BIM_PAYLOAD_INVALID, 'bim.invalid_payload');
  // A real empty answer is a page with 0 rows, not a null payload.
  const empty = parseBimQueryAnswer(wirePage({ total: 0, rows: [] }));
  assert.ok(empty);
  assert.deepEqual(empty.rows, []);
});

test('the stale flag is read from the host, not inferred', () => {
  assert.equal(parseBimQueryAnswer(wirePage())?.stale, false);
  assert.equal(parseBimQueryAnswer(wirePage({ stale: true }))?.stale, true);
});

test('a limit-0 answer keeps the host window and its empty rows', () => {
  const answer = parseBimQueryAnswer(wirePage({ total: 12, limit: 0, rows: [] }));
  assert.ok(answer);
  assert.equal(answer.total, 12);
  assert.equal(answer.limit, 0);
  assert.deepEqual(answer.rows, []);
  // The panel refuses to offer a next page for a window with no capacity.
  assert.equal(bimPageInfo(answer.total, answer.offset, answer.limit).hasNext, false);
  assert.equal(bimPageInfo(answer.total, answer.offset, answer.limit).pages, 1);
});

test('the nested summary keeps the host facet lists and their null names', () => {
  const answer = parseBimQueryAnswer(wirePage({
    total: 12,
    summary: wireSummaryBlock({
      byCategory: [
        { key: 'ifcwall', name: 'IfcWall', count: 12 },
        { key: 'ifcwall', name: 'IfcWall again', count: 99 }, // duplicate key: first wins
        { key: 'ifcslab', name: null, count: 0 },
      ],
      byLevel: [],
      byMaterial: [{ key: 'material:concrete', name: 'Concrete', count: 4 }],
    }),
  }));
  assert.ok(answer);
  assert.equal(answer.total, 12);
  assert.equal(answer.summary.total, 12, 'summary.total is the same filtered total');
  assert.deepEqual(answer.summary.byCategory, [
    { key: 'ifcwall', name: 'IfcWall', count: 12 },
    { key: 'ifcslab', name: null, count: 0 },
  ]);
  assert.deepEqual(answer.summary.byLevel, []);
  assert.deepEqual(answer.summary.byMaterial, [{ key: 'material:concrete', name: 'Concrete', count: 4 }]);
});

test('the summary block needs its own total, and a missing list is an honest empty group', () => {
  // A block without its documented total is not the frozen shape: the answer is
  // refused rather than read as a zero-component document.
  assert.equal(parseBimSummary(null), null);
  assert.equal(parseBimSummary({}), null);
  assert.equal(parseBimSummary({ total: -1 }), null);
  assert.deepEqual(parseBimSummary({ total: 3 }), {
    total: 3, byCategory: [], byLevel: [], byType: [], byMaterial: [],
  });
});

test('selection ids are the row entity ids, de-duplicated and safe', () => {
  assert.deepEqual(bimSelectionIds({ entityIds: [3, 3, 4] }), [3, 4]);
  assert.deepEqual(bimSelectionIds({ entityIds: [-1, 1.5, '2'] }), []);
  assert.deepEqual(bimSelectionIds(null), []);
});
