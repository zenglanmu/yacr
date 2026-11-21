// BIM browse panel: filter → summary → component table (ADR-0042 Phase 3 UI).
//
// It is a non-modal floating palette window on the desktop band and a bottom
// sheet on the compact band — the same component, the same rows, exactly like
// the references and fonts panels. It never takes layout space: the desktop
// window floats in `.palette-layer`, so opening it does not change the CAD
// partition rect and the host never re-presents the drawing.
//
// # Reading the panel top to bottom
//
//  1. **Totals** — the document's BIM component count (`cad_bim_summary`, always
//     asked **unfiltered**) and, when a filter is on, the filtered count from the
//     page.
//  2. **Facets** — category / level / type / material chips, each carrying its
//     count. A press filters by that value; a second press removes it. The
//     values are the host's own facet list, never a hard-coded IFC category
//     table.
//  3. **Property filter** — one `pset.property op value` term (eq / ne /
//     contains). The value's kind is inferred as the user types (text / integer
//     / real / bool) and shown as a small tag, so the wire shape is visible.
//  4. **Table** — one row per logical component: category, name, level, type,
//     material, evidence and the `Pset (count)` summary. Pressing a row asks the
//     host to select **all** of the component's geometry rows; one source
//     component split into several meshes is still one row and one press.
//  5. **Pager** — fixed 100-row pages with the host's own total.
//
// # States that are deliberately different
//
//  * `unsupported` — a carrier with no BIM query channel answers
//    `unsupported-runtime: bimQuery`. The panel says so and shows the **raw
//    marker**; it never renders an empty table (that would read as "no BIM
//    data") and never fabricates one.
//  * `waiting` — the host has no session yet (`cad_waiting`): still loading, not
//    an empty drawing.
//  * `empty` — the summary itself says the document has 0 BIM components.
//  * `filtered-empty` — the document has components, the current filter matched
//    none. Its copy offers "clear filters".
//  * `stale` — the host flagged the page as built before a document change; a
//    banner offers a refresh and the rows stay visible, because they are still
//    the best answer available.
//
// Filter changes are debounced by the store (150 ms) and superseded (an older
// reply is discarded), so a burst of typing sends one request and the table
// never jumps back to an earlier result.
import React, { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '../host/invoke.ts';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import {
  BIM_PAGE_SIZE,
  BIM_UNSUPPORTED_MARKER,
  bimFacetActive,
  bimFilterActive,
  bimPageInfo,
  bimValueKindKey,
  bimOperatorKey,
  bimDraftValueKind,
  buildBimPropertyFilter,
  bimPropertyDraft,
  EMPTY_BIM_PROPERTY_DRAFT,
  sameBimFilter,
} from '../bim/bim-query.ts';
import type { BimPropertyDraft } from '../bim/bim-query.ts';
import {
  BIM_UNSET_KEY,
  bimEmptyCopy,
  bimFacetGroups,
  bimRowViews,
  bimTotalsView,
} from '../bim/bim-view.ts';
import type { BimRowView } from '../bim/bim-view.ts';
import { createBimStore } from '../bim/bim-store.ts';
import type { BimStore } from '../bim/bim-store.ts';
import { closeBimPanel } from '../bim/panel.ts';

/** Desktop palette window, or the compact bottom sheet. */
export type BimPanelVariant = 'window' | 'floating';

export interface BimPanelProps {
  readonly variant?: BimPanelVariant;
}

/** The skeleton's row bars. */
const SKELETON_ROWS = [0, 1, 2, 3];

/** One labelled field pair in a row's context column. */
function RowField({ label, value }: { readonly label: string; readonly value: BimRowView['level'] }) {
  const { t } = useI18n();
  return (
    <span className="bim__pair">
      <span className="bim__pair-label">{label}</span>
      <span className={['bim__pair-value', value.unset ? 'is-unset' : ''].filter(Boolean).join(' ')}>
        {value.unset ? t(BIM_UNSET_KEY) : value.value}
      </span>
    </span>
  );
}

/** One component row: a real button, so Tab reaches every row and Enter selects. */
function ComponentRow({
  row,
  view,
  onSelect,
}: {
  readonly row: { readonly entityIds: readonly number[] };
  readonly view: BimRowView;
  readonly onSelect: () => void;
}) {
  const { t } = useI18n();
  const unset = t(BIM_UNSET_KEY);
  const title = view.name.unset ? t('bim.row.select_hint') : `${view.name.value} — ${t('bim.row.select_hint')}`;
  const hasTags = view.evidence.raw !== null || row.entityIds.length > 1 || view.properties.length > 0;
  return (
    <li className="bim__item">
      <button
        type="button"
        className="bim__row"
        title={title}
        aria-disabled={view.selectable ? undefined : true}
        onClick={() => { if (view.selectable) onSelect(); }}
      >
        <span className="bim__row-main">
          <span className={['bim__category', view.category.unset ? 'is-unset' : ''].filter(Boolean).join(' ')}>
            {view.category.unset ? unset : view.category.value}
          </span>
          <span className={['bim__name', view.name.unset ? 'is-unset' : ''].filter(Boolean).join(' ')}>
            {view.name.unset ? unset : view.name.value}
          </span>
        </span>
        <span className="bim__row-context">
          <RowField label={t('bim.field.level')} value={view.level} />
          <RowField label={t('bim.field.type')} value={view.type} />
          <RowField label={t('bim.field.material')} value={view.material} />
        </span>
        {hasTags ? (
          <span className="bim__row-tags">
            {view.evidence.raw === null ? null : (
              <span className={['bim__tag', view.evidence.key ? 'is-evidence' : 'is-raw'].filter(Boolean).join(' ')}>
                {view.evidence.key ? t(view.evidence.key) : view.evidence.raw}
              </span>
            )}
            {row.entityIds.length > 1 ? (
              <span className="bim__tag is-parts" title={t('bim.row.parts_hint', { count: view.entityCount })}>
                {t('bim.row.parts', { count: view.entityCount })}
              </span>
            ) : null}
            {view.properties.length > 0 ? (
              <span className="bim__tag is-props" title={view.properties.join('\n')}>
                <Icon name="properties" size={11} />
                <span>{view.properties.join(' · ')}</span>
              </span>
            ) : null}
          </span>
        ) : null}
      </button>
    </li>
  );
}

export function BimPanel({ variant = 'window' }: BimPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const storeRef = useRef<BimStore | null>(null);
  if (!storeRef.current) storeRef.current = createBimStore(invoke);
  const store = storeRef.current;
  const state = useSyncExternalStore(store.subscribe, store.getState);

  const [propertyOpen, setPropertyOpen] = useState(false);
  const [draft, setDraft] = useState<BimPropertyDraft>(EMPTY_BIM_PROPERTY_DRAFT);
  const [draftError, setDraftError] = useState<string | null>(null);
  const rootRef = useRef<HTMLElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);
  const psetRef = useRef<HTMLInputElement | null>(null);
  const propertyTriggerRef = useRef<HTMLButtonElement | null>(null);
  const restorePropertyFocus = useRef(false);

  // Open reads once, and unmounting drops a pending debounce so a closed panel
  // cannot publish.
  useEffect(() => {
    void store.load();
    return () => store.dispose();
  }, [store]);

  // A different drawing is the only honest "the old rows describe another
  // document" signal; the status poll itself must not re-read.
  const documentLabel = shell.documentLabel ?? null;
  const seenDocument = useRef(documentLabel);
  useEffect(() => {
    if (seenDocument.current === documentLabel) return;
    seenDocument.current = documentLabel;
    store.reset();
    void store.load();
  }, [documentLabel, store]);

  // Escape closes the panel while its own tree (or nothing in particular) has
  // the keyboard. A field in the panel stops its own Escape first when it is a
  // one-step cancel (the property form closes instead of the panel). A host
  // dialog or a pending plugin prompt owns Escape, so the panel stands down.
  const popupOwnsEscape = shell.popupOpen || shell.pluginPrompt !== null;
  useEffect(() => {
    if (popupOwnsEscape) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) return;
      const root = rootRef.current;
      const active = document.activeElement;
      if (root && active && active !== document.body && !root.contains(active)) return;
      closeBimPanel();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [popupOwnsEscape]);

  // A new page or filter starts at the top of the table, not wherever the old
  // list happened to be scrolled.
  const pageOffset = state.page?.offset ?? 0;
  useEffect(() => {
    listRef.current?.scrollTo?.({ top: 0 });
  }, [pageOffset, state.filter]);

  // Focus follows the property form: the first field on open, the trigger on close.
  useEffect(() => {
    if (propertyOpen) {
      psetRef.current?.focus?.();
      return;
    }
    if (!restorePropertyFocus.current) return;
    restorePropertyFocus.current = false;
    propertyTriggerRef.current?.focus?.();
  }, [propertyOpen]);

  const views = useMemo(() => bimRowViews(state.rows), [state.rows]);
  const groups = useMemo(() => bimFacetGroups(state.summary), [state.summary]);
  const filterActive = bimFilterActive(state.filter);
  const totals = bimTotalsView(state.summary?.total ?? null, state.filter, state.page, state.queriedFilter);
  const info = bimPageInfo(state.page?.total ?? 0, state.page?.offset ?? 0, state.page?.limit ?? BIM_PAGE_SIZE);
  const firstRead = (state.phase === 'idle' || state.phase === 'loading') && state.page === null;
  // The host answers "no document" with an empty page (a real answer), so the
  // summary alone cannot tell a BIM-free drawing from no drawing at all. The
  // layout fact (`documentVisible`, which also covers a `dom-canvas` session's
  // relayed identity) can, and it changes only the copy here.
  const emptyCopy = shell.documentVisible
    ? bimEmptyCopy(state.phase, state.filter, state.summary?.total ?? null, state.page?.total ?? 0)
    : { titleKey: 'plot.no_document', hintKey: 'bim.empty_hint' };
  const applied = state.filter.property;
  // The rows on screen belong to this filter. Until a new page answers, the
  // pager is hidden: its numbers describe the old filter, and pressing it would
  // page the new one at an offset it never had.
  const pageIsCurrent = sameBimFilter(state.filter, state.queriedFilter);
  const propertyKind = bimDraftValueKind(draft);
  const floating = variant === 'floating';
  const busy = state.busy;

  const refresh = () => { if (!busy) void store.refresh(); };

  const toggleProperty = () => {
    if (propertyOpen) {
      restorePropertyFocus.current = true;
      setPropertyOpen(false);
      return;
    }
    setDraft(bimPropertyDraft(applied));
    setDraftError(null);
    setPropertyOpen(true);
  };

  const closeProperty = () => {
    setDraftError(null);
    restorePropertyFocus.current = true;
    setPropertyOpen(false);
  };

  const applyProperty = () => {
    const filter = buildBimPropertyFilter(draft);
    if (!filter) {
      setDraftError('bim.property.invalid');
      psetRef.current?.focus?.();
      return;
    }
    setDraftError(null);
    restorePropertyFocus.current = true;
    setPropertyOpen(false);
    store.setFilter({ ...state.filter, property: filter });
  };

  const removeProperty = () => {
    setDraftError(null);
    setDraft(EMPTY_BIM_PROPERTY_DRAFT);
    store.setFilter({ ...state.filter, property: null });
  };

  const onFormKeyDown = (event: React.KeyboardEvent<HTMLFormElement>) => {
    if (event.key !== 'Escape' || event.nativeEvent.isComposing) return;
    event.preventDefault();
    event.stopPropagation();
    closeProperty();
  };

  const wentPrevious = () => {
    if (busy) return;
    store.setPage(Math.max(0, (state.page?.offset ?? 0) - (state.page?.limit ?? BIM_PAGE_SIZE)));
  };

  const goNext = () => {
    if (busy || !info.hasNext) return;
    store.setPage((state.page?.offset ?? 0) + (state.page?.limit ?? BIM_PAGE_SIZE));
  };

  const close = () => closeBimPanel();

  /** Merge one controlled field into the draft and clear a stale validation note. */
  const setDraftEntry = (entry: Partial<BimPropertyDraft>): void => {
    setDraft(current => ({ ...current, ...entry }));
    if (draftError !== null) setDraftError(null);
  };

  return (
    <section
      ref={rootRef}
      className={['palette', 'bim', floating ? 'bim--floating' : '', busy ? 'is-busy' : ''].filter(Boolean).join(' ')}
      aria-label={t('bim.panel')}
      role={floating ? 'dialog' : undefined}
      aria-modal={floating ? true : undefined}
    >
      <header className="palette__head">
        <h2><Icon name="bim" size={15} />{t('bim.panel')}</h2>
        <span className="bim__head-actions">
          <IconButton
            icon="refresh"
            size="xs"
            iconSize={14}
            label={t('bim.refresh')}
            aria-disabled={busy || undefined}
            onClick={refresh}
          />
          <IconButton icon="close" size="xs" label={t('bim.close')} onClick={close} />
        </span>
      </header>

      {state.phase === 'unsupported' ? (
        <div className="palette__body bim__body">
          <div className="bim__notice is-unavailable" role="status">
            <Icon name="alert" size={20} />
            <p className="bim__notice-title">{t('bim.unavailable.title')}</p>
            <p className="bim__notice-detail">{t('bim.unavailable.detail')}</p>
            <code className="bim__marker">{state.error?.reason ?? BIM_UNSUPPORTED_MARKER}</code>
            <Button variant="ghost" size="xs" icon="refresh" iconSize={13} aria-disabled={busy || undefined} onClick={refresh}>
              {t('bim.retry')}
            </Button>
          </div>
        </div>
      ) : null}

      {firstRead ? (
        <div className="palette__body bim__body" aria-busy="true">
          <div className="bim__skeleton" aria-hidden="true">
            <span className="bim__skeleton-total" />
            <span className="bim__skeleton-chips" />
            <span className="bim__skeleton-chips is-short" />
            {SKELETON_ROWS.map(index => <span key={index} className="bim__skeleton-row" />)}
          </div>
        </div>
      ) : null}

      {state.phase === 'waiting' ? (
        <div className="palette__body bim__body">
          <div className="bim__notice" role="status">
            <Icon name="spinner" size={18} className="spin" />
            <p className="bim__notice-detail">{t('host.poll_not_ready')}</p>
            <Button variant="ghost" size="xs" icon="refresh" iconSize={13} aria-disabled={busy || undefined} onClick={refresh}>
              {t('bim.retry')}
            </Button>
          </div>
        </div>
      ) : null}

      {state.phase === 'error' && state.summary === null && state.rows.length === 0 ? (
        <div className="palette__body bim__body">
          <div className="bim__notice is-error" role="alert">
            <Icon name="alert" size={18} />
            <p className="bim__notice-detail">{t('bim.read_failed', { reason: state.error?.reason ?? '' })}</p>
            <Button variant="ghost" size="xs" icon="refresh" iconSize={13} aria-disabled={busy || undefined} onClick={refresh}>
              {t('bim.retry')}
            </Button>
          </div>
        </div>
      ) : null}

      {state.phase !== 'unsupported' && state.phase !== 'waiting'
        && !firstRead
        && !(state.phase === 'error' && state.summary === null && state.rows.length === 0) ? (
        <>
          <div className="bim__filters">
            <div className="bim__totals">
              <span className="bim__total">{t('bim.summary.total', { count: totals.total })}</span>
              {totals.matched !== null ? (
                <span className="bim__matched">{t('bim.summary.matched', { count: totals.matched })}</span>
              ) : null}
              {filterActive ? (
                <Button
                  variant="ghost"
                  size="xs"
                  className="bim__clear"
                  onClick={() => store.clearFilter()}
                >
                  {t('bim.filter.clear')}
                </Button>
              ) : null}
            </div>

            {groups.length > 0 ? (
              <div className="bim__facets">
                {groups.map(group => (
                  <div className="bim__group" key={group.kind}>
                    <span className="bim__group-label">{t(group.labelKey)}</span>
                    <div className="bim__chips" role="group" aria-label={t(group.labelKey)}>
                      {group.facets.map(facet => {
                        const name = facet.name ?? t(BIM_UNSET_KEY);
                        const active = bimFacetActive(state.filter, group.kind, facet.key);
                        return (
                          <button
                            key={facet.key}
                            type="button"
                            className={['bim__chip', active ? 'is-on' : ''].filter(Boolean).join(' ')}
                            aria-pressed={active}
                            title={t(active ? 'bim.facet.remove' : 'bim.facet.filter_hint', { name })}
                            onClick={() => store.toggleFacet(group.kind, facet.key)}
                          >
                            <span className="bim__chip-name">{name}</span>
                            <span className="bim__chip-count">{facet.count}</span>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
            ) : null}

            <div className="bim__property-bar">
              <Button
                ref={propertyTriggerRef}
                variant="ghost"
                size="xs"
                icon="filter"
                iconSize={13}
                aria-expanded={propertyOpen}
                onClick={toggleProperty}
              >
                {t('bim.property.title')}
              </Button>
              {applied ? (
                <span className="bim__applied">
                  <span className="bim__applied-text" title={t('bim.property.applied', {
                    pset: applied.pset,
                    property: applied.property,
                    op: t(bimOperatorKey(applied.op)),
                    value: String(applied.value.value),
                  })}>
                    {t('bim.property.applied', {
                      pset: applied.pset,
                      property: applied.property,
                      op: t(bimOperatorKey(applied.op)),
                      value: String(applied.value.value),
                    })}
                  </span>
                  <IconButton
                    icon="close"
                    size="xs"
                    iconSize={12}
                    label={t('bim.property.remove')}
                    onClick={removeProperty}
                  />
                </span>
              ) : null}
            </div>

            {propertyOpen ? (
              <form className="bim__form" aria-label={t('bim.property.title')} onKeyDown={onFormKeyDown} onSubmit={event => {
                event.preventDefault();
                applyProperty();
              }}>
                <label className="bim__field">
                  <span className="bim__field-label">{t('bim.property.pset')}</span>
                  <input
                    ref={psetRef}
                    type="text"
                    className="bim__input"
                    value={draft.pset}
                    placeholder={t('bim.property.pset_placeholder')}
                    aria-invalid={draftError ? true : undefined}
                    autoComplete="off"
                    spellCheck={false}
                    onChange={event => { setDraftEntry({ pset: event.target.value }); }}
                  />
                </label>
                <label className="bim__field">
                  <span className="bim__field-label">{t('bim.property.name')}</span>
                  <input
                    type="text"
                    className="bim__input"
                    value={draft.property}
                    placeholder={t('bim.property.name_placeholder')}
                    aria-invalid={draftError ? true : undefined}
                    autoComplete="off"
                    spellCheck={false}
                    onChange={event => { setDraftEntry({ property: event.target.value }); }}
                  />
                </label>
                <div className="bim__field-row">
                  <label className="bim__field bim__field--op">
                    <span className="bim__field-label">{t('bim.property.op')}</span>
                    <select
                      className="bim__input bim__select"
                      value={draft.op}
                      onChange={event => { setDraftEntry({ op: event.target.value as BimPropertyDraft['op'] }); }}
                    >
                      <option value="eq">{t('bim.property.op.eq')}</option>
                      <option value="ne">{t('bim.property.op.ne')}</option>
                      <option value="contains">{t('bim.property.op.contains')}</option>
                    </select>
                  </label>
                  <label className="bim__field bim__field--value">
                    <span className="bim__field-label">
                      {t('bim.property.value')}
                      {propertyKind === null ? null : (
                        <em className="bim__kind">{t(bimValueKindKey(propertyKind))}</em>
                      )}
                    </span>
                    <input
                      type="text"
                      className="bim__input"
                      value={draft.value}
                      placeholder={t('bim.property.value_placeholder')}
                      aria-invalid={draftError ? true : undefined}
                      autoComplete="off"
                      spellCheck={false}
                      onChange={event => { setDraftEntry({ value: event.target.value }); }}
                    />
                  </label>
                </div>
                {draftError ? <p className="bim__form-error" role="alert">{t(draftError)}</p> : null}
                <div className="bim__form-actions">
                  <Button variant="ghost" size="xs" onClick={closeProperty}>{t('tool.cancel')}</Button>
                  <Button variant="solid" size="xs" type="submit">{t('bim.property.apply')}</Button>
                </div>
              </form>
            ) : null}
          </div>

          <div className="bim__result">
            {busy && state.rows.length > 0 ? <div className="bim__progress" aria-hidden="true"><span /></div> : null}

            {state.page?.stale ? (
              <div className="bim__banner is-stale" role="status">
                <Icon name="alert" size={13} />
                <span className="bim__banner-text" title={t('bim.stale')}>{t('bim.stale')}</span>
                <Button variant="ghost" size="xs" icon="refresh" iconSize={12} aria-disabled={busy || undefined} onClick={refresh}>
                  {t('bim.stale_refresh')}
                </Button>
              </div>
            ) : null}

            {state.selectionError ? (
              <div className="bim__banner is-error" role="alert">
                <span className="bim__banner-text" title={state.selectionError}>
                  {t('bim.select_failed', { reason: state.selectionError })}
                </span>
                <IconButton
                  icon="close"
                  size="xs"
                  iconSize={12}
                  label={t('bim.selection.dismiss')}
                  onClick={() => store.clearSelectionError()}
                />
              </div>
            ) : null}

            {state.error ? (
              <p className="bim__error" role="alert" title={state.error.reason}>
                {state.error.kind === 'summary'
                  ? t('bim.summary_failed', { reason: state.error.reason })
                  : t('bim.read_failed', { reason: state.error.reason })}
              </p>
            ) : null}

            {emptyCopy ? (
              <div className="bim__empty" role="status">
                <Icon name="bim" size={22} />
                <p className="bim__empty-title">{t(emptyCopy.titleKey)}</p>
                <p className="bim__empty-hint">{t(emptyCopy.hintKey)}</p>
                {filterActive ? (
                  <Button variant="ghost" size="xs" onClick={() => store.clearFilter()}>
                    {t('bim.filter.clear')}
                  </Button>
                ) : null}
              </div>
            ) : null}

            {views.length > 0 ? (
              <>
                <div className="bim__columns" aria-hidden="true">
                  <span className="bim__column bim__column--component">{t('bim.column.component')}</span>
                  <span className="bim__column bim__column--context">{t('bim.column.context')}</span>
                </div>
                <ul className="bim__rows" ref={listRef} aria-busy={busy || undefined}>
                  {state.rows.map((row, index) => {
                    const view = views[index];
                    return (
                      <ComponentRow
                        key={view.key}
                        row={row}
                        view={view}
                        onSelect={() => { void store.select(row.entityIds); }}
                      />
                    );
                  })}
                </ul>
              </>
            ) : null}

            {state.page !== null && state.page.total > 0 && pageIsCurrent ? (
              <footer className="bim__footer">
                <IconButton
                  icon="chevronLeft"
                  size="xs"
                  label={t('bim.page.prev')}
                  aria-disabled={info.hasPrev ? undefined : true}
                  onClick={() => { if (info.hasPrev) wentPrevious(); }}
                />
                <span className="bim__range">
                  <span>{info.first > 0
                    ? t('bim.page.range', { first: info.first, last: info.last, total: info.total })
                    : t('bim.page.empty')}</span>
                  {info.pages > 1 ? (
                    <span className="bim__page-index">{t('bim.page.index', { page: info.page, pages: info.pages })}</span>
                  ) : null}
                </span>
                <IconButton
                  icon="chevronRight"
                  size="xs"
                  label={t('bim.page.next')}
                  aria-disabled={info.hasNext ? undefined : true}
                  onClick={goNext}
                />
              </footer>
            ) : null}
          </div>
        </>
      ) : null}
    </section>
  );
}
