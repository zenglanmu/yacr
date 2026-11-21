// The BIM browse panel's state machine: one request per action (the answer
// carries the page and the facet summary), the debounced filter, response-race
// supersession and the selection request.
//
// A plain object, not a hook, so its behaviour is testable with a fake `invoke`
// and a fake clock — the same shape `layers/layers.ts` uses. The component
// subscribes through `useSyncExternalStore`.
//
// # One round trip
//
// `cad_bim_query` answers the page **and** the `summary` block of the same
// filtered set, so a load, a refresh or a filter change is exactly one `invoke`.
// The former two-call load (`cad_bim_summary` + `cad_bim_query`) is gone, and
// with it every chance of mixing two revisions.
//
// # The facet list is the document's value list
//
// The response's summary is bound to the request's filter (`summary.total` is
// the matched total), while the panel's facet chips must stay the **document's**
// value list: adopting a filtered summary would collapse the chips to the
// current selection ("pick a category and every other category disappears"),
// making the core interaction — press another facet — impossible. The store
// therefore adopts the answer's summary only when the answer was asked with an
// **empty** filter (the first load, a clear-filter press, a refresh with no
// filter); a filtered answer updates the rows and the page and keeps the
// document summary already on screen. The filtered count is the answer's
// `total`, shown beside the document total.
//
// # Races are discarded, not serialised
//
// A filter or page change bumps a generation counter. Every answer is applied
// only when its generation is still current, so an older in-flight request can
// never overwrite a newer one. There is no queue: the newest request is the only
// one that matters, and a 150 ms debounce already folds a burst of typing into
// one request.
//
// # Loading never flashes
//
// A first read publishes `loading` (the skeleton). A later filter/page change
// keeps the current rows on screen and only raises `busy`, so the table does not
// blink between every chip press.
import {
  BIM_PAGE_SIZE,
  BIM_PAYLOAD_INVALID,
  BIM_QUERY_COMMAND,
  buildBimQuery,
  bimFilterActive,
  bimPageInfo,
  bimRefusal,
  emptyBimFilter,
  isBimStaleRefusal,
  parseBimQueryAnswer,
  reasonText,
  sameBimFilter,
  toggleBimFacet,
} from './bim-query.ts';
import type {
  BimFacetGroupKind,
  BimFilter,
  BimPhase,
  BimQueryAnswer,
  BimRow,
  BimSummary,
  Invoke,
} from './bim-query.ts';
import { selectBimEntities } from './selection.ts';
import type { BimSelectResult } from './selection.ts';

/** The filter debounce: one request per pause, not one per keystroke. */
export const BIM_DEBOUNCE_MS = 150;

/**
 * Why a read failed.
 *
 * Since the page and the summary ride one answer, every failure the store
 * publishes is a `read`. The `summary` member is kept because the panel still
 * renders that catalog sentence for it; no store path produces it today.
 */
export interface BimPanelError {
  readonly kind: 'read' | 'summary';
  /** The host's raw reason (or `bim.invalid_payload`), never empty. */
  readonly reason: string;
}

/** The page the current rows came from. */
export interface BimPage {
  readonly total: number;
  readonly offset: number;
  readonly limit: number;
  /** The host says the document changed after this page was built. */
  readonly stale: boolean;
}

/** Everything the panel renders. */
export interface BimState {
  readonly phase: BimPhase;
  /** The applied filter (the facet chips' pressed state). */
  readonly filter: BimFilter;
  readonly rows: readonly BimRow[];
  readonly page: BimPage | null;
  readonly summary: BimSummary | null;
  /** The filter the rows on screen were answered for, or `null` before one was. */
  readonly queriedFilter: BimFilter | null;
  /** A request is in flight; rows and summary stay on screen. */
  readonly busy: boolean;
  readonly error: BimPanelError | null;
  /** The host's refusal of the last selection request, or `null`. */
  readonly selectionError: string | null;
}

/** The store the panel subscribes to. */
export interface BimStore {
  getState(): BimState;
  subscribe(listener: () => void): () => void;
  /** First read (page 1): rows and summary in one request. */
  load(): Promise<void>;
  /** Drop every document fact (a different drawing opened); a pending read is superseded. */
  reset(): void;
  /** Re-read the current page (the stale banner's refresh), rows and summary in one request. */
  refresh(): Promise<void>;
  /** Replace the filter; the query is debounced and resets to page 1. */
  setFilter(filter: BimFilter): void;
  /** Add or remove one facet value (the summary's chip press). */
  toggleFacet(group: BimFacetGroupKind, key: string): void;
  /** Drop every facet, the property filter and the evidence filter. */
  clearFilter(): void;
  /** Go to one page offset (immediate: a press is not typing). */
  setPage(offset: number): void;
  /** Ask the host to select one component's geometry rows. */
  select(ids: readonly number[] | null | undefined): Promise<BimSelectResult>;
  /** Dismiss the selection refusal line. */
  clearSelectionError(): void;
  /** Drop the pending debounce (the panel unmounted). */
  dispose(): void;
}

/** The state before anything was read. */
export const EMPTY_BIM_STATE: BimState = Object.freeze({
  phase: 'idle',
  filter: emptyBimFilter(),
  rows: Object.freeze([]) as readonly BimRow[],
  page: null,
  summary: null,
  queriedFilter: null,
  busy: false,
  error: null,
  selectionError: null,
});

/** A one-shot read's outcome. */
type Read<T> =
  | { readonly kind: 'ok'; readonly value: T }
  | { readonly kind: 'error'; readonly reason: string };

/** The timer seam, so the store's tests never depend on wall time. */
export type BimSchedule = (run: () => void | Promise<void>, ms: number) => unknown;
export type BimCancel = (handle: unknown) => void;

/** What a caller may inject (the clock is the test seam). */
export interface BimStoreOptions {
  readonly debounceMs?: number;
  readonly schedule?: BimSchedule;
  readonly cancel?: BimCancel;
}

/** The phase one applied page answers. */
function phaseFor(
  summaryTotal: number | null,
  queryTotal: number,
  filterActive: boolean,
): BimPhase {
  if (summaryTotal === 0) return 'empty';
  if (queryTotal === 0) return filterActive ? 'filtered-empty' : 'empty';
  return 'ready';
}

/**
 * Build the panel's store over one `invoke`.
 *
 * All the rules live here so the component is layout plus local form state:
 * generation-based supersession, the debounce, the document-wide facet list, the
 * empty/no-match distinction and a selection request that never claims a
 * selection on its own. `dispose()` clears the pending timer; every later
 * answer is dropped (`disposed`), so an unmounted panel cannot publish.
 */
export function createBimStore(invoke: Invoke, options: BimStoreOptions = {}): BimStore {
  const debounceMs = typeof options.debounceMs === 'number' && options.debounceMs >= 0
    ? options.debounceMs
    : BIM_DEBOUNCE_MS;
  const schedule: BimSchedule = options.schedule
    ?? ((run, ms) => setTimeout(run, ms));
  const cancel: BimCancel = options.cancel
    ?? (handle => { clearTimeout(handle as ReturnType<typeof setTimeout>); });

  let state: BimState = EMPTY_BIM_STATE;
  let generation = 0;
  let timer: unknown = null;
  let disposed = false;
  /** The filter the rows on screen were answered for (page-offset continuity). */
  let queriedFilter: BimFilter | null = null;
  const listeners = new Set<() => void>();

  function publish(next: BimState): void {
    state = Object.freeze({ ...next });
    for (const listener of [...listeners]) listener();
  }

  function cancelTimer(): void {
    if (timer === null) return;
    cancel(timer);
    timer = null;
  }

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  async function readQuery(filter: BimFilter, offset: number, limit: number): Promise<Read<BimQueryAnswer>> {
    try {
      const answer = await invoke(BIM_QUERY_COMMAND, { ...buildBimQuery(filter, offset, limit) });
      const parsed = parseBimQueryAnswer(answer);
      return parsed ? { kind: 'ok', value: parsed } : { kind: 'error', reason: BIM_PAYLOAD_INVALID };
    } catch (error) {
      return { kind: 'error', reason: reasonText(error) };
    }
  }

  /** Apply one answer: the page always, the summary when it is the document's. */
  function applyQuery(read: Read<BimQueryAnswer>, filter: BimFilter): void {
    if (read.kind === 'error') {
      const refusal = bimRefusal(read.reason);
      if (refusal === 'unsupported') {
        queriedFilter = null;
        publish({
          ...state,
          phase: 'unsupported',
          rows: [],
          page: null,
          summary: null,
          queriedFilter: null,
          busy: false,
          error: { kind: 'read', reason: read.reason },
        });
        return;
      }
      if (refusal === 'waiting') {
        queriedFilter = null;
        publish({
          ...state,
          phase: 'waiting',
          rows: [],
          page: null,
          summary: null,
          queriedFilter: null,
          busy: false,
          error: null,
        });
        return;
      }
      if (isBimStaleRefusal(read.reason) && state.page !== null) {
        // The host says this page is bound to another document. The rows are the
        // last good answer, so they stay and the stale banner asks for a refresh.
        publish({
          ...state,
          busy: false,
          error: null,
          page: { ...state.page, stale: true },
        });
        return;
      }
      publish({
        ...state,
        phase: state.rows.length > 0 ? state.phase : 'error',
        busy: false,
        error: { kind: 'read', reason: read.reason },
      });
      return;
    }
    const answer = read.value;
    queriedFilter = filter;
    // The facet list is the document's value list, so it is adopted from an
    // **unfiltered** answer only. A filtered answer's summary is bound to that
    // filter (`summary.total === total`); adopting it would collapse the chips
    // to the current selection and hide every other value. The filtered count is
    // the page's `total`, beside the document total.
    const summary = bimFilterActive(filter) ? state.summary : answer.summary;
    publish({
      ...state,
      phase: phaseFor(summary?.total ?? null, answer.total, bimFilterActive(filter)),
      filter,
      rows: answer.rows,
      page: { total: answer.total, offset: answer.offset, limit: answer.limit, stale: answer.stale },
      summary,
      queriedFilter,
      busy: false,
      error: null,
    });
  }

  async function load(offset: number): Promise<void> {
    const current = ++generation;
    cancelTimer();
    const filter = state.filter;
    // A first read owns the skeleton; a later read keeps what is on screen.
    publish({
      ...state,
      phase: state.page === null && state.summary === null ? 'loading' : state.phase,
      busy: true,
      error: null,
    });
    // One request: the answer carries the rows and the summary of the same
    // filtered set, so the page and the counters can never come from two
    // revisions.
    const read = await readQuery(filter, offset, limitOf(state.page));
    if (disposed || current !== generation) return;
    applyQuery(read, filter);
  }

  function limitOf(page: BimPage | null): number {
    return page && page.limit > 0 ? page.limit : BIM_PAGE_SIZE;
  }

  /** Schedule (or run) one query for `filter` at `offset`. */
  function requestQuery(filter: BimFilter, offset: number, immediate: boolean): void {
    const current = ++generation;
    cancelTimer();
    const run = async (): Promise<void> => {
      timer = null;
      publish({ ...state, busy: true, error: null });
      const read = await readQuery(filter, offset, limitOf(state.page));
      if (disposed || current !== generation) return;
      applyQuery(read, filter);
    };
    if (immediate) {
      void run();
      return;
    }
    // The callback hands the run's promise back so a test clock can await the
    // request; the production `setTimeout` simply ignores it.
    timer = schedule(run, debounceMs);
  }

  function setFilter(filter: BimFilter): void {
    const next = buildBimQuery(filter).filter;
    if (sameBimFilter(state.filter, next)) return;
    publish({ ...state, filter: next });
    requestQuery(next, 0, false);
  }

  return {
    getState: () => state,
    subscribe,
    load(): Promise<void> {
      return load(0);
    },
    reset(): void {
      if (disposed) return;
      // The rows, the summary and the filter describe one document. A new
      // drawing starts from the empty state, and a read that was in flight for
      // the old one is superseded by the generation bump in `load`.
      generation += 1;
      cancelTimer();
      queriedFilter = null;
      publish({ ...EMPTY_BIM_STATE });
    },
    refresh(): Promise<void> {
      // Only a page answered for the current filter keeps its offset. A filter
      // change (even one whose debounce has not fired yet) resets the logical
      // page to 1, so refreshing must not ask the new filter for the old window.
      const sameWindow = queriedFilter !== null && sameBimFilter(queriedFilter, state.filter);
      return load(sameWindow ? state.page?.offset ?? 0 : 0);
    },
    setFilter,
    toggleFacet(group: BimFacetGroupKind, key: string): void {
      setFilter(toggleBimFacet(state.filter, group, key));
    },
    clearFilter(): void {
      if (!bimFilterActive(state.filter)) return;
      const next = emptyBimFilter();
      publish({ ...state, filter: next });
      requestQuery(next, 0, true);
    },
    setPage(offset: number): void {
      const info = bimPageInfo(state.page?.total ?? 0, offset, limitOf(state.page));
      // Only a page that exists is requested; a stale offset is clamped back to
      // the last real page instead of asking the host for an empty window.
      const limit = limitOf(state.page);
      const last = info.pages > 0 ? (info.pages - 1) * limit : 0;
      const next = Math.min(Math.max(0, Number.isSafeInteger(offset) ? offset : 0), last);
      requestQuery(state.filter, next, true);
    },
    select(ids: readonly number[] | null | undefined): Promise<BimSelectResult> {
      return selectBimEntities(invoke, ids).then(result => {
        if (disposed) return result;
        if (result.kind === 'error') {
          publish({ ...state, selectionError: result.reason });
        } else if (result.kind === 'ok' && state.selectionError !== null) {
          publish({ ...state, selectionError: null });
        }
        return result;
      });
    },
    clearSelectionError(): void {
      if (state.selectionError === null) return;
      publish({ ...state, selectionError: null });
    },
    dispose(): void {
      disposed = true;
      cancelTimer();
      listeners.clear();
    },
  };
}
