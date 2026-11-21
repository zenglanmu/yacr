// The BIM browse panel's wire model: the `cad_bim_query` / `cad_bim_summary`
// contract (ADR-0042 Phase 3), its normalisation and the pure filter → request
// mapping.
//
// Pure data + functions only (no React, no JSX, no DOM), the same shape
// `layers/layers.ts` and `fonts/fonts.ts` use: the panel's behaviour is unit
// testable without a window.
//
// # Wire shape (frozen by the host lane)
//
//   cad_bim_query / cad_bim_summary  { filter, offset, limit }
//     filter: {
//       categories: [string], levels: [string], types: [string], materials: [string],
//       property: null | { pset, property, op: "eq" | "ne" | "contains",
//                          value: { kind: "text" | "real" | "integer" | "bool", value } },
//       evidence: null | "declared" | "heuristic"
//     }
//
//   cad_bim_query -> { databaseId, revision, total, offset, limit, stale,
//                      rows: [{ object, entityIds, category, name, levelName,
//                               typeName, materialName, source, evidence,
//                               propertySummary: [{ name, count }] }] }
//   cad_bim_summary -> { databaseId, revision, total,
//                        summary: { total, byCategory, byLevel, byType, byMaterial } }
//                      // each facet: { key, name, count }
//
// # Three rules this module is built around
//
//  * **Nothing is invented.** A facet value the host did not send is not added;
//    the panel's chips are exactly the host's list. A row field the host sent as
//    `null` stays `null` here — the view layer renders the catalog's "unset"
//    label, never an empty cell or the word `null`.
//  * **A refusal is not an empty answer.** `unsupported-runtime: bimQuery` (a
//    `dom-canvas` carrier with no BIM query channel) and `cad_waiting` (no
//    session yet) are distinct from `{ total: 0 }`. [`bimRefusal`] owns that
//    classification; the panel renders each state differently.
//  * **The request is built from what the user chose.** Facet keys are trimmed
//    and de-duplicated but never re-spelled or case-folded: they are the host's
//    own facet keys. The property value's kind is inferred once, here, so the
//    form and the wire can never disagree.

/** The Tauri IPC arguments this module sends. */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;

/** The typed IPC call the store needs (`invoke` satisfies it). */
export type Invoke = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The query command: one filtered, paged page of logical BIM components. */
export const BIM_QUERY_COMMAND = 'cad_bim_query';

/** The summary command: total plus the four facet lists (unfiltered by the panel). */
export const BIM_SUMMARY_COMMAND = 'cad_bim_summary';

/**
 * The fixed page size. The panel has one size, so "next page" is always one
 * `limit` further and a wire that answers a different `limit` cannot desync the
 * offset: the answer's own `limit` is what the next request uses.
 */
export const BIM_PAGE_SIZE = 100;

/**
 * The stable refusal of a carrier with no BIM query channel.
 *
 * A `dom-canvas` carrier (browser, Android WebView on the wasm session, X11)
 * answers `unsupported-runtime: bimQuery` — never an empty table that would read
 * as "this drawing has no BIM data".
 */
export const BIM_UNSUPPORTED_MARKER = 'unsupported-runtime: bimQuery';

/** The host's "no session yet" code: a loading state, never an empty answer. */
export const BIM_WAITING_CODE = 'cad_waiting';

/**
 * The stable reason the store reports for an unreadable answer.
 *
 * A payload that is not the documented shape is a contract failure; it is never
 * folded into an empty page (which would read as "no BIM data").
 */
export const BIM_PAYLOAD_INVALID = 'bim.invalid_payload';

/**
 * The result area's states.
 *
 * `filtered-empty` and `empty` are deliberately separate: one says "the filter
 * matched nothing", the other "this drawing has no BIM data". `unsupported` and
 * `waiting` are not empty states at all — they mean the panel never got an
 * answer it could show.
 */
export type BimPhase =
  | 'idle'
  | 'loading'
  | 'ready'
  | 'empty'
  | 'filtered-empty'
  | 'waiting'
  | 'unsupported'
  | 'error';

/** The closed set of property-value kinds the wire accepts. */
export type BimValueKind = 'text' | 'real' | 'integer' | 'bool';

/** One typed property value as `cad_bim_query` carries it. */
export interface BimValue {
  readonly kind: BimValueKind;
  readonly value: string | number | boolean;
}

/** The three property comparison operators. */
export type BimPropertyOp = 'eq' | 'ne' | 'contains';

/** One `(pset, property) op value` filter. */
export interface BimPropertyFilter {
  readonly pset: string;
  readonly property: string;
  readonly op: BimPropertyOp;
  readonly value: BimValue;
}

/** The persisted evidence token a row can carry. */
export type BimEvidence = 'declared' | 'heuristic';

/**
 * The filter both commands share.
 *
 * The arrays are **ANDed** by the host (one term each) and the panel sends the
 * host's own facet keys. `evidence` is modelled because the wire has it, but the
 * panel's first version exposes only the four facet groups and the property
 * filter; it always sends `null` there.
 */
export interface BimFilter {
  readonly categories: readonly string[];
  readonly levels: readonly string[];
  readonly types: readonly string[];
  readonly materials: readonly string[];
  readonly property: BimPropertyFilter | null;
  readonly evidence: BimEvidence | null;
}

/** The request body both commands take. */
export interface BimQueryPayload {
  readonly filter: BimFilter;
  readonly offset: number;
  readonly limit: number;
}

/** The facet groups the summary answer carries, in the panel's order. */
export type BimFacetGroupKind = 'category' | 'level' | 'type' | 'material';

/**
 * One logical component's identity: the persisted source key when the element
 * declared one, otherwise the element's own id.
 */
export type BimObjectRef =
  | {
    readonly kind: 'source';
    readonly connector: string;
    readonly documentId: string;
    readonly id: string;
  }
  | { readonly kind: 'entity'; readonly entityId: number };

/** Where one component came from (the row's provenance block). */
export interface BimSource {
  readonly connector: string;
  /** The source document's own identifier, when it had one. */
  readonly documentId: string | null;
  /** The element's identifier inside that document, when it had one. */
  readonly id: string | null;
  readonly nativeId: string | null;
}

/** One `Pset (count)` summary entry. */
export interface BimPropertySummaryEntry {
  readonly name: string;
  readonly count: number;
}

/** One logical component as `cad_bim_query` describes it. */
export interface BimRow {
  readonly object: BimObjectRef;
  /** Every geometry row of this component (one entry for a single-mesh wall). */
  readonly entityIds: readonly number[];
  readonly category: string | null;
  readonly name: string | null;
  readonly levelName: string | null;
  readonly typeName: string | null;
  readonly materialName: string | null;
  readonly source: BimSource | null;
  /** `declared`/`heuristic`, or any other host token (shown verbatim). */
  readonly evidence: string | null;
  readonly propertySummary: readonly BimPropertySummaryEntry[];
}

/** One parsed `cad_bim_query` answer. */
export interface BimQueryAnswer {
  readonly databaseId: number | null;
  readonly revision: number | null;
  readonly total: number;
  readonly offset: number;
  readonly limit: number;
  /** The host says the document changed after this answer was built. */
  readonly stale: boolean;
  readonly rows: readonly BimRow[];
}

/** One facet entry: the host's key, its display name (or `null`) and its count. */
export interface BimFacet {
  readonly key: string;
  readonly name: string | null;
  readonly count: number;
}

/** The four facet lists of one summary answer. */
export interface BimSummary {
  readonly total: number;
  readonly byCategory: readonly BimFacet[];
  readonly byLevel: readonly BimFacet[];
  readonly byType: readonly BimFacet[];
  readonly byMaterial: readonly BimFacet[];
}

/** One parsed `cad_bim_summary` answer. */
export interface BimSummaryAnswer {
  readonly databaseId: number | null;
  readonly revision: number | null;
  readonly total: number;
  readonly summary: BimSummary;
}

/** The evidence label keys the catalog ships, or `null` for an unknown token. */
export function bimEvidenceKey(evidence: unknown): string | null {
  switch (evidence) {
    case 'declared': return 'bim.evidence.declared';
    case 'heuristic': return 'bim.evidence.heuristic';
    default: return null;
  }
}

/** The catalogue label for a property operator. */
export function bimOperatorKey(op: BimPropertyOp): string {
  switch (op) {
    case 'eq': return 'bim.property.op.eq';
    case 'ne': return 'bim.property.op.ne';
    case 'contains': return 'bim.property.op.contains';
  }
}

/** The catalogue label for an inferred value kind. */
export function bimValueKindKey(kind: BimValueKind): string {
  switch (kind) {
    case 'text': return 'bim.property.kind.text';
    case 'real': return 'bim.property.kind.real';
    case 'integer': return 'bim.property.kind.integer';
    case 'bool': return 'bim.property.kind.bool';
  }
}

/** A frozen empty filter (the summary's own request, and the clear target). */
export const EMPTY_BIM_FILTER: BimFilter = Object.freeze({
  categories: Object.freeze([]) as readonly string[],
  levels: Object.freeze([]) as readonly string[],
  types: Object.freeze([]) as readonly string[],
  materials: Object.freeze([]) as readonly string[],
  property: null,
  evidence: null,
});

/** An empty filter for a form's draft. */
export function emptyBimFilter(): BimFilter {
  return EMPTY_BIM_FILTER;
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** A non-negative safe integer, or the fallback. */
function countOf(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : fallback;
}

/** A finite number (the id fields are integers, but nothing is trusted to be). */
function safeNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && Number.isSafeInteger(value) ? value : null;
}

/** A trimmed non-empty string, or `null` (an empty cell is not a label). */
export function textOrNull(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const text = value.trim();
  return text.length > 0 ? text : null;
}

// --------------------------------------------------------------- inference ---

/** A whole decimal literal (the sign is part of the user's input). */
const INTEGER_RE = /^[+-]?\d+$/;

/** A decimal literal with a fraction and/or an exponent. */
const REAL_RE = /^[+-]?(?:\d+\.\d*|\.\d+|\d+)(?:[eE][+-]?\d+)?$/;

/**
 * Infer one typed value from the form's text.
 *
 * The order is deliberate: `true`/`false` (any case) first, then an integer that
 * JavaScript can represent exactly, then any other finite decimal, then text.
 * A literal too large for a safe integer is sent as a real (its numeric value is
 * still finite) rather than silently truncated; an empty or blank field answers
 * `null`, which is what makes the request's property filter invalid instead of
 * sending an empty string.
 */
export function inferBimValue(raw: unknown): BimValue | null {
  const text = typeof raw === 'string' ? raw.trim() : '';
  if (text.length === 0) return null;
  const lower = text.toLowerCase();
  if (lower === 'true') return { kind: 'bool', value: true };
  if (lower === 'false') return { kind: 'bool', value: false };
  if (INTEGER_RE.test(text)) {
    const number = Number(text);
    if (Number.isSafeInteger(number)) return { kind: 'integer', value: number === 0 ? 0 : number };
  }
  if (REAL_RE.test(text)) {
    const number = Number(text);
    if (Number.isFinite(number)) return { kind: 'real', value: number === 0 ? 0 : number };
  }
  return { kind: 'text', value: text };
}

/** A validated property value, or `null` when `kind`/`value` disagree. */
function parseBimValue(value: unknown): BimValue | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const kind = fields.kind;
  const raw = fields.value;
  switch (kind) {
    case 'text':
      return typeof raw === 'string' ? { kind, value: raw } : null;
    case 'bool':
      return typeof raw === 'boolean' ? { kind, value: raw } : null;
    case 'integer':
      return typeof raw === 'number' && Number.isSafeInteger(raw) ? { kind, value: raw } : null;
    case 'real':
      return typeof raw === 'number' && Number.isFinite(raw) ? { kind, value: raw } : null;
    default:
      return null;
  }
}

// ------------------------------------------------------------ filter build ---

/** Trim, drop blanks and de-duplicate one facet selection, keeping its order. */
export function facetTerms(values: unknown): string[] {
  if (!Array.isArray(values)) return [];
  const seen = new Set<string>();
  const terms: string[] = [];
  for (const raw of values) {
    if (typeof raw !== 'string') continue;
    const key = raw.trim();
    if (key.length === 0 || seen.has(key)) continue;
    seen.add(key);
    terms.push(key);
  }
  return terms;
}

/** One property filter as the wire spells it, or `null` when unusable. */
export function parseBimPropertyFilter(value: unknown): BimPropertyFilter | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const pset = textOrNull(fields.pset);
  const property = textOrNull(fields.property);
  if (!pset || !property) return null;
  const op = fields.op;
  if (op !== 'eq' && op !== 'ne' && op !== 'contains') return null;
  const typed = parseBimValue(fields.value);
  if (!typed) return null;
  return { pset, property, op, value: typed };
}

/** The property-filter draft the form holds (all strings, before inference). */
export interface BimPropertyDraft {
  readonly pset: string;
  readonly property: string;
  readonly op: BimPropertyOp;
  readonly value: string;
}

/** A blank draft. */
export const EMPTY_BIM_PROPERTY_DRAFT: BimPropertyDraft = Object.freeze({
  pset: '',
  property: '',
  op: 'eq',
  value: '',
});

/**
 * The kind the form's live tag shows and the wire will carry for one draft.
 *
 * `contains` is a **substring test**, which the host only ever answers for text
 * values ("never true for a non-text value", ADR-0042 D11), so a numeric or
 * boolean draft is sent as text under that operator. `eq`/`ne` keep the inferred
 * kind, because there a number really is compared numerically.
 */
export function bimDraftValueKind(draft: BimPropertyDraft | null | undefined): BimValueKind | null {
  const value = inferBimValue(draft?.value);
  if (!value) return null;
  return draft?.op === 'contains' && value.kind !== 'text' ? 'text' : value.kind;
}

/**
 * Build one property filter from the form's draft.
 *
 * A blank `pset`, `property` or value answers `null` (the form refuses locally;
 * no request whose only possible answer is a refusal is sent). The value's kind
 * comes from [`bimDraftValueKind`], which is also what the form shows as a small
 * "text / integer / real / bool" tag, so the tag and the wire cannot drift.
 */
export function buildBimPropertyFilter(draft: BimPropertyDraft | null | undefined): BimPropertyFilter | null {
  if (!draft) return null;
  const pset = draft.pset.trim();
  const property = draft.property.trim();
  if (!pset || !property) return null;
  const value = inferBimValue(draft.value);
  if (!value) return null;
  const op = draft.op === 'ne' || draft.op === 'contains' ? draft.op : 'eq';
  const typed: BimValue = op === 'contains' && value.kind !== 'text'
    ? { kind: 'text', value: draft.value.trim() }
    : value;
  return { pset, property, op, value: typed };
}

/** The draft a form opens with: the applied filter's own values as text. */
export function bimPropertyDraft(filter: BimPropertyFilter | null): BimPropertyDraft {
  if (!filter) return EMPTY_BIM_PROPERTY_DRAFT;
  return {
    pset: filter.pset,
    property: filter.property,
    op: filter.op,
    value: String(filter.value.value),
  };
}

/**
 * Build one command payload from the panel's filter and page.
 *
 * The property and evidence keys are **always present** (as `null` when unset):
 * the wire shape is explicit, so an omitted key is not a second spelling of
 * "no filter". Facet lists are normalised by [`facetTerms`]; an unusable
 * property filter becomes `null` rather than a half-filled term.
 */
export function buildBimQuery(
  filter: BimFilter | null | undefined,
  offset = 0,
  limit = BIM_PAGE_SIZE,
): BimQueryPayload {
  const property = parseBimPropertyFilter(filter?.property);
  const evidence = filter?.evidence === 'declared' || filter?.evidence === 'heuristic'
    ? filter.evidence
    : null;
  return {
    filter: {
      categories: facetTerms(filter?.categories),
      levels: facetTerms(filter?.levels),
      types: facetTerms(filter?.types),
      materials: facetTerms(filter?.materials),
      property,
      evidence,
    },
    offset: countOf(offset, 0),
    limit: countOf(limit, BIM_PAGE_SIZE),
  };
}

/** Whether any facet, property or evidence term is set. */
export function bimFilterActive(filter: BimFilter | null | undefined): boolean {
  if (!filter) return false;
  return filter.categories.length > 0
    || filter.levels.length > 0
    || filter.types.length > 0
    || filter.materials.length > 0
    || filter.property !== null
    || filter.evidence !== null;
}

/** How many terms the filter carries (one per facet value plus the property). */
export function bimFilterCount(filter: BimFilter | null | undefined): number {
  if (!filter) return 0;
  return facetTerms(filter.categories).length
    + facetTerms(filter.levels).length
    + facetTerms(filter.types).length
    + facetTerms(filter.materials).length
    + (filter.property ? 1 : 0)
    + (filter.evidence ? 1 : 0);
}

/** Whether one facet key is selected in its group. */
export function bimFacetActive(
  filter: BimFilter | null | undefined,
  group: BimFacetGroupKind,
  key: unknown,
): boolean {
  if (!filter || typeof key !== 'string') return false;
  const list = groupList(filter, group);
  return list.includes(key);
}

/** The selected keys of one facet group. */
export function groupList(filter: BimFilter, group: BimFacetGroupKind): readonly string[] {
  switch (group) {
    case 'category': return filter.categories;
    case 'level': return filter.levels;
    case 'type': return filter.types;
    case 'material': return filter.materials;
  }
}

/** The filter with one facet key added or removed (the panel's core press). */
export function toggleBimFacet(
  filter: BimFilter,
  group: BimFacetGroupKind,
  key: unknown,
): BimFilter {
  if (typeof key !== 'string' || key.trim().length === 0) return filter;
  const list = groupList(filter, group);
  const next = list.includes(key) ? list.filter(entry => entry !== key) : [...list, key];
  switch (group) {
    case 'category': return { ...filter, categories: next };
    case 'level': return { ...filter, levels: next };
    case 'type': return { ...filter, types: next };
    case 'material': return { ...filter, materials: next };
  }
}

/** Structural equality for two filters (order-sensitive within a facet group). */
export function sameBimFilter(a: BimFilter | null | undefined, b: BimFilter | null | undefined): boolean {
  const left = buildBimQuery(a).filter;
  const right = buildBimQuery(b).filter;
  return JSON.stringify(left) === JSON.stringify(right);
}

// ------------------------------------------------------------------- pages ---

/** One page's placement, derived from the host's total/offset/limit. */
export interface BimPageInfo {
  /** 1-based page number, or `0` when there is nothing to show. */
  readonly page: number;
  /** Number of pages, or `0` when there is nothing to show. */
  readonly pages: number;
  /** 1-based index of the first visible row, or `0` for an empty range. */
  readonly first: number;
  /** 1-based index of the last visible row, or `0` for an empty range. */
  readonly last: number;
  readonly total: number;
  readonly hasPrev: boolean;
  readonly hasNext: boolean;
}

/**
 * Derive one page from the host's own numbers.
 *
 * The honest edges:
 *  * `total = 0` — no rows, no pages (`page`/`pages` are `0`, so the footer is
 *    not offered rather than showing "page 1 / 0");
 *  * `limit = 0` — the host answered a page with no capacity. Nothing is
 *    visible, there is no next page, but the total is still reported;
 *  * `offset >= total` — a stale offset after the document shrank. The range is
 *    empty and Go Previous stays available so the user can get back.
 */
export function bimPageInfo(total: unknown, offset: unknown, limit: unknown): BimPageInfo {
  const size = countOf(total, 0);
  const start = countOf(offset, 0);
  const pageSize = countOf(limit, 0);
  if (size === 0) {
    return { page: 0, pages: 0, first: 0, last: 0, total: 0, hasPrev: false, hasNext: false };
  }
  const pages = pageSize === 0 ? 1 : Math.max(1, Math.ceil(size / pageSize));
  const page = pageSize === 0 ? 1 : Math.min(pages, Math.floor(start / pageSize) + 1);
  if (pageSize === 0 || start >= size) {
    return {
      page,
      pages,
      first: 0,
      last: 0,
      total: size,
      hasPrev: start > 0,
      hasNext: false,
    };
  }
  const first = start + 1;
  const last = Math.min(size, start + pageSize);
  return {
    page,
    pages,
    first,
    last,
    total: size,
    hasPrev: start > 0,
    hasNext: start + pageSize < size,
  };
}

/** The offset of the previous page (never negative). */
export function previousPageOffset(offset: unknown, limit: unknown): number {
  const start = countOf(offset, 0);
  const pageSize = countOf(limit, 0);
  if (pageSize === 0) return 0;
  return Math.max(0, start - pageSize);
}

/** The offset of the next page, or the current offset when there is none. */
export function nextPageOffset(total: unknown, offset: unknown, limit: unknown): number {
  const info = bimPageInfo(total, offset, limit);
  const start = countOf(offset, 0);
  return info.hasNext ? start + countOf(limit, 0) : start;
}

// ------------------------------------------------------------- refusals ------

/** The three kinds a failed BIM command can be. */
export type BimRefusal = 'unsupported' | 'waiting' | 'error';

/** The raw text of an IPC failure, never empty. */
export function reasonText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? '');
  return text.trim() || 'unknown';
}

/** The `cad_waiting` spelling (with or without a detail suffix). */
const WAITING_RE = /^cad_waiting(?:\s*:|$)/;

/** The `unsupported-runtime: bimQuery` spelling (a trailing detail is allowed). */
const UNSUPPORTED_RE = /^unsupported-runtime:\s*bimQuery(?:\s*:|$)/;

/** The host's "the query is bound to another document" marker (`BIM_STALE`). */
const STALE_RE = /^bim\.stale(?:\s*:|$)/;

/**
 * Whether a refusal is the host's stale-query marker.
 *
 * `cad_bim_query` answers a successful page with `stale: false`; a page bound to
 * another document is an **error** named `bim.stale`. The panel keeps its rows
 * and turns that into the same "document changed, refresh" banner the `stale`
 * field produces, because the user's only correct reaction is the same.
 */
export function isBimStaleRefusal(reason: unknown): boolean {
  return STALE_RE.test(String(reason ?? '').trim());
}

/**
 * Classify one refusal.
 *
 * Only the two stable markers are recognised; everything else (including a
 * carrier answering `unsupported_on_surface_mode: …`) is an ordinary error whose
 * words the panel prints verbatim.
 */
export function bimRefusal(reason: unknown): BimRefusal {
  const text = String(reason ?? '').trim();
  if (UNSUPPORTED_RE.test(text)) return 'unsupported';
  if (WAITING_RE.test(text)) return 'waiting';
  return 'error';
}

// ----------------------------------------------------------------- parsing ---

/** One row's `object` identity, or `null` when it cannot be addressed. */
function parseObject(value: unknown): BimObjectRef | null {
  const fields = recordOf(value);
  if (!fields) return null;
  if (fields.kind === 'entity') {
    const entityId = safeNumber(fields.entityId);
    return entityId !== null && entityId >= 0 ? { kind: 'entity', entityId } : null;
  }
  if (fields.kind === 'source') {
    const connector = textOrNull(fields.connector);
    const documentId = textOrNull(fields.documentId);
    const id = textOrNull(fields.id);
    if (!connector || !documentId || !id) return null;
    return { kind: 'source', connector, documentId, id };
  }
  return null;
}

/** The row's provenance block, or `null` when it names no connector. */
function parseSource(value: unknown): BimSource | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const connector = textOrNull(fields.connector);
  if (!connector) return null;
  return {
    connector,
    documentId: textOrNull(fields.documentId),
    id: textOrNull(fields.id),
    nativeId: textOrNull(fields.nativeId),
  };
}

/** The row's `entityIds`: every safe non-negative integer, de-duplicated. */
function parseEntityIds(value: unknown): number[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<number>();
  const ids: number[] = [];
  for (const raw of value) {
    const id = safeNumber(raw);
    if (id === null || id < 0 || seen.has(id)) continue;
    seen.add(id);
    ids.push(id);
  }
  return ids;
}

function parsePropertySummary(value: unknown): BimPropertySummaryEntry[] {
  if (!Array.isArray(value)) return [];
  const entries: BimPropertySummaryEntry[] = [];
  for (const raw of value) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const name = textOrNull(fields.name);
    if (!name) continue;
    entries.push({ name, count: countOf(fields.count, 0) });
  }
  return entries;
}

function parseRow(value: unknown): BimRow | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const object = parseObject(fields.object);
  if (!object) return null;
  return {
    object,
    entityIds: parseEntityIds(fields.entityIds),
    category: textOrNull(fields.category),
    name: textOrNull(fields.name),
    levelName: textOrNull(fields.levelName),
    typeName: textOrNull(fields.typeName),
    materialName: textOrNull(fields.materialName),
    source: parseSource(fields.source),
    evidence: textOrNull(fields.evidence),
    propertySummary: parsePropertySummary(fields.propertySummary),
  };
}

/**
 * Parse one `cad_bim_query` answer, or `null` when the payload is unusable.
 *
 * `null` is a **contract failure** the store reports (`bim.read_failed`), never
 * an empty page: an answer with no `rows` array cannot be read as "no rows",
 * because that would turn a broken reply into a successful empty filter.
 * Individual malformed rows are skipped (a row with no addressable object is
 * not rendered), which is the same choice the layers palette makes for a row
 * with no name.
 */
export function parseBimQueryAnswer(value: unknown): BimQueryAnswer | null {
  const fields = recordOf(value);
  if (!fields) return null;
  if (!Array.isArray(fields.rows)) return null;
  const total = safeNumber(fields.total);
  if (total === null || total < 0) return null;
  const rows: BimRow[] = [];
  for (const raw of fields.rows) {
    const row = parseRow(raw);
    if (row) rows.push(row);
  }
  return {
    databaseId: safeNumber(fields.databaseId),
    revision: safeNumber(fields.revision),
    total,
    offset: countOf(fields.offset, 0),
    limit: countOf(fields.limit, BIM_PAGE_SIZE),
    stale: fields.stale === true,
    rows,
  };
}

function parseFacets(value: unknown): BimFacet[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  const facets: BimFacet[] = [];
  for (const raw of value) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const key = textOrNull(fields.key);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    facets.push({ key, name: textOrNull(fields.name), count: countOf(fields.count, 0) });
  }
  return facets;
}

/**
 * Parse one `cad_bim_summary` answer, or `null` when it is unusable.
 *
 * The four facet lists are the host's own lists; a missing list is an empty
 * group, not a fabricated one. The total is the outer `total` when it is a
 * usable number, otherwise the summary's own total (the two are the same fact;
 * the outer field is the command's documented answer).
 */
export function parseBimSummaryAnswer(value: unknown): BimSummaryAnswer | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const summary = recordOf(fields.summary);
  if (!summary) return null;
  // The inner total is the summary's own documented field; a block without it is
  // not the frozen shape (an empty facet list is a real answer, a missing total
  // is not).
  const inner = safeNumber(summary.total);
  if (inner === null || inner < 0) return null;
  const total = safeNumber(fields.total) ?? inner;
  if (total < 0) return null;
  return {
    databaseId: safeNumber(fields.databaseId),
    revision: safeNumber(fields.revision),
    total,
    summary: {
      total: inner,
      byCategory: parseFacets(summary.byCategory),
      byLevel: parseFacets(summary.byLevel),
      byType: parseFacets(summary.byType),
      byMaterial: parseFacets(summary.byMaterial),
    },
  };
}

/** The stable key one row is rendered (and paginated) under. */
export function bimRowKey(row: BimRow): string {
  const object = row.object;
  return object.kind === 'source'
    ? `source:${object.connector}:${object.documentId}:${object.id}`
    : `entity:${object.entityId}`;
}

/** The entity ids a row selects (deduplicated, safe integers only). */
export function bimSelectionIds(row: BimRow | null | undefined): number[] {
  return parseEntityIds(row?.entityIds);
}
