// The BIM panel's view model: one `cad_bim_query` row / `cad_bim_summary`
// answer projected into exactly what the panel renders.
//
// Pure (no React, no DOM), so the "unset" rules, the evidence labels and the
// empty-vs-no-match distinction are unit-tested instead of being asserted on a
// rendered tree.
//
// Two rules:
//
//  * **A null field is a real state, not a blank cell.** `category`, `name`,
//    `levelName`, `typeName` and `materialName` may be `null` (the element
//    declared no such value). The view answers `unset: true` and the component
//    prints the catalog's `bim.value.unset`; it never prints an empty string and
//    never the literal word `null`.
//  * **Evidence is a machine token.** `declared`/`heuristic` map to catalog
//    sentences; any other token is shown verbatim (the host's own word), never
//    guessed at.
import type {
  BimFacet,
  BimFacetGroupKind,
  BimFilter,
  BimPhase,
  BimQueryAnswer,
  BimRow,
  BimSummary,
} from './bim-query.ts';
import { bimEvidenceKey, bimFilterActive, bimRowKey, sameBimFilter } from './bim-query.ts';

/** The catalog key every unset optional field renders through. */
export const BIM_UNSET_KEY = 'bim.value.unset';

/** One row field as the view shows it. */
export interface BimFieldView {
  /** The host's own text, or `''` when it sent none. */
  readonly value: string;
  /** `true` when the host sent `null`/blank: render [`BIM_UNSET_KEY`]. */
  readonly unset: boolean;
}

/** Project one optional row field. */
export function bimField(value: string | null | undefined): BimFieldView {
  return typeof value === 'string' && value.trim().length > 0
    ? { value, unset: false }
    : { value: '', unset: true };
}

/** One evidence chip: a catalog key, or the host's raw token. */
export interface BimEvidenceView {
  /** The catalog key when this is a known evidence token, else `null`. */
  readonly key: string | null;
  /** The raw token, or `null` when the row sent none. */
  readonly raw: string | null;
}

/** Project one row's evidence token. */
export function bimEvidenceView(evidence: string | null | undefined): BimEvidenceView {
  const raw = typeof evidence === 'string' && evidence.trim().length > 0 ? evidence : null;
  return { key: bimEvidenceKey(raw), raw };
}

/** One logical component as the list renders it. */
export interface BimRowView {
  /** Stable identity for React and the row list. */
  readonly key: string;
  readonly category: BimFieldView;
  readonly name: BimFieldView;
  readonly level: BimFieldView;
  readonly type: BimFieldView;
  readonly material: BimFieldView;
  readonly evidence: BimEvidenceView;
  /** `Pset (count)` entries, in the host's order. */
  readonly properties: readonly string[];
  /** How many geometry rows this one component carries (1 for a single mesh). */
  readonly entityCount: number;
  /** Whether a press can ask the host to select it. */
  readonly selectable: boolean;
}

/** Project one row. */
export function bimRowView(row: BimRow): BimRowView {
  return {
    key: bimRowKey(row),
    category: bimField(row.category),
    name: bimField(row.name),
    level: bimField(row.levelName),
    type: bimField(row.typeName),
    material: bimField(row.materialName),
    evidence: bimEvidenceView(row.evidence),
    properties: row.propertySummary.map(entry => `${entry.name} (${entry.count})`),
    entityCount: row.entityIds.length,
    selectable: row.entityIds.length > 0,
  };
}

/** Project a whole page. */
export function bimRowViews(rows: readonly BimRow[] | null | undefined): BimRowView[] {
  return (Array.isArray(rows) ? rows : []).map(bimRowView);
}

/** One facet group as the panel renders it (label key plus sorted chips). */
export interface BimFacetGroupView {
  readonly kind: BimFacetGroupKind;
  readonly labelKey: string;
  readonly facets: readonly BimFacet[];
}

/** The catalogue label of one facet group. */
export function bimFacetLabelKey(kind: BimFacetGroupKind): string {
  switch (kind) {
    case 'category': return 'bim.facet.category';
    case 'level': return 'bim.facet.level';
    case 'type': return 'bim.facet.type';
    case 'material': return 'bim.facet.material';
  }
}

/**
 * Sort one facet list for display: biggest first, then by display name, then by
 * key. The host's own order is not meaningful to a reader, and a count-sorted
 * list answers "what is in this drawing?" in one look.
 */
export function sortBimFacets(facets: readonly BimFacet[]): BimFacet[] {
  return [...facets].sort((a, b) => {
    if (a.count !== b.count) return b.count - a.count;
    const left = (a.name ?? a.key).toLowerCase();
    const right = (b.name ?? b.key).toLowerCase();
    if (left !== right) return left < right ? -1 : 1;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
}

/**
 * The four groups, in the panel's fixed order. A group with no facets is kept
 * out: an empty heading would be chrome with nothing behind it. The other three
 * stay visible, so `summary.total` and every count that exists are on screen
 * together.
 */
export function bimFacetGroups(summary: BimSummary | null | undefined): BimFacetGroupView[] {
  if (!summary) return [];
  const groups: Array<[BimFacetGroupKind, readonly BimFacet[]]> = [
    ['category', summary.byCategory],
    ['level', summary.byLevel],
    ['type', summary.byType],
    ['material', summary.byMaterial],
  ];
  const views: BimFacetGroupView[] = [];
  for (const [kind, facets] of groups) {
    const sorted = sortBimFacets(facets);
    if (sorted.length === 0) continue;
    views.push({ kind, labelKey: bimFacetLabelKey(kind), facets: sorted });
  }
  return views;
}

/** The distinct states the result area can be in (see [`BimPhase`]). */
export type BimResultPhase = BimPhase;

/** The catalog copy one empty result renders. */
export interface BimEmptyCopy {
  readonly titleKey: string;
  readonly hintKey: string;
}

/**
 * The sentence pair for an empty result — the one place the panel distinguishes
 * "this drawing has no BIM data" from "your filter matched nothing".
 *
 * `summaryTotal` is the **unfiltered** total (the summary command is always
 * asked without the panel's filter). `0` there means the document itself has no
 * BIM components, which is not the same fact as a filter that excluded all of
 * them. Anything else is only empty because of the filter when the filter is
 * active; a host that answered `total: 0` with no filter is still "no data".
 */
export function bimEmptyCopy(
  phase: BimResultPhase,
  filter: BimFilter | null | undefined,
  summaryTotal: number | null,
  queryTotal: number,
): BimEmptyCopy | null {
  if (phase === 'filtered-empty' || (phase === 'empty' && queryTotal === 0
    && summaryTotal !== 0 && bimFilterActive(filter))) {
    return { titleKey: 'bim.no_match', hintKey: 'bim.no_match_hint' };
  }
  if (phase === 'empty') {
    return { titleKey: 'bim.empty', hintKey: 'bim.empty_hint' };
  }
  return null;
}

/** The total line: the document's BIM count, plus the filtered count when one is applied. */
export interface BimTotalsView {
  /** The unfiltered component total (`cad_bim_summary`'s `total`). */
  readonly total: number;
  /** The filtered total, or `null` when no filter is applied. */
  readonly matched: number | null;
}

/**
 * Project the totals header from the summary and the current page.
 *
 * `pageFilter` is the filter the page was actually answered for. When it differs
 * from the filter on screen (a chip was just pressed and its request is still in
 * flight, or the request failed), the filtered count is withheld rather than
 * labelling an old page with a new filter's name. The document total stays: it
 * comes from the unfiltered summary and is always current.
 */
export function bimTotalsView(
  summaryTotal: number | null,
  filter: BimFilter | null | undefined,
  query: Pick<BimQueryAnswer, 'total'> | null,
  pageFilter: BimFilter | null | undefined = filter,
): BimTotalsView {
  const total = summaryTotal ?? query?.total ?? 0;
  const samePage = sameBimFilter(filter, pageFilter);
  const matched = bimFilterActive(filter) && samePage ? (query?.total ?? 0) : null;
  return { total, matched };
}
