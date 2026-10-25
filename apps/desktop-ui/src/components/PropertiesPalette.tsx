// Properties palette: an AutoCAD-style inspector for the current selection.
//
// Docked (it shrinks the drawing area) rather than floating, because the host
// composites a native CAD surface in the central partition. The rows come only
// from `cad_status.selection`; this file is layout plus the edit state machine.
// The view model (`properties-view.ts`) decides the sections, labels, editors
// and the mixed placeholder; the write contract (`properties.ts`) owns the
// grammar mirror and the one IPC name.
//
// Editing is Local until committed, like AutoCAD's palette: typing changes only
// this component's draft, `Enter`/blur/the select's own change queue exactly one
// write, and the host's next `selection.rows` is what confirms it. A refusal
// restores the host value and shows localized copy with the raw stable code in
// `title`. A mixed row writes to **every** selected object (the host resolves
// the whole selection at apply time), which the palette says above the sections
// instead of leaving as a surprise.
//
// The text-content editor is a separate mounted component (ADR 0020 addendum B):
// `selection.text` is a round-trippable string, not a display row, and it has
// its own `cad_set_selected_text` path. It stays mounted in every state so a
// parked draft survives a selection change.
import React, { useEffect, useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { SelectionTextEditor } from './SelectionTextEditor.tsx';
import { useI18n, useShell } from '../context.ts';
import type { ComposingKeyEvent } from './key-events.ts';
import type { Translator } from '../i18n.ts';
import { propertiesView } from '../properties-view.ts';
import type { PropertyRowView } from '../properties-view.ts';
import type { CadSelectionRow } from '../cad.ts';
import {
  PROPERTY_ERRORS,
  propertyCommit,
  propertyEditMessage,
  propertyErrorCode,
} from '../properties.ts';

/** How many status polls a queued write may wait for the host's echo. */
const PROPERTY_CONFIRM_POLLS = 8;

/** One row's local edit: the draft, the baseline it was typed against, the queued write. */
interface EditEntry {
  readonly draft: string | null;
  readonly base: string | null;
  readonly sent: string | null;
  readonly polls: number;
  readonly error: string | null;
  readonly errorAtStart: string | null;
}

const EMPTY_ENTRY: EditEntry = Object.freeze({
  draft: null,
  base: null,
  sent: null,
  polls: 0,
  error: null,
  errorAtStart: null,
});

function isPropertyCode(code: string | null): boolean {
  return typeof code === 'string'
    && (code.startsWith('property.') || code.startsWith('popup.'));
}

/** The choices of a select-type row, in the order the host delivered them. */
function selectChoices(row: PropertyRowView, current: string, t: Translator) {
  if (row.kind === 'bool') {
    const values = ['true', 'false'];
    if (current !== '' && !values.includes(current)) values.unshift(current);
    return values.map(value => ({
      value,
      label: value === 'true'
        ? t('properties.value.true')
        : value === 'false' ? t('properties.value.false') : value,
    }));
  }
  const values = row.options.slice();
  // The host's table is the source; a stale current value is still shown
  // verbatim rather than silently snapped to a neighbouring option.
  if (current !== '' && !values.includes(current)) values.unshift(current);
  return values.map(value => ({ value, label: value }));
}

/** One row's editor: a read-only value, a select, or a text/number field. */
interface RowEditorProps {
  readonly row: PropertyRowView;
  readonly entry: EditEntry;
  readonly t: Translator;
  readonly onChange: (row: PropertyRowView, value: string) => void;
  readonly onCommit: (row: PropertyRowView) => void;
  readonly onCancel: (row: PropertyRowView) => void;
}

function RowEditor({ row, entry, t, onChange, onCommit, onCancel }: RowEditorProps) {
  const id = `properties-value-${row.key}`;
  const saving = entry.sent !== null;
  const mixedTitle = row.mixed ? row.mixedHelp : null;
  const title = saving
    ? t('properties.edit.pending')
    : mixedTitle ?? undefined;
  const current = entry.draft !== null
    ? entry.draft
    : (row.mixed ? '' : row.rawValue ?? '');

  if (row.editor === 'value') {
    return (
      <span
        className={['properties__readonly', row.mixed ? 'is-mixed' : ''].filter(Boolean).join(' ')}
        title={mixedTitle ?? undefined}
      >
        {row.value}
      </span>
    );
  }

  if (row.editor === 'select') {
    const choices = selectChoices(row, current, t);
    const placeholder = row.mixed && current === '' ? row.mixedText : null;
    return (
      <select
        id={id}
        className="select properties__select"
        value={current}
        disabled={saving}
        title={title}
        onChange={event => {
          const value = event.target.value;
          // Re-selecting the mixed placeholder is not a value.
          if (row.mixed && value === '') return;
          onChange(row, value);
          onCommit(row);
        }}
      >
        {placeholder ? <option value="">{placeholder}</option> : null}
        {choices.map(choice => (
          <option key={choice.value} value={choice.value}>{choice.label}</option>
        ))}
      </select>
    );
  }

  const common = {
    id,
    className: 'properties__input',
    value: current,
    disabled: saving,
    // The mixed placeholder is the field's value until the user types a real
    // one; it is never sent, and a blur without typing stays "untouched".
    placeholder: (row.mixed && entry.draft === null ? row.mixedText : null) ?? undefined,
    title,
    autoComplete: 'off',
    spellCheck: false,
    'aria-invalid': entry.error ? true : undefined,
  };
  const keyHandler = (event: ComposingKeyEvent<HTMLInputElement>) => {
    // An IME owns its Enter (including the legacy keyCode 229): never commit a
    // half-composed value.
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape') {
      // Drop the draft instead of letting the global Escape blur the field —
      // a blur would commit exactly what the user just asked to discard.
      event.preventDefault();
      event.stopPropagation();
      onCancel(row);
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      onCommit(row);
    }
  };

  if (row.editor === 'number') {
    return (
      <input
        {...common}
        type="number"
        inputMode="numeric"
        step={1}
        min={row.range ? row.range.min : undefined}
        max={row.range ? row.range.max : undefined}
        onChange={event => onChange(row, event.target.value)}
        onBlur={() => onCommit(row)}
        onKeyDown={keyHandler}
      />
    );
  }

  return (
    <input
      {...common}
      type="text"
      onChange={event => onChange(row, event.target.value)}
      onBlur={() => onCommit(row)}
      onKeyDown={keyHandler}
    />
  );
}

export function PropertiesPalette() {
  const { t } = useI18n();
  const shell = useShell();
  const selection = shell.status?.selection ?? null;
  const view = propertiesView(selection, t);
  const [edits, setEdits] = useState<Record<string, EditEntry>>({});
  // The edit map is also read synchronously by event handlers and the status
  // reconciler, so the ref is the source the state mirrors (never a stale render
  // closure, never a second commit while one is queued).
  const editsRef = useRef<Record<string, EditEntry>>({});

  function writeEdits(next: Record<string, EditEntry>) {
    editsRef.current = next;
    setEdits(next);
  }

  function mutate(key: string, change: EditEntry) {
    writeEdits({ ...editsRef.current, [key]: change });
  }

  function clear(key: string) {
    if (!(key in editsRef.current)) return;
    const next = { ...editsRef.current };
    delete next[key];
    writeEdits(next);
  }

  // Reconcile queued writes against the host's own rows. A write is confirmed by
  // an exact echo or by a changed canonical value (e.g. `#ff0000` reads back as
  // `aci:1`); a new `property.*` error refuses the oldest pending write; a write
  // the host never confirms is reported as unconfirmed instead of as success.
  useEffect(() => {
    const wireRows: readonly CadSelectionRow[] = shell.status?.selection?.rows ?? [];
    const rows = new Map(wireRows.map(row => [row.key, row]));
    const rawError = shell.status?.error ?? null;
    const code = rawError ? propertyErrorCode(rawError) : null;
    const propertyError = isPropertyCode(code);
    let errorTaken = false;
    let changed = false;
    const next: Record<string, EditEntry> = {};
    for (const [key, entry] of Object.entries(editsRef.current)) {
      const row = rows.get(key);
      if (!row) {
        // The row (and its object) is gone: never commit a draft across a
        // selection change.
        changed = true;
        continue;
      }
      if (entry.sent === null) {
        // A local draft is dropped when the host truth moved underneath it.
        if (entry.draft !== null && row.value !== entry.base) {
          changed = true;
          continue;
        }
        next[key] = entry;
        continue;
      }
      const freshError = propertyError && code !== entry.errorAtStart && !errorTaken;
      if (row.value === entry.sent || (row.value !== entry.base && !freshError)) {
        changed = true;
        continue;
      }
      if (freshError) {
        errorTaken = true;
        changed = true;
        next[key] = { ...EMPTY_ENTRY, error: code };
        continue;
      }
      const polls = entry.polls + 1;
      if (polls > PROPERTY_CONFIRM_POLLS) {
        changed = true;
        next[key] = { ...EMPTY_ENTRY, error: PROPERTY_ERRORS.unconfirmed };
        continue;
      }
      next[key] = { ...entry, polls };
      changed = true;
    }
    if (changed) writeEdits(next);
    // `shell.status` is a fresh object every poll; the reconciler runs on each
    // poll but only writes state when something really changed.
  }, [shell.status]);

  function change(row: PropertyRowView, value: string) {
    const entry = editsRef.current[row.key];
    const base = entry && entry.sent === null
      ? entry.base
      : row.rawValue;
    mutate(row.key, {
      draft: value,
      base,
      sent: null,
      polls: 0,
      error: null,
      errorAtStart: null,
    });
  }

  async function send(row: PropertyRowView, draft: string | null) {
    const check = propertyCommit(row, draft);
    if (check.kind === 'skip') {
      if (check.reason === 'unchanged') clear(row.key);
      return;
    }
    if (check.kind === 'invalid') {
      // Keep the base value so the status reconciler does not mistake the
      // still-current host row for "the truth moved" and drop the draft and its
      // message on the very next poll.
      mutate(row.key, { ...EMPTY_ENTRY, draft, base: row.rawValue, error: check.code });
      return;
    }
    const entry = editsRef.current[row.key];
    const errorAtStart = shell.status?.error ? propertyErrorCode(shell.status.error) : null;
    mutate(row.key, {
      draft: check.value,
      base: entry?.base ?? row.rawValue,
      sent: check.value,
      polls: 0,
      error: null,
      errorAtStart,
    });
    const result = await shell.applySelectedProperty(check.key, check.value);
    if (result.kind !== 'refused') return;
    // A synchronous refusal (modal gate, probe mode, queue bound) restores the
    // previous value immediately. A value/table refusal arrives through status.
    const latest = editsRef.current[row.key];
    if (!latest || latest.sent !== check.value) return;
    mutate(row.key, { ...EMPTY_ENTRY, error: result.code });
  }

  function commit(row: PropertyRowView) {
    const entry = editsRef.current[row.key];
    if (entry && entry.sent !== null) return;
    void send(row, entry ? entry.draft : null);
  }

  function cancel(row: PropertyRowView) {
    clear(row.key);
  }

  function rowMessage(row: PropertyRowView) {
    const entry = edits[row.key];
    const code = entry?.error;
    if (!code) return null;
    return propertyEditMessage(code, t);
  }

  return (
    <section className="palette properties" aria-label={t('properties.panel')}>
      <header className="palette__head">
        <h2><Icon name="properties" size={15} />{t('properties.panel')}</h2>
        <IconButton
          icon="close"
          size="xs"
          label={t('properties.close')}
          onClick={() => shell.setPropertiesOpen(false)}
        />
      </header>
      <div className="palette__body">
        {view.kind === 'empty' ? <p className="palette__empty">{t('properties.empty')}</p> : null}

        {view.kind !== 'empty' ? (
          <>
            {view.kind === 'multi' ? (
              <div className="properties__multi">
                <p className="properties__count">{view.countLabel}</p>
                {view.mixedHelp ? <p className="properties__hint">{view.mixedHelp}</p> : null}
              </div>
            ) : null}

            <div className="properties__groups">
              {view.sections.map(section => (
                <section
                  key={section.group}
                  className="properties__group"
                  aria-labelledby={`properties-group-${section.group}`}
                >
                  <h3 id={`properties-group-${section.group}`} className="properties__group-title">
                    {section.label}
                  </h3>
                  <dl className="properties__rows">
                    {section.rows.map(row => {
                      const entry = edits[row.key] ?? EMPTY_ENTRY;
                      const message = rowMessage(row);
                      const editable = row.editor !== 'value';
                      return (
                        <div
                          key={row.key}
                          className={['properties__row', editable ? 'is-editable' : ''].filter(Boolean).join(' ')}
                        >
                          <dt title={row.key}>
                            {editable
                              ? <label htmlFor={`properties-value-${row.key}`}>{row.label}</label>
                              : row.label}
                          </dt>
                          <dd>
                            <RowEditor
                              row={row}
                              entry={entry}
                              t={t}
                              onChange={change}
                              onCommit={commit}
                              onCancel={cancel}
                            />
                            {message ? (
                              <p className="properties__row-error" role="alert" title={message.code}>
                                {message.text}
                              </p>
                            ) : null}
                          </dd>
                        </div>
                      );
                    })}
                  </dl>
                </section>
              ))}
            </div>
          </>
        ) : null}

        {/* Mounted in every state so a draft survives a selection change; it
            renders nothing unless the host reports editable text or a parked
            draft exists. */}
        <SelectionTextEditor />
      </div>
    </section>
  );
}
