// Layers palette: the docked layer list (spec F03).
//
// Docked on the left (it shrinks the drawing area) like the properties and
// diagnostics palettes, because the host composites a native CAD surface in the
// central partition: nothing here floats across the drawing, and there is no
// backdrop, so the CAD partition keeps receiving every pointer press that is not
// on the panel itself.
//
// The rows are the host's own answer ([`layers.mjs`] owns the wire contract and
// the single-flight state machine); this file is layout plus the local search
// cursor. It never writes the drawing: a visibility toggle is a session-only
// temporary override, `restore` drops them, and "set current" only says where new
// entities go.
//
// Two deliberate choices worth naming:
//
//  * **Not a `role="listbox"`.** Every row owns two real controls (the visibility
//    toggle and "set as current"), and an ARIA option may not contain interactive
//    descendants. So the list is a plain `<ul>`/`<li>` list of native buttons:
//    `Tab` reaches every control, `Space`/`Enter` are the buttons' own activation,
//    and `ArrowUp`/`ArrowDown`/`Home`/`End` move focus between rows as a
//    convenience (the roving cursor). Nothing is announced as "selected" that is
//    not actually a selection — the *current layer* is `aria-current`, not a
//    listbox selection.
//  * **No invented copy.** The loading state is a skeleton (no "Loading…" string
//    exists in the catalog), the not-ready state uses the host's own sentence
//    (`host.poll_not_ready`), a failed *read* uses `layers.read_failed` and a
//    failed *mutation* uses `layers.operation_failed`, both carrying the host's
//    raw reason.
import React, { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from '../icons.jsx';
import { Button, IconButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { createLayersStore, filterLayers, layerTone, nextFocusIndex } from '../layers.mjs';

/** The three skeleton bars of a first read. */
const SKELETON_ROWS = [0, 1, 2];

export function LayersPalette() {
  const { t } = useI18n();
  const shell = useShell();
  // One store per mounted palette. Closing and reopening therefore starts from a
  // fresh read instead of a stale list — "refresh on open" is the mount itself.
  const storeRef = useRef(null);
  if (!storeRef.current) storeRef.current = createLayersStore(invoke);
  const store = storeRef.current;
  const state = useSyncExternalStore(store.subscribe, store.getState);

  const [query, setQuery] = useState('');
  // The roving keyboard cursor: an index into `rows`, or `-1` when no row has
  // focus yet. It is only a focus hint; the *current layer* is the host's own
  // `current` flag on a row.
  const [cursor, setCursor] = useState(-1);
  const searchRef = useRef(null);
  const eyeRefs = useRef([]);

  const rows = useMemo(() => filterLayers(state.layers, query), [state.layers, query]);
  const searching = query.trim().length > 0;
  const firstRead = state.phase === 'idle' || state.phase === 'loading';
  const controlsBusy = state.busy;

  // First read on open.
  useEffect(() => {
    void store.load();
  }, [store]);

  // Document identity is the only honest "a different drawing is open" signal, so
  // that is what triggers a re-read. It is deliberately not tied to the 250 ms
  // status poll: a poll that changes nothing must not re-read the layer table.
  const documentLabel = shell.documentLabel ?? null;
  const seenDocument = useRef(documentLabel);
  useEffect(() => {
    if (seenDocument.current === documentLabel) return;
    seenDocument.current = documentLabel;
    void store.load();
  }, [documentLabel, store]);

  // Opening the palette takes the keyboard (the `LAYER` selector, the ribbon
  // button and the palette toggle all land here). A repeat `LAYER` while it is
  // already open re-focuses and re-reads, which is also the manual way out of the
  // `waiting` state.
  const focusToken = shell.layersFocus ?? 0;
  const openedOnce = useRef(false);
  useEffect(() => {
    searchRef.current?.focus?.({ preventScroll: true });
    if (!openedOnce.current) {
      openedOnce.current = true;
      return;
    }
    void store.load();
  }, [focusToken, store]);

  // A filter or a refresh can shorten the list under the cursor.
  useEffect(() => {
    setCursor(current => (current >= rows.length ? (rows.length > 0 ? rows.length - 1 : -1) : current));
  }, [rows.length]);

  const clearSearch = () => {
    setQuery('');
    searchRef.current?.focus?.({ preventScroll: true });
  };

  const focusRow = index => {
    if (index < 0) return;
    setCursor(index);
    eyeRefs.current[index]?.focus?.();
  };

  // Row navigation only. `Space`/`Enter` are the native activation of whichever
  // control in the row has focus, so they are not handled (and never swallowed).
  const onListKeyDown = event => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      focusRow(nextFocusIndex(cursor, event.key === 'ArrowDown' ? 1 : -1, rows.length));
      return;
    }
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      focusRow(nextFocusIndex(-1, event.key === 'Home' ? 1 : -1, rows.length));
    }
  };

  const rowClass = layer => [
    'layers__row',
    layer.visible ? '' : 'is-hidden',
    layer.current ? 'is-current' : '',
  ].filter(Boolean).join(' ');

  // A mutation is single flight in the store, so a second press while one is in
  // flight is refused here as well — locally, without an IPC call and without a
  // queued request. The controls stay focusable (see the row markup) so the
  // roving cursor survives the refresh.
  const toggleVisibility = layer => {
    if (controlsBusy) return;
    void store.setVisible(layer.name, !layer.visible);
  };

  const makeCurrent = layer => {
    if (controlsBusy) return;
    void store.setActive(layer.name);
  };

  return (
    <section className={['palette', 'layers', controlsBusy ? 'is-busy' : ''].filter(Boolean).join(' ')} aria-label={t('layers.panel')}>
      <header className="palette__head">
        <h2><Icon name="layers" size={15} />{t('layers.panel')}</h2>
        <IconButton
          icon="close"
          size="xs"
          label={t('layers.close')}
          onClick={() => shell.setLayersOpen(false)}
        />
      </header>

      <div className="layers__tools">
        <div className="layers__search">
          <Icon name="search" size={13} />
          <input
            ref={searchRef}
            type="text"
            className="layers__search-input"
            value={query}
            placeholder={t('layers.search_placeholder')}
            aria-label={t('layers.search_placeholder')}
            autoComplete="off"
            spellCheck={false}
            onChange={event => setQuery(event.target.value)}
            // Handing the keyboard to the list from the field: one ArrowDown
            // lands on the first row, so search → flip a layer needs no Tab.
            onKeyDown={event => {
              if (event.key !== 'ArrowDown' || rows.length === 0) return;
              event.preventDefault();
              focusRow(nextFocusIndex(-1, 1, rows.length));
            }}
          />
          {query ? (
            <IconButton icon="close" size="xs" label={t('layers.clear_search')} onClick={clearSearch} />
          ) : null}
        </div>
        {/* Restore exists only to clear temporary overrides, so it appears exactly
            while there is one to clear — together with the host's own count. */}
        {state.overridden > 0 ? (
          <div className="layers__override">
            <Button
              variant="ghost"
              size="xs"
              icon="undo"
              disabled={controlsBusy}
              title={t('layers.restore')}
              onClick={() => { void store.restore(); }}
            >
              {t('layers.restore')}
            </Button>
            <span className="layers__override-count">
              {t('layers.override_count', { count: state.overridden })}
            </span>
          </div>
        ) : null}
      </div>

      <div className="palette__body layers__body" aria-busy={firstRead ? true : undefined}>
        {state.error ? (
          // A mutation failure is the catalog's own sentence plus the real reason;
          // a read failure has its own sentence, also carrying the host's words.
          <p className="layers__error" role="alert" title={state.error.reason}>
            {state.error.kind === 'mutation'
              ? t('layers.operation_failed', { reason: state.error.reason })
              : t('layers.read_failed', { reason: state.error.reason })}
          </p>
        ) : null}

        {firstRead ? (
          <div className="layers__skeleton" aria-hidden="true">
            {SKELETON_ROWS.map(index => <span key={index} />)}
          </div>
        ) : null}

        {!firstRead && state.phase === 'waiting' ? (
          <p className="palette__empty">{t('host.poll_not_ready')}</p>
        ) : null}

        {!firstRead && state.phase === 'empty' ? (
          <p className="palette__empty">{t('layers.empty')}</p>
        ) : null}

        {!firstRead && state.phase === 'ready' && rows.length === 0 ? (
          <p className="palette__empty">{t('layers.no_matches')}</p>
        ) : null}

        {rows.length > 0 ? (
          <ul className="layers__list" aria-busy={controlsBusy || undefined} onKeyDown={onListKeyDown}>
            {rows.map((layer, index) => {
              const nameId = `layers-name-${index}`;
              const tone = layerTone(layer);
              // The row exists below the drawing's own flag: a hidden layer the
              // user forced on. Saying so is the difference between "I turned this
              // on" and "this layer is on in the drawing".
              const forcedOn = layer.visible && !layer.databaseVisible;
              return (
                <li
                  key={layer.name}
                  className={rowClass(layer)}
                  aria-current={layer.current ? 'true' : undefined}
                  onFocus={() => setCursor(index)}
                >
                  {/* Temporarily unavailable, never `disabled`: a disabled button
                      drops the keyboard focus it just had, which would break the
                      roving cursor right after every toggle. */}
                  <button
                    type="button"
                    ref={node => { eyeRefs.current[index] = node; }}
                    className={['layers__eye', layer.visible ? '' : 'is-off'].filter(Boolean).join(' ')}
                    aria-label={layer.visible ? t('layers.visibility_off') : t('layers.visibility_on')}
                    aria-describedby={nameId}
                    aria-disabled={controlsBusy || undefined}
                    title={layer.visible ? t('layers.visibility_off') : t('layers.visibility_on')}
                    onClick={() => toggleVisibility(layer)}
                  >
                    <Icon name={layer.visible ? 'eye' : 'eyeOff'} size={14} />
                  </button>
                  <span className="layers__swatch" style={{ backgroundColor: tone.color }} aria-hidden="true" />
                  <span className="layers__name" id={nameId} title={layer.name}>{layer.name}</span>
                  <span className="layers__marks">
                    {layer.current ? (
                      <span className="layers__chip is-current">{t('layers.current')}</span>
                    ) : (
                      <IconButton
                        icon="check"
                        size="xs"
                        label={t('layers.set_current')}
                        aria-describedby={nameId}
                        aria-disabled={controlsBusy || undefined}
                        onClick={() => makeCurrent(layer)}
                      />
                    )}
                    {layer.overridden ? (
                      <span className="layers__chip is-temp">{t('layers.overridden_marker')}</span>
                    ) : null}
                  </span>
                  {forcedOn ? (
                    <span className="layers__note">{t('layers.database_hidden')}</span>
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : null}
      </div>
    </section>
  );
}
