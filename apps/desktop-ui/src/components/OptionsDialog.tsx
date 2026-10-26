// Options dialog: the AutoCAD-style settings dialog (ADR 0021 D1 shape).
// Rendered inside `PopupOverlay` for `kind: 'options'`; the shared overlay chrome
// owns the title, the close button and Escape.
//
// Structure: a left tab rail and one content column, then the footer's
// Apply / Apply-and-close (OK) / Cancel row. Everything the dialog edits is
// **staged**: `cad_settings` is the baseline, the controls edit a local draft,
// and only Apply/OK send the diff through `cad_set_setting` and re-read the
// snapshot. Cancel and Escape (owned by `PopupOverlay`) discard the draft and close.
//
// Failure rules, deliberately explicit:
//
//   * a bad value is marked before submit (client-side mirror of the host's
//     ranges) and blocks Apply;
//   * a host refusal keeps the dialog open, marks the field that produced it and
//     shows catalog copy (`options.error.<code>`), never the bare code — the code
//     stays in the element's `title` for diagnostics;
//   * `cad_waiting` (no session yet) is a loading state that retries on a timer
//     while the window is open (never an error, never a fabricated default);
//   * a write that was accepted but not yet visible in the re-read is reported
//     as such instead of being called applied.
//
// The language control is not a `cad_settings` key: it persists through
// `storeLocale` (`localStorage['yacr.locale']`) and switches this dialog through
// the shared i18n context. The main window picks the value up through the
// storage event and a focus-time re-read (see `App.tsx`).
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import type { Locale, Translator } from '../i18n.ts';
import type { CSSProperties } from 'react';
import { storeLocale } from '../i18n.ts';
import { POPUP_BLANK } from '../popup/popup.ts';
import {
  LOCALE_CHOICES,
  OPTIONS_FIELDS,
  OPTIONS_KEYS,
  applyPayload,
  dirtyFields,
  draftFromSettings,
  fieldsForTab,
  languageChanged,
  normalizeValue,
  optionsErrorView,
  optionsTabs,
  readSettings,
  settleSettings,
  validateDraft,
  validateField,
  writeAll,
} from '../options/options.ts';
import type { OptionField, OptionTab, OptionsDraft, SettingsSnapshot } from '../options/options.ts';
import {
  DIALOG_FOOT,
  OPTIONS_BADGE,
  OPTIONS_BODY,
  OPTIONS_CHECKBOX,
  OPTIONS_CONTENT,
  OPTIONS_CONTROL,
  OPTIONS_FIELD,
  OPTIONS_FIELD_FOCUS,
  OPTIONS_FIELD_INVALID,
  OPTIONS_FOOT_ERROR,
  OPTIONS_FOOT_NOTE,
  OPTIONS_HELP,
  OPTIONS_HELP_INVALID,
  OPTIONS_LABEL,
  OPTIONS_LABEL_INVALID,
  OPTIONS_RAIL,
  OPTIONS_ROW,
  OPTIONS_ROW_RULE,
  OPTIONS_STATE,
  OPTIONS_SWATCH,
  OPTIONS_TAB,
  OPTIONS_TAB_ACTIVE,
  OPTIONS_UNIT,
  OPTIONS_WELL,
} from '../popup/popup-layout.ts';

const PANEL_ID = 'cad-options-panel';

/** How often the dialog re-reads `cad_settings` while the host has no session. */
const LOAD_RETRY_MS = 250;

/** The three load states of the dialog (the snapshot is the fourth, via `draft`). */
type Phase =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready' }
  | { readonly kind: 'failed'; readonly error: string };

function fieldIds(field: OptionField) {
  return {
    input: `options-field-${field.id}`,
    help: `options-help-${field.id}`,
    error: `options-error-${field.id}`,
  };
}

/** The one catalog phrase a field's refusals render. */
interface FieldError {
  readonly code: string;
  readonly text: string;
}

function describedBy(field: OptionField, error: FieldError | null) {
  const ids = fieldIds(field);
  return error ? `${ids.help} ${ids.error}` : ids.help;
}

/** The field surface with the transient focus ring and/or the invalid edge. */
function fieldStyle({ focused = false, invalid = false }: { readonly focused?: boolean; readonly invalid?: boolean } = {}) {
  const style = { ...OPTIONS_FIELD };
  style.flex = '1 1 auto';
  if (focused) Object.assign(style, OPTIONS_FIELD_FOCUS);
  if (invalid) style.borderColor = OPTIONS_FIELD_INVALID.borderColor;
  return style;
}

/**
 * One text-shaped editor. Its own focus state is what draws the ring: the
 * stylesheet's `:focus-visible` rule is keyboard-only, and a click into a field
 * must still read as focused (the text editor made the same decision).
 *
 * `maxWidth` keeps a three-digit number from stretching across the whole
 * control column: the field stays as wide as its value class needs, and a long
 * read-only path still uses every pixel.
 */
/** One text-shaped editor. */
interface DraftInputProps {
  readonly id: string;
  readonly value: string | boolean;
  readonly invalid: boolean;
  readonly disabled: boolean;
  readonly placeholder?: string;
  readonly describedBy: string;
  readonly inputMode?: 'numeric' | 'text';
  readonly maxWidth?: number;
  readonly onChange: (value: string) => void;
  readonly title?: string;
}

function DraftInput({
  id, value, invalid, disabled, placeholder, describedBy: description, inputMode, maxWidth, onChange, title,
}: DraftInputProps) {
  const [focused, setFocused] = useState(false);
  const style = fieldStyle({ focused, invalid });
  if (maxWidth) style.maxWidth = maxWidth;
  if (inputMode) style.fontVariantNumeric = 'tabular-nums';
  return (
    <input
      id={id}
      type="text"
      value={typeof value === 'boolean' ? String(value) : value}
      style={style}
      disabled={disabled}
      placeholder={placeholder}
      title={title}
      inputMode={inputMode}
      autoComplete="off"
      spellCheck={false}
      aria-invalid={invalid || undefined}
      aria-describedby={description}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onChange={event => onChange(event.target.value)}
    />
  );
}

/** The choices of one select control. */
interface ChoiceControlProps {
  readonly id: string;
  readonly value: string | boolean | null | undefined;
  readonly choices: readonly string[];
  readonly labelFor: (choice: string) => string;
  readonly invalid: boolean;
  readonly disabled: boolean;
  readonly describedBy: string;
  readonly onChange: (value: string) => void;
}

/** A native select, styled with the shared `.select` surface. */
function ChoiceControl({
  id, value, choices, labelFor, invalid, disabled, describedBy: description, onChange,
}: ChoiceControlProps) {
  const [focused, setFocused] = useState(false);
  const style: CSSProperties = {
    height: 24,
    minWidth: 150,
    paddingRight: 4,
    fontFamily: 'var(--font-ui)',
  };
  if (focused) {
    style.borderColor = OPTIONS_FIELD_FOCUS.borderColor;
    style.boxShadow = OPTIONS_FIELD_FOCUS.boxShadow;
    style.outline = 'none';
  }
  if (invalid) style.borderColor = OPTIONS_FIELD_INVALID.borderColor;
  return (
    <select
      id={id}
      className="select"
      style={style}
      value={value === null || value === undefined ? undefined : String(value)}
      disabled={disabled}
      aria-invalid={invalid || undefined}
      aria-describedby={description}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onChange={event => onChange(event.target.value)}
    >
      {choices.map(choice => <option key={choice} value={choice}>{labelFor(choice)}</option>)}
    </select>
  );
}

/**
 * One settings row: label column, control column, help line under the control.
 * `children` is the editor; the row owns the copy, ids and error placement so
 * every field reads identically.
 */
/** One settings row: label column, control column, help line under the control. */
interface OptionsRowProps {
  readonly field: OptionField;
  readonly error: FieldError | null;
  readonly first: boolean;
  readonly t: Translator;
  readonly children: React.ReactNode;
}

function OptionsRow({ field, error, first, t, children }: OptionsRowProps) {
  const ids = fieldIds(field);
  const labelStyle = error ? { ...OPTIONS_LABEL, ...OPTIONS_LABEL_INVALID } : OPTIONS_LABEL;
  return (
    <div style={first ? OPTIONS_ROW : { ...OPTIONS_ROW, ...OPTIONS_ROW_RULE }}>
      {field.kind === 'readonly' ? (
        <span style={labelStyle}>
          {t(field.labelKey)}
        </span>
      ) : (
        <label
          htmlFor={ids.input}
          style={labelStyle}
        >
          {t(field.labelKey)}
        </label>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 }}>
        {children}
        <p id={ids.help} style={error ? { ...OPTIONS_HELP, ...OPTIONS_HELP_INVALID } : OPTIONS_HELP}>
          {t(field.helpKey)}
        </p>
        {error ? (
          <p
            id={ids.error}
            role="alert"
            className="palette__error"
            title={error.code}
            style={{ margin: 0, fontSize: 11 }}
          >
            {error.text}
          </p>
        ) : null}
      </div>
    </div>
  );
}

/** The "not reported by this build" marker next to a disabled control. */
function MissingBadge({ t }: { readonly t: Translator }) {
  return <span style={OPTIONS_BADGE}>{t(OPTIONS_KEYS.missing)}</span>;
}

/** What `PopupOverlay` hands the Options body. */
interface OptionsDialogProps {
  readonly onClose: () => void;
  /** A failed close, shown in the footer; empty when there is none. */
  readonly closeFailure: string;
}

export function OptionsDialog({ onClose, closeFailure }: OptionsDialogProps) {
  const { t, locale, setLocale } = useI18n();
  const tabs = useMemo(() => optionsTabs(), []);
  const [phase, setPhase] = useState<Phase>({ kind: 'loading' });
  const [settings, setSettings] = useState<SettingsSnapshot | null>(null);
  const [draft, setDraft] = useState<OptionsDraft | null>(null);
  const [baselineLocale, setBaselineLocale] = useState<Locale>(locale);
  const [activeTab, setActiveTab] = useState<string | null>(() => tabs[0]?.id ?? null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [notice, setNotice] = useState<{ readonly code?: string; readonly text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const alive = useRef(true);
  const retryTimer = useRef<number | undefined>(undefined);
  const localeRef = useRef<Locale>(locale);

  useEffect(() => { localeRef.current = locale; });

  /**
   * Read the snapshot, retrying while the host reports `cad_waiting`.
   *
   * The recursion goes through a local timeout so an unmount or a successful
   * read always clears the only pending timer. The draft is seeded from the
   * host snapshot, never from a local default table.
   */
  const load = useCallback(async function run() {
    clearTimeout(retryTimer.current);
    retryTimer.current = undefined;
    setPhase({ kind: 'loading' });
    const result = await readSettings(invoke);
    if (!alive.current) return;
    if (result.kind === 'ok') {
      setSettings(result.settings);
      setDraft(draftFromSettings(result.settings, localeRef.current));
      setBaselineLocale(localeRef.current);
      setFieldErrors({});
      setNotice(null);
      setPhase({ kind: 'ready' });
      return;
    }
    if (result.kind === 'waiting') {
      retryTimer.current = setTimeout(() => { void run(); }, LOAD_RETRY_MS);
      return;
    }
    setPhase({ kind: 'failed', error: result.error });
  }, []);

  useEffect(() => {
    alive.current = true;
    void load();
    return () => {
      alive.current = false;
      clearTimeout(retryTimer.current);
    };
  }, [load]);

  const update = useCallback((id: string, value: string | boolean) => {
    setDraft(prev => (prev ? { ...prev, [id]: value } : prev));
    setNotice(null);
    setFieldErrors(prev => {
      const next = { ...prev };
      delete next[id];
      const code = validateField(id, value);
      if (code) next[id] = code;
      return next;
    });
  }, []);

  const dirty = useMemo(
    () => (draft && phase.kind === 'ready' ? dirtyFields(draft, settings, baselineLocale) : []),
    [draft, settings, baselineLocale, phase.kind],
  );

  /**
   * `after: 'apply'` stays open, `after: 'close'` closes on success (OK).
   *
   * The writes are sequential and stop at the first refusal, so the field that
   * failed is exact. The language is committed only after every settings write
   * was accepted, and the re-read is awaited before either outcome: no branch
   * claims success the host has not confirmed.
   */
  const commit = useCallback(async (after: 'apply' | 'close') => {
    if (!draft || !settings || busy) return;
    const errors = validateDraft(draft);
    const firstInvalid = OPTIONS_FIELDS.find(field => errors[field.id]);
    if (firstInvalid) {
      setFieldErrors(errors);
      setNotice(optionsErrorView(errors[firstInvalid.id], t));
      return;
    }
    const entries = applyPayload(draft, settings);
    const languageValue = languageChanged(draft, baselineLocale)
      ? normalizeValue('language', draft.language)
      : null;
    const language = typeof languageValue === 'string' ? languageValue : null;
    if (entries.length === 0 && !language) {
      if (after === 'close') onClose();
      return;
    }
    setBusy(true);
    setNotice(null);
    setFieldErrors({});
    const written = await writeAll(invoke, entries);
    if (!alive.current) return;
    if (written.kind === 'failed') {
      setBusy(false);
      setFieldErrors({ [written.field]: written.code });
      setNotice(optionsErrorView(written.code, t));
      return;
    }
    if (language) {
      // A blocked storage is a silent no-op by contract; the running document
      // still switches, and the next start falls back to the host locale.
      storeLocale(language);
      setLocale(language as Locale);
      setBaselineLocale(language as Locale);
    }
    const settled = await settleSettings(invoke, entries);
    if (!alive.current) return;
    setBusy(false);
    if (settled.kind !== 'ok') {
      // The writes were accepted but the snapshot does not show them (or could
      // not be read). Stay open and say so: closing here would claim a result
      // nobody verified.
      setNotice({
        code: ('error' in settled ? settled.error : null) ?? 'settings.apply_not_visible',
        text: t(settled.kind === 'failed' ? OPTIONS_KEYS.errorLoad : OPTIONS_KEYS.errorNotVisible),
      });
      return;
    }
    setSettings(settled.settings);
    setDraft(draftFromSettings(settled.settings, language ?? baselineLocale));
    setFieldErrors({});
    setNotice(null);
    if (after === 'close') onClose();
  }, [baselineLocale, busy, draft, onClose, setLocale, settings, t]);

  // Roving-tab keyboard support for the rail: Up/Down (and Left/Right) move the
  // selection, Home/End jump to the ends. Focus follows, so the rail is usable
  // without a pointer.
  const moveTab = useCallback((target: OptionTab | undefined, focus: boolean) => {
    if (!target) return;
    setActiveTab(target.id);
    if (focus) document.getElementById(`options-tab-${target.id}`)?.focus();
  }, []);

  const onRailKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    const index = tabs.findIndex(tab => tab.id === activeTab);
    if (index === -1) return;
    const last = tabs.length - 1;
    if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
      event.preventDefault();
      moveTab(tabs[index === last ? 0 : index + 1], true);
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
      event.preventDefault();
      moveTab(tabs[index === 0 ? last : index - 1], true);
    } else if (event.key === 'Home') {
      event.preventDefault();
      moveTab(tabs[0], true);
    } else if (event.key === 'End') {
      event.preventDefault();
      moveTab(tabs[last], true);
    }
  }, [activeTab, moveTab, tabs]);

  const tab = tabs.some(entry => entry.id === activeTab) ? activeTab : tabs[0]?.id ?? null;
  const fields = tab ? fieldsForTab(tab) : [];
  const ready = phase.kind === 'ready' && draft !== null;
  const dirtyCount = dirty.length;
  const hasLocalErrors = Object.keys(fieldErrors).length > 0;
  const failure = notice ?? (closeFailure ? { code: 'popup.close_failed', text: closeFailure } : null);

  const control = (field: OptionField) => {
    const value = draft?.[field.id];
    const missing = value === null || value === undefined;
    const error = fieldErrors[field.id] ? optionsErrorView(fieldErrors[field.id], t) : null;
    const ids = fieldIds(field);
    const description = describedBy(field, error);
    switch (field.kind) {
      case 'bool':
        return (
          <span style={OPTIONS_CONTROL}>
            <input
              id={ids.input}
              type="checkbox"
              style={OPTIONS_CHECKBOX}
              checked={typeof value === 'boolean' ? value : false}
              disabled={missing}
              aria-invalid={error ? true : undefined}
              aria-describedby={description}
              onChange={event => update(field.id, event.target.checked)}
            />
            {missing ? <MissingBadge t={t} /> : null}
          </span>
        );
      case 'number':
        return (
          <span style={OPTIONS_CONTROL}>
            <DraftInput
              id={ids.input}
              value={missing ? '' : value}
              invalid={Boolean(error)}
              disabled={missing}
              placeholder={missing ? t(OPTIONS_KEYS.missing) : undefined}
              describedBy={description}
              inputMode="numeric"
              maxWidth={120}
              onChange={next => update(field.id, next)}
            />
            <span style={OPTIONS_UNIT}>{missing ? null : t(field.unitKey)}</span>
          </span>
        );
      case 'color': {
        const valid = typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value.trim());
        return (
          <span style={OPTIONS_CONTROL}>
            <DraftInput
              id={ids.input}
              value={missing ? '' : value}
              invalid={Boolean(error)}
              disabled={missing}
              placeholder={missing ? t(OPTIONS_KEYS.missing) : '#rrggbb'}
              describedBy={description}
              maxWidth={170}
              onChange={next => update(field.id, next)}
            />
            <span
              aria-hidden="true"
              style={{
                ...OPTIONS_SWATCH,
                background: valid ? value.trim() : 'var(--field)',
                borderColor: error ? OPTIONS_FIELD_INVALID.borderColor : 'var(--line-soft)',
              }}
            />
          </span>
        );
      }
      case 'select': {
        // A choice with catalog copy (the WGPU backend) reads exactly like the
        // language selector; a bare enum keeps its upper-case wire spelling.
        const choiceLabel: (choice: string) => string = field.choiceKeyPrefix
          ? choice => t(`${field.choiceKeyPrefix}.${choice}`)
          : choice => choice.toUpperCase();
        return (
          <span style={OPTIONS_CONTROL}>
            <ChoiceControl
              id={ids.input}
              value={value}
              choices={field.choices}
              labelFor={choiceLabel}
              invalid={Boolean(error)}
              disabled={missing}
              describedBy={description}
              onChange={next => update(field.id, next)}
            />
            {missing ? <MissingBadge t={t} /> : null}
          </span>
        );
      }
      case 'locale':
        return (
          <span style={OPTIONS_CONTROL}>
            <ChoiceControl
              id={ids.input}
              value={value}
              choices={LOCALE_CHOICES}
              labelFor={choice => t(`options.language.${choice}`)}
              invalid={Boolean(error)}
              disabled={missing}
              describedBy={description}
              onChange={next => update(field.id, next)}
            />
          </span>
        );
      case 'readonly':
      default:
        return (
          <span style={OPTIONS_CONTROL}>
            <span
              style={{ ...OPTIONS_WELL, flex: '1 1 auto', minWidth: 0 }}
              title={missing ? t(OPTIONS_KEYS.missing) : String(value)}
            >
              {missing ? POPUP_BLANK : value}
            </span>
            <span style={OPTIONS_BADGE}>{t(OPTIONS_KEYS.readOnly)}</span>
          </span>
        );
    }
  };

  const panelHeading = tab ? tabs.find(entry => entry.id === tab)?.labelKey : null;
  const label = t(OPTIONS_KEYS.title);

  return (
    <>
      <div style={OPTIONS_BODY}>
        <div
          role="tablist"
          aria-orientation="vertical"
          aria-label={label}
          style={OPTIONS_RAIL}
          onKeyDown={onRailKeyDown}
        >
          {tabs.map(entry => {
            const selected = entry.id === tab;
            return (
              <button
                key={entry.id}
                id={`options-tab-${entry.id}`}
                type="button"
                role="tab"
                aria-selected={selected}
                aria-controls={PANEL_ID}
                tabIndex={selected ? 0 : -1}
                className="ui-button"
                style={selected ? { ...OPTIONS_TAB, ...OPTIONS_TAB_ACTIVE } : OPTIONS_TAB}
                onClick={() => setActiveTab(entry.id)}
              >
                <Icon name={entry.icon as IconName} size={14} />
                <span className="ui-button__label">{t(entry.labelKey)}</span>
              </button>
            );
          })}
        </div>
        <div
          id={PANEL_ID}
          role="tabpanel"
          aria-labelledby={tab ? `options-tab-${tab}` : undefined}
          aria-label={tab ? undefined : label}
          tabIndex={-1}
          style={OPTIONS_CONTENT}
        >
          {phase.kind === 'loading' ? (
            <div style={OPTIONS_STATE} role="status" aria-live="polite">
              <Icon name="spinner" size={20} className="spin" />
              <p style={{ margin: 0, fontSize: 12, color: 'var(--text-dim)' }}>
                {t(OPTIONS_KEYS.loading)}
              </p>
            </div>
          ) : phase.kind === 'failed' ? (
            <div style={OPTIONS_STATE}>
              <Icon name="alert" size={22} style={{ color: 'var(--danger)' }} />
              <p style={{ margin: 0, fontSize: 12, lineHeight: 1.5, color: 'var(--text-dim)' }} title={phase.error}>
                {t(OPTIONS_KEYS.errorLoad)}
              </p>
              <Button icon="undo" onClick={() => { void load(); }}>{t(OPTIONS_KEYS.retry)}</Button>
            </div>
          ) : (
            <>
              {panelHeading ? (
                <p
                  style={{
                    margin: '0 0 2px',
                    fontSize: 10.5,
                    letterSpacing: '.5px',
                    textTransform: 'uppercase',
                    color: 'var(--text-muted)',
                  }}
                >
                  {t(panelHeading)}
                </p>
              ) : null}
              {fields.map((field, index) => {
                const error = fieldErrors[field.id] ? optionsErrorView(fieldErrors[field.id], t) : null;
                return (
                  <OptionsRow key={field.id} field={field} error={error} first={index === 0} t={t}>
                    {control(field)}
                  </OptionsRow>
                );
              })}
            </>
          )}
        </div>
      </div>
      <footer style={DIALOG_FOOT}>
        {failure ? (
          <p
            className="palette__error"
            role="alert"
            title={failure.code}
            style={OPTIONS_FOOT_ERROR}
          >
            {failure.text}
          </p>
        ) : (
          <p style={OPTIONS_FOOT_NOTE}>
            {dirtyCount > 0
              ? t(OPTIONS_KEYS.changed, { count: dirtyCount })
              : t(OPTIONS_KEYS.noChanges)}
          </p>
        )}
        {busy ? <span style={OPTIONS_UNIT}>{t(OPTIONS_KEYS.saving)}</span> : null}
        <Button onClick={onClose}>{t(OPTIONS_KEYS.cancel)}</Button>
        <Button
          icon="check"
          iconSize={14}
          disabled={!ready || busy || dirtyCount === 0 || hasLocalErrors}
          aria-busy={busy || undefined}
          onClick={() => { void commit('apply'); }}
        >
          {t(OPTIONS_KEYS.apply)}
        </Button>
        <Button
          variant="solid"
          disabled={!ready || busy}
          aria-busy={busy || undefined}
          onClick={() => { void commit('close'); }}
        >
          {t(OPTIONS_KEYS.ok)}
        </Button>
      </footer>
    </>
  );
}
