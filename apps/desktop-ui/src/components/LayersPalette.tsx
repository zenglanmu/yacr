// Layers palette: the docked layer list (spec F03).
//
// Docked on the left (it shrinks the drawing area) like the properties and
// diagnostics palettes, because the host composites a native CAD surface in the
// central partition: nothing here floats across the drawing, and there is no
// backdrop, so the CAD partition keeps receiving every pointer press that is not
// on the panel itself.
//
// The rows are the host's own answer ([`layers.ts`] owns the wire contract and
// the single-flight state machine); this file is layout plus the local search
// cursor and the three inline table-write forms. It never writes the drawing: a
// visibility toggle is a session-only temporary override, `restore` drops them,
// and "set current" only says where new entities go.
//
// Four deliberate choices worth naming:
//
//  * **Not a `role="listbox"`.** Every row owns real controls (the visibility
//    toggle, "set as current", rename and delete), and an ARIA option may not
//    contain interactive descendants. So the list is a plain `<ul>`/`<li>` list of
//    native buttons: `Tab` reaches every control, `Space`/`Enter` are the
//    buttons' own activation, and `ArrowUp`/`ArrowDown`/`Home`/`End` move focus
//    between rows as a convenience (the roving cursor) — except from a field
//    inside a row, where those keys belong to the caret. Nothing is announced as
//    "selected" that is not actually a selection — the *current layer* is
//    `aria-current`, not a listbox selection.
//  * **No invented copy.** The loading state is a skeleton (no "Loading…" string
//    exists in the catalog), the not-ready state uses the host's own sentence
//    (`host.poll_not_ready`), a failed *read* uses `layers.read_failed`, and a
//    refused *mutation* is looked up by its stable code through `layerErrorKey`
//    (`layer.in_use` → `layers.error.layer.in_use`), falling back to
//    `layers.write_failed` with the host's raw words for a code this build has no
//    sentence for (the neutral wording covers visibility, create, rename and
//    delete alike). Validation this panel does itself (an empty name) reuses the
//    catalog's own `layers.error.layer.name_empty`.
//  * **Inline, never a dialog.** Create, rename and delete are one-field forms in
//    the panel's own flow: no second window, no modal role, no browser `alert`.
//    Only one form is open at a time, it is only reachable through a real
//    `<button>`, and closing it always puts focus back on the button that opened
//    it. Delete asks once by name (`layers.delete_confirm`, with
//    `layers.delete_in_use_hint` as the note) because the wire carries no
//    per-layer entity count: every non-default layer *could* still hold objects,
//    so the note is unavoidable rather than guessed.
//  * **Nothing moves when a form opens.** The dock width is fixed and the row grid
//    is unchanged: rename replaces the name cell with a field of the same line
//    height, the row's actions keep their reserved space, and the create form
//    appears under the search strip. No cell is ever re-measured.
import React, { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { createLayersStore, errorCode, filterLayers, layerTone, nextFocusIndex } from '../layers.ts';
import type { LayerRow, LayersStore } from '../layers.ts';
import { deletableLayer, layerErrorKey } from '../layer-labels.ts';
import type { ComposingKeyEvent } from './key-events.ts';

/** The three skeleton bars of a first read. */
const SKELETON_ROWS = [0, 1, 2];

/** The create field's hint, referenced by `aria-describedby`. */
const NEW_LAYER_HINT_ID = 'layers-new-hint';

/** Which inline form the palette has open (at most one at a time). */
type LayerForm =
  | { readonly kind: 'create' }
  | { readonly kind: 'rename'; readonly layer: string }
  | { readonly kind: 'delete'; readonly layer: string };

export function LayersPalette() {
  const { t } = useI18n();
  const shell = useShell();
  // One store per mounted palette. Closing and reopening therefore starts from a
  // fresh read instead of a stale list — "refresh on open" is the mount itself.
  const storeRef = useRef<LayersStore | null>(null);
  if (!storeRef.current) storeRef.current = createLayersStore(invoke);
  const store = storeRef.current;
  const state = useSyncExternalStore(store.subscribe, store.getState);

  const [query, setQuery] = useState('');
  // The roving keyboard cursor: an index into `rows`, or `-1` when no row has
  // focus yet. It is only a focus hint; the *current layer* is the host's own
  // `current` flag on a row.
  const [cursor, setCursor] = useState(-1);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const eyeRefs = useRef<Array<HTMLButtonElement | null>>([]);

  // The one open form plus its draft. `draftError` holds a **catalog key** (never
  // copy) for a refusal this panel makes itself: a blank name, which is refused
  // here so a request whose only possible answer is `layer.name_empty` is never
  // sent — the host refuses it too, but that is not the only guard.
  const [form, setForm] = useState<LayerForm | null>(null);
  const [draft, setDraft] = useState('');
  const [draftError, setDraftError] = useState<string | null>(null);
  // A re-entrant submit guard for the window before the store reports `busy`.
  const submitting = useRef(false);

  // Where focus goes when the open form closes. Set when it opens (the button that
  // opened it), re-pointed just before a write that renames its own row, and read
  // by the effect below **after** the closing commit, so the trigger is mounted.
  const restoreFocus = useRef<(() => void) | null>(null);
  const newTriggerRef = useRef<HTMLButtonElement | null>(null);
  const newFieldRef = useRef<HTMLInputElement | null>(null);
  const renameFieldRef = useRef<HTMLInputElement | null>(null);
  const renameTriggers = useRef(new Map<string, HTMLButtonElement>());
  const deleteTriggers = useRef(new Map<string, HTMLButtonElement>());

  const rows = useMemo(() => filterLayers(state.layers, query), [state.layers, query]);
  const searching = query.trim().length > 0;
  const firstRead = state.phase === 'idle' || state.phase === 'loading';
  const controlsBusy = state.busy;
  // A create can only be written into a layer table the host actually answered, so
  // the entry point appears exactly then — the same rule Restore follows ("appears
  // exactly while there is something to clear"). No session, a first read still in
  // flight or a failed read means there is nothing to add to, and a form whose only
  // possible answer is a refusal must not be offered.
  const canCreate = state.phase === 'ready' || state.phase === 'empty';

  /**
   * Focus a row's own action button, or the search field when that row is gone
   * (a delete that succeeded, or a rename whose new row the host has not shown
   * yet). Never leaves focus on `<body>`.
   */
  const focusFallback = (node: HTMLElement | null | undefined) => {
    (node ?? searchRef.current)?.focus?.();
  };

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

  // Focus follows the form, and it moves **after** the commit that mounts the
  // target: a field takes focus when a form opens, and the trigger takes it back
  // when the form closes (its node only exists once the form is gone).
  useEffect(() => {
    if (form === null) {
      const restore = restoreFocus.current;
      restoreFocus.current = null;
      restore?.();
      return;
    }
    if (form.kind === 'create') {
      newFieldRef.current?.focus?.();
      return;
    }
    if (form.kind === 'rename') {
      renameFieldRef.current?.focus?.();
      renameFieldRef.current?.select?.();
    }
  }, [form]);

  // A rename or delete form addresses one row **by name**. A re-read — its own
  // read-back after a rename, another client's write, or a new document — can drop
  // that name, and a form that writes to a name the list no longer shows would
  // rewrite the wrong thing. It closes instead, restoring focus like a cancel.
  useEffect(() => {
    if (!form || form.kind === 'create') return;
    if (rows.some(layer => layer.name === form.layer)) return;
    setDraftError(null);
    setForm(null);
  }, [rows, form]);

  const clearSearch = () => {
    setQuery('');
    searchRef.current?.focus?.({ preventScroll: true });
  };

  const focusRow = (index: number) => {
    if (index < 0) return;
    setCursor(index);
    eyeRefs.current[index]?.focus?.();
  };

  // Row navigation only. `Space`/`Enter` are the native activation of whichever
  // control in the row has focus, so they are not handled (and never swallowed).
  const onListKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
    // A field inside a row (the rename input) owns its own keys: the caret has to
    // keep Arrow/Home/End, so the roving cursor stands down while it has focus.
    if (event.target instanceof HTMLElement && event.target.tagName === 'INPUT') return;
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

  const rowClass = (layer: LayerRow, editing: boolean) => [
    'layers__row',
    layer.visible ? '' : 'is-hidden',
    layer.current ? 'is-current' : '',
    editing ? 'is-editing' : '',
  ].filter(Boolean).join(' ');

  // --- the inline forms ---------------------------------------------------

  const openCreate = () => {
    setDraft('');
    setDraftError(null);
    restoreFocus.current = () => newTriggerRef.current?.focus?.();
    setForm({ kind: 'create' });
  };

  const openRename = (layer: LayerRow) => {
    setDraft(layer.name);
    setDraftError(null);
    restoreFocus.current = () => focusFallback(renameTriggers.current.get(layer.name));
    setForm({ kind: 'rename', layer: layer.name });
  };

  const openDelete = (layer: LayerRow) => {
    setDraftError(null);
    restoreFocus.current = () => focusFallback(deleteTriggers.current.get(layer.name));
    setForm({ kind: 'delete', layer: layer.name });
  };

  // Closing keeps whatever `restoreFocus` the form was opened with; the focus
  // effect above runs it. Called by Escape, by a Cancel button, by a successful
  // write, and by the "the row is gone" guard.
  const closeForm = () => {
    setDraftError(null);
    setForm(null);
  };

  const cancelForm = () => closeForm();

  // `Escape` cancels, `Enter` submits — on a field, never on the document. Both
  // forms stop the event so the shell's global Escape neither blurs the field
  // behind our back nor closes a palette, and an IME keeps its own keys.
  const onFieldKeyDown = (event: ComposingKeyEvent<HTMLInputElement>, submit: () => void) => {
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      cancelForm();
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      submit();
    }
  };

  const submitCreate = async () => {
    if (controlsBusy || submitting.current) return;
    const name = draft.trim();
    // A blank name never reaches the host (see `draftError` above).
    if (!name) {
      setDraftError('layers.error.layer.name_empty');
      newFieldRef.current?.focus?.();
      return;
    }
    setDraftError(null);
    submitting.current = true;
    const ack = await store.createLayer(name);
    submitting.current = false;
    // `ok` means the host admitted the write and the read-back has run, so the
    // form has nothing left to hold. A `busy` second press is not a refusal.
    if (ack.kind === 'ok') { closeForm(); return; }
    if (ack.kind === 'busy') return;
    // Refused: the name stays in the field to be corrected, and the mapped reason
    // is already on the palette's error line.
    newFieldRef.current?.focus?.();
    newFieldRef.current?.select?.();
  };

  const submitRename = async (layer: LayerRow) => {
    if (controlsBusy || submitting.current) return;
    const name = draft.trim();
    if (!name) {
      setDraftError('layers.error.layer.name_empty');
      renameFieldRef.current?.focus?.();
      return;
    }
    // Unchanged name: no IPC at all. Closing is the honest answer — there is
    // nothing to write and nothing to explain.
    if (name === layer.name) { closeForm(); return; }
    setDraftError(null);
    // The row this form points at is renamed by this very write, so the focus
    // target moves to the new name (falling back to the search field if the host
    // has not shown it yet). A `busy` answer puts it back on this row.
    const onThisRow = restoreFocus.current;
    restoreFocus.current = () => focusFallback(renameTriggers.current.get(name));
    submitting.current = true;
    const ack = await store.renameLayer(layer.name, name);
    submitting.current = false;
    if (ack.kind === 'ok') { closeForm(); return; }
    if (ack.kind === 'busy') { restoreFocus.current = onThisRow; return; }
    // Refused: the row keeps its name, so the form stays open on it and the
    // mapped reason explains why.
    restoreFocus.current = () => focusFallback(renameTriggers.current.get(layer.name));
    renameFieldRef.current?.focus?.();
    renameFieldRef.current?.select?.();
  };

  const confirmDelete = async (layer: LayerRow, index: number) => {
    if (controlsBusy || submitting.current) return;
    // The row is about to disappear: focus lands on the row that takes its place
    // (the same index, or the last row when this was the last one) and on the
    // search field when the list is empty.
    const onThisRow = restoreFocus.current;
    restoreFocus.current = () => focusFallback(eyeRefs.current[index] ?? eyeRefs.current[index - 1] ?? null);
    submitting.current = true;
    const ack = await store.deleteLayer(layer.name);
    submitting.current = false;
    if (ack.kind === 'busy') { restoreFocus.current = onThisRow; return; }
    // A refusal closes the confirm too: the row is still there and the mapped
    // reason explains it, so leaving the note up would only invite the same
    // failed press again.
    if (ack.kind !== 'ok') restoreFocus.current = () => focusFallback(deleteTriggers.current.get(layer.name));
    closeForm();
  };

  // A mutation is single flight in the store, so a second press while one is in
  // flight is refused here as well — locally, without an IPC call and without a
  // queued request. The controls stay focusable (see the row markup) so the
  // roving cursor survives the refresh.
  const toggleVisibility = (layer: LayerRow) => {
    if (controlsBusy) return;
    void store.setVisible(layer.name, !layer.visible);
  };

  const makeCurrent = (layer: LayerRow) => {
    if (controlsBusy) return;
    void store.setActive(layer.name);
  };

  /**
   * The one line in the palette's error affordance. A refusal this panel made
   * itself (a blank name) comes first, because that is the attempt still on
   * screen; then a refused write, named by its stable code; then a failed read.
   */
  function errorText(): string | null {
    if (draftError) return t(draftError);
    if (!state.error) return null;
    if (state.error.kind !== 'mutation') return t('layers.read_failed', { reason: state.error.reason });
    const key = layerErrorKey(errorCode(state.error.reason));
    return key ? t(key) : t('layers.write_failed', { reason: state.error.reason });
  }

  const message = errorText();

  return (
    <section className={['palette', 'layers', controlsBusy ? 'is-busy' : ''].filter(Boolean).join(' ')} aria-label={t('layers.panel')}>
      <header className="palette__head">
        <h2><Icon name="layers" size={15} />{t('layers.panel')}</h2>
        <span className="layers__head-actions">
          {/* The create entry point. It stays a toggle: a second press closes the
              form it opened (and takes focus back). A form that is already open
              keeps its own trigger mounted, so Escape always has somewhere to put
              focus back. */}
          {canCreate || form?.kind === 'create' ? (
            <IconButton
              icon="plus"
              size="xs"
              label={t('layers.new')}
              ref={newTriggerRef}
              aria-expanded={form?.kind === 'create' || undefined}
              onClick={() => { if (form?.kind === 'create') cancelForm(); else openCreate(); }}
            />
          ) : null}
          <IconButton
            icon="close"
            size="xs"
            label={t('layers.close')}
            onClick={() => shell.setLayersOpen(false)}
          />
        </span>
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

        {/* Create: one field, its rules, then submit/cancel. Nothing here widens
            the dock — the field takes the strip's own width. */}
        {form?.kind === 'create' ? (
          <div className="layers__new">
            <label className="layers__field">
              <span className="layers__field-label">{t('layers.name_label')}</span>
              <input
                ref={newFieldRef}
                type="text"
                className="layers__field-input"
                value={draft}
                aria-describedby={NEW_LAYER_HINT_ID}
                aria-invalid={draftError ? true : undefined}
                autoComplete="off"
                spellCheck={false}
                onChange={event => { setDraft(event.target.value); setDraftError(null); }}
                onKeyDown={event => onFieldKeyDown(event, () => { void submitCreate(); })}
              />
            </label>
            <p className="layers__hint" id={NEW_LAYER_HINT_ID}>{t('layers.new_hint')}</p>
            <div className="layers__form-actions">
              <Button variant="ghost" size="xs" onClick={cancelForm}>{t('tool.cancel')}</Button>
              <Button
                variant="ghost"
                size="xs"
                icon="plus"
                iconSize={13}
                aria-disabled={controlsBusy || undefined}
                onClick={() => { void submitCreate(); }}
              >
                {t('layers.new')}
              </Button>
            </div>
          </div>
        ) : null}

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
        {message ? (
          // One affordance for every refusal: a blank name this panel refused, a
          // write the host refused (its stable code, in the catalog's words) or a
          // failed read. A host refusal keeps its raw words in the tooltip, so a
          // code with no sentence is still reported verbatim.
          <p className="layers__error" role="alert" title={draftError ? undefined : state.error?.reason ?? undefined}>
            {message}
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
              const renaming = form?.kind === 'rename' && form.layer === layer.name;
              const confirming = form?.kind === 'delete' && form.layer === layer.name;
              const editing = renaming || confirming;
              const confirmHintId = `layers-delete-hint-${index}`;
              return (
                <li
                  key={layer.name}
                  className={rowClass(layer, editing)}
                  aria-current={layer.current ? 'true' : undefined}
                  onFocus={() => setCursor(index)}
                  // Escape closes whatever this row has open. The rename field
                  // stops its own Escape first, so a field never closes twice.
                  onKeyDown={event => {
                    if (!editing || event.key !== 'Escape' || event.defaultPrevented) return;
                    event.preventDefault();
                    event.stopPropagation();
                    cancelForm();
                  }}
                  // Blur cancels — a rename is never committed by looking away.
                  // Focus moving *inside the row* (its own action buttons) is not
                  // leaving the edit, so the pencil's toggle keeps working.
                  onBlur={editing
                    ? event => {
                      if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                      cancelForm();
                    }
                    : undefined}
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
                  {/* The name cell keeps the row's `nameId` whether it shows the
                      name or the rename field, so every `aria-describedby` that
                      points at it stays resolvable while the field is open. */}
                  {renaming ? (
                    <input
                      id={nameId}
                      ref={renameFieldRef}
                      type="text"
                      className="layers__rename"
                      value={draft}
                      aria-label={t('layers.name_label')}
                      aria-invalid={draftError ? true : undefined}
                      autoComplete="off"
                      spellCheck={false}
                      onChange={event => { setDraft(event.target.value); setDraftError(null); }}
                      onKeyDown={event => onFieldKeyDown(event, () => { void submitRename(layer); })}
                    />
                  ) : (
                    <span
                      className="layers__name"
                      id={nameId}
                      title={layer.name}
                      // The pointer shortcut for the explicit pencil next to it.
                      onDoubleClick={() => openRename(layer)}
                    >
                      {layer.name}
                    </span>
                  )}
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
                    {/* Row actions. The space is reserved at all times, so the row
                        grid never moves; the buttons themselves are revealed on
                        hover or on `:focus-within`, which is also how a keyboard
                        reaches them. While the row is renaming the same two slots
                        become the commit/cancel pair, so a pointer has a way to
                        finish an edit without the keyboard (Enter commits, Escape
                        cancels — both still work). The default layer offers no
                        delete at all: the host would refuse it. */}
                    <span className="layers__actions">
                      {renaming ? (
                        <>
                          <IconButton
                            icon="check"
                            size="xs"
                            label={t('layers.rename')}
                            aria-disabled={controlsBusy || undefined}
                            onClick={() => { void submitRename(layer); }}
                          />
                          <IconButton
                            icon="close"
                            size="xs"
                            label={t('tool.cancel')}
                            onClick={cancelForm}
                          />
                        </>
                      ) : (
                        <>
                          <IconButton
                            icon="pencil"
                            size="xs"
                            label={t('layers.rename')}
                            ref={node => {
                              if (node) renameTriggers.current.set(layer.name, node);
                              else renameTriggers.current.delete(layer.name);
                            }}
                            aria-describedby={nameId}
                            aria-disabled={controlsBusy || undefined}
                            onClick={() => openRename(layer)}
                          />
                          {deletableLayer(layer.name) ? (
                            <IconButton
                              icon="trash"
                              size="xs"
                              label={t('layers.delete')}
                              ref={node => {
                                if (node) deleteTriggers.current.set(layer.name, node);
                                else deleteTriggers.current.delete(layer.name);
                              }}
                              aria-describedby={nameId}
                              aria-pressed={confirming}
                              aria-disabled={controlsBusy || undefined}
                              onClick={() => { if (confirming) cancelForm(); else openDelete(layer); }}
                            />
                          ) : null}
                        </>
                      )}
                    </span>
                  </span>
                  {forcedOn ? (
                    <span className="layers__note">{t('layers.database_hidden')}</span>
                  ) : null}
                  {/* The confirm step: one note plus Cancel/Delete. It spans the
                      row's own grid, so the row keeps its identity (chips
                      included) and nothing above it moves. */}
                  {confirming ? (
                    <div className="layers__confirm">
                      <p className="layers__confirm-text">{t('layers.delete_confirm', { name: layer.name })}</p>
                      <p className="layers__note" id={confirmHintId}>{t('layers.delete_in_use_hint')}</p>
                      <div className="layers__form-actions">
                        <Button variant="ghost" size="xs" onClick={cancelForm}>{t('tool.cancel')}</Button>
                        <Button
                          variant="ghost"
                          size="xs"
                          icon="trash"
                          iconSize={13}
                          className="layers__danger"
                          aria-describedby={confirmHintId}
                          aria-disabled={controlsBusy || undefined}
                          onClick={() => { void confirmDelete(layer, index); }}
                        >
                          {t('layers.delete')}
                        </Button>
                      </div>
                    </div>
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
