// Options dialog model (pure data + validation, no JSX and no React).
//
// The dialog is a *staged* editor: the host's `cad_settings` snapshot is the
// baseline, every control edits a local draft, and only Apply/OK send the
// difference. This module owns that whole contract so it can be unit-tested
// without a window:
//
//   * the tab/field table (the authoritative list of controls; a tab with no
//     field is dropped instead of rendered empty);
//   * `validateField` / `validateDraft`, a client-side mirror of the host's
//     closed key table (`crates/cad-host/src/settings.rs`). The server still
//     rejects; validating here only marks a bad value before it is submitted;
//   * `dirtyFields` / `applyPayload`, the diff. The payload carries the **write**
//     key, which is not always the read field: the wire field is
//     `crosshairPercent` while `cad_set_setting` expects `crosshairSize`;
//   * the two IPCs (`cad_settings` read, `cad_set_setting` write) and the
//     `cad_waiting` boot state.
//
// Nothing here invents a value. A field the host does not report stays `null`
// (rendered as "not reported"), is never dirty and is never written.
import type { Translator } from '../i18n.ts';

/** The IPC that returns the effective settings snapshot. */
export const SETTINGS_READ_COMMAND = 'cad_settings';

/** The IPC that admits one typed setting write. */
export const SETTINGS_WRITE_COMMAND = 'cad_set_setting';

/**
 * The host's explicit "no session yet" answer to `cad_settings`.
 *
 * It is a loading state, not a failure: the owner thread mirrors the snapshot
 * after the first tick, so the dialog retries instead of showing an error.
 */
export const SETTINGS_WAITING = 'cad_waiting';

/** Stable refusal codes the host returns (the dialog maps each to catalog copy). */
export const SETTINGS_ERROR = Object.freeze({
  invalidValue: 'settings.invalid_value',
  invalidPlotFormat: 'settings.invalid_plot_format',
  unknownKey: 'settings.unknown_key',
});

/** The only plot formats the host can produce (`"svg" | "pdf"`). */
export const PLOT_FORMATS: readonly string[] = Object.freeze(['svg', 'pdf']);

/**
 * The graphics backends the user may ask WGPU for (`"auto" | "vulkan" | "dx12"`).
 *
 * The values are the shared wire contract: the host stores the choice under the
 * JSON key `wgpu_backend` with exactly these spellings, and `"auto"` is the
 * default. The renderer decides per platform what a choice can actually do, so
 * the list is the same everywhere; this dialog only reports and writes it.
 */
export const WGPU_BACKENDS: readonly string[] = Object.freeze(['auto', 'vulkan', 'dx12']);

/** The UI languages this build ships (`localStorage['yacr.locale']`). */
export const LOCALE_CHOICES: readonly string[] = Object.freeze(['zh-CN', 'en']);

/**
 * The dialog's own catalog keys. The tab/field/help/unit/error keys live on the
 * descriptors below, so every catalog key has exactly one declaration site.
 */
export const OPTIONS_KEYS = Object.freeze({
  title: 'dialog.options.title',
  ribbonEntry: 'file.options',
  apply: 'dialog.options.apply',
  ok: 'dialog.options.ok',
  cancel: 'dialog.options.cancel',
  close: 'dialog.options.close',
  retry: 'dialog.options.retry',
  loading: 'dialog.options.loading',
  saving: 'dialog.options.saving',
  readOnly: 'dialog.options.read_only',
  noChanges: 'dialog.options.no_changes',
  changed: 'dialog.options.changed',
  missing: 'options.value.missing',
  errorGeneric: 'options.error.generic',
  errorLoad: 'options.error.load_failed',
  errorNotVisible: 'options.error.not_visible',
});

/** One entry of the tab rail. */
export interface OptionTab {
  readonly id: string;
  readonly labelKey: string;
  readonly icon: string;
}

/**
 * The tab rail, AutoCAD order. `fields` are resolved from [`OPTIONS_FIELDS`] so
 * a control cannot appear in two places; see [`optionsTabs`] for the filtered
 * list the dialog actually renders (empty tabs are dropped).
 */
export const OPTIONS_TABS: readonly OptionTab[] = Object.freeze<OptionTab[]>([
  { id: 'display', labelKey: 'options.tab.display', icon: 'grid' },
  { id: 'drafting', labelKey: 'options.tab.drafting', icon: 'snap' },
  { id: 'plot', labelKey: 'options.tab.plot', icon: 'printer' },
  { id: 'files', labelKey: 'options.tab.files', icon: 'open' },
  { id: 'user', labelKey: 'options.tab.user', icon: 'language' },
]);

/** The editor a control offers. */
export type OptionFieldKind = 'bool' | 'number' | 'color' | 'select' | 'readonly' | 'locale';

/** The facts every control carries. */
interface OptionFieldBase {
  readonly id: string;
  readonly tab: string;
  readonly labelKey: string;
  readonly helpKey: string;
}

/**
 * A control whose draft is a `cad_set_setting` write: the `cad_settings` field
 * it is seeded from (`wire`) and the write key `cad_set_setting` expects (`key`).
 */
interface OptionWritableField extends OptionFieldBase {
  readonly wire: string;
  readonly key: string;
}

/**
 * One control. The `kind` tag decides which editor renders it and which facts
 * are meaningful, so the numeric range only exists on a `number` field and the
 * delivered options only exist on a `select` field.
 */
export type OptionField =
  | (OptionWritableField & { readonly kind: 'bool' | 'color' })
  | (OptionWritableField & {
    readonly kind: 'number';
    readonly min: number;
    readonly max: number;
    readonly unitKey: string;
  })
  | (OptionWritableField & {
    readonly kind: 'select';
    readonly choices: readonly string[];
    readonly choiceKeyPrefix?: string;
    readonly errorCode?: string;
  })
  | (OptionFieldBase & { readonly kind: 'readonly'; readonly wire: string })
  | (OptionFieldBase & { readonly kind: 'locale' });

/**
 * The complete control table.
 *
 * `wire` is the `cad_settings` field, `key` is the `cad_set_setting` key (absent
 * for the two non-settings controls: the read-only fonts directory and the UI
 * language). `kind` selects the editor: `bool`, `number`, `color`, `select`,
 * `readonly`, `locale`. Number fields carry the host's own range and the unit
 * suffix key; a value outside it is refused, never clamped. A `select` field
 * whose options have their own copy carries `choiceKeyPrefix`, and its refusal
 * code can be overridden with `errorCode` when the host reports a more specific
 * one.
 */
export const OPTIONS_FIELDS: readonly OptionField[] = Object.freeze<OptionField[]>([
  {
    id: 'backgroundModel',
    tab: 'display',
    kind: 'color',
    wire: 'backgroundModel',
    key: 'backgroundModel',
    labelKey: 'options.field.backgroundModel',
    helpKey: 'options.help.backgroundModel',
  },
  {
    id: 'backgroundPaper',
    tab: 'display',
    kind: 'color',
    wire: 'backgroundPaper',
    key: 'backgroundPaper',
    labelKey: 'options.field.backgroundPaper',
    helpKey: 'options.help.backgroundPaper',
  },
  {
    id: 'crosshairSize',
    tab: 'display',
    kind: 'number',
    wire: 'crosshairPercent',
    key: 'crosshairSize',
    min: 1,
    max: 100,
    unitKey: 'options.unit.percent',
    labelKey: 'options.field.crosshairSize',
    helpKey: 'options.help.crosshairSize',
  },
  {
    id: 'grid',
    tab: 'display',
    kind: 'bool',
    wire: 'grid',
    key: 'grid',
    labelKey: 'options.field.grid',
    helpKey: 'options.help.grid',
  },
  {
    id: 'axes',
    tab: 'display',
    kind: 'bool',
    wire: 'axes',
    key: 'axes',
    labelKey: 'options.field.axes',
    helpKey: 'options.help.axes',
  },
  {
    id: 'selectionHighlight',
    tab: 'display',
    kind: 'bool',
    wire: 'selectionHighlight',
    key: 'selectionHighlight',
    labelKey: 'options.field.selectionHighlight',
    helpKey: 'options.help.selectionHighlight',
  },
  {
    id: 'snapHints',
    tab: 'display',
    kind: 'bool',
    wire: 'snapHints',
    key: 'snapHints',
    labelKey: 'options.field.snapHints',
    helpKey: 'options.help.snapHints',
  },
  {
    // The share of the graphics card's memory CAD may use. The renderer lane
    // reads this persisted value; neither the wire field, the write key nor the
    // config leaf is renamed here, so the three spellings cannot drift apart.
    id: 'gpuBudgetPercent',
    tab: 'display',
    kind: 'number',
    wire: 'gpuBudgetPercent',
    key: 'gpuBudgetPercent',
    min: 10,
    max: 100,
    unitKey: 'options.unit.percent',
    labelKey: 'options.field.gpuBudgetPercent',
    helpKey: 'options.help.gpuBudgetPercent',
  },
  {
    id: 'snapEnabled',
    tab: 'drafting',
    kind: 'bool',
    wire: 'snapEnabled',
    key: 'snapEnabled',
    labelKey: 'options.field.snapEnabled',
    helpKey: 'options.help.snapEnabled',
  },
  {
    id: 'hoverPreview',
    tab: 'drafting',
    kind: 'bool',
    wire: 'hoverPreview',
    key: 'hoverPreview',
    labelKey: 'options.field.hoverPreview',
    helpKey: 'options.help.hoverPreview',
  },
  {
    id: 'pickboxSize',
    tab: 'drafting',
    kind: 'number',
    wire: 'pickboxSize',
    key: 'pickboxSize',
    min: 1,
    max: 20,
    unitKey: 'options.unit.pixel',
    labelKey: 'options.field.pickboxSize',
    helpKey: 'options.help.pickboxSize',
  },
  {
    id: 'apertureSize',
    tab: 'drafting',
    kind: 'number',
    wire: 'apertureSize',
    key: 'apertureSize',
    min: 1,
    max: 50,
    unitKey: 'options.unit.pixel',
    labelKey: 'options.field.apertureSize',
    helpKey: 'options.help.apertureSize',
  },
  {
    id: 'plotFormat',
    tab: 'plot',
    kind: 'select',
    wire: 'plotFormat',
    key: 'plotFormat',
    choices: PLOT_FORMATS,
    errorCode: SETTINGS_ERROR.invalidPlotFormat,
    labelKey: 'options.field.plotFormat',
    helpKey: 'options.help.plotFormat',
  },
  {
    id: 'fontsDir',
    tab: 'files',
    kind: 'readonly',
    wire: 'fontsDir',
    labelKey: 'options.field.fontsDir',
    helpKey: 'options.help.fontsDir',
  },
  {
    id: 'language',
    tab: 'user',
    kind: 'locale',
    labelKey: 'options.field.language',
    helpKey: 'options.help.language',
  },
  {
    // The WGPU backend choice. Both the snapshot field and the write key are the
    // shared snake_case `wgpu_backend` (never camelCased into `wgpuBackend`), so
    // the host config key and this control cannot drift apart.
    id: 'wgpuBackend',
    tab: 'user',
    kind: 'select',
    wire: 'wgpu_backend',
    key: 'wgpu_backend',
    choices: WGPU_BACKENDS,
    choiceKeyPrefix: 'options.wgpuBackend',
    labelKey: 'options.field.wgpuBackend',
    helpKey: 'options.help.wgpuBackend',
  },
]);

const FIELD_BY_ID = new Map<string, OptionField>(OPTIONS_FIELDS.map(field => [field.id, field]));

const HEX_COLOUR = /^#[0-9a-fA-F]{6}$/;

/** The `cad_settings` snapshot as the dialog reads it (untrusted strings). */
export type SettingsSnapshot = Readonly<Record<string, unknown>>;

/** One staged control value: the draft's own text/boolean, or `null`. */
export type OptionsDraft = Record<string, string | boolean | null>;

/** One changed settings write, in table order. */
export interface SettingWrite {
  readonly field: string;
  readonly key: string;
  readonly value: string | boolean | number;
}

/** The `invoke` handle this module needs (the Tauri IPC). */
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/** One field descriptor by id, or `null`. */
export function fieldById(id: string): OptionField | null {
  return FIELD_BY_ID.get(id) ?? null;
}

/** One tab descriptor by id, or `null`. */
export function tabById(id: string): OptionTab | null {
  return OPTIONS_TABS.find(tab => tab.id === id) ?? null;
}

/** The fields of one tab, in declaration order. */
export function fieldsForTab(tabId: string): OptionField[] {
  return OPTIONS_FIELDS.filter(field => field.tab === tabId);
}

/**
 * The tabs the dialog renders: only tabs that own at least one field. A tab
 * with no backend control is dropped, never shown as an empty page.
 */
export function optionsTabs(): OptionTab[] {
  return OPTIONS_TABS.filter(tab => fieldsForTab(tab.id).length > 0);
}

/** The initial draft for a settings snapshot (string-valued text controls). */
export function draftFromSettings(
  settings: SettingsSnapshot | null | undefined,
  locale: string | null = null,
): OptionsDraft {
  const draft: OptionsDraft = {};
  for (const field of OPTIONS_FIELDS) {
    if (field.kind === 'locale') {
      draft[field.id] = typeof locale === 'string' && LOCALE_CHOICES.includes(locale) ? locale : null;
      continue;
    }
    const value = settings?.[field.wire];
    if (field.kind === 'bool') {
      draft[field.id] = typeof value === 'boolean' ? value : null;
    } else if (field.kind === 'number') {
      const number = parseIntegerText(value);
      draft[field.id] = number === null ? null : String(number);
    } else if (field.kind === 'color') {
      draft[field.id] = typeof value === 'string' && HEX_COLOUR.test(value)
        ? value.trim().toLowerCase()
        : null;
    } else if (field.kind === 'select') {
      draft[field.id] = typeof value === 'string' && field.choices.includes(value) ? value : null;
    } else {
      // `readonly` / `locale`: verbatim display data, `null` when absent.
      draft[field.id] = typeof value === 'string' ? value : null;
    }
  }
  return draft;
}

/** A trimmed, unsigned integer from draft text (or a number), else `null`. */
export function parseIntegerText(value: unknown): number | null {
  if (typeof value === 'number') return Number.isInteger(value) ? value : null;
  if (typeof value !== 'string') return null;
  const text = value.trim();
  if (!/^\d+$/.test(text)) return null;
  const number = Number(text);
  return Number.isSafeInteger(number) ? number : null;
}

/**
 * Validate one control value against the host's rules.
 *
 * Returns a stable error code (`settings.*`) or `null` when the value is
 * acceptable. This is a mirror, not an authority: the host validates again and
 * its refusal is what the dialog reports.
 */
export function validateField(id: string, value: unknown): string | null {
  const field = fieldById(id);
  if (!field) return SETTINGS_ERROR.unknownKey;
  switch (field.kind) {
    case 'bool':
      return typeof value === 'boolean' ? null : SETTINGS_ERROR.invalidValue;
    case 'number': {
      const number = parseIntegerText(value);
      if (number === null) return SETTINGS_ERROR.invalidValue;
      return number >= field.min && number <= field.max ? null : SETTINGS_ERROR.invalidValue;
    }
    case 'color':
      return typeof value === 'string' && HEX_COLOUR.test(value.trim())
        ? null
        : SETTINGS_ERROR.invalidValue;
    case 'select':
      if (typeof value === 'string' && field.choices.includes(value)) return null;
      // The host's own code for this control (the plot format has a dedicated
      // one); a plain enum mismatch is the generic invalid value.
      return field.errorCode ?? SETTINGS_ERROR.invalidValue;
    case 'locale':
      return typeof value === 'string' && LOCALE_CHOICES.includes(value)
        ? null
        : SETTINGS_ERROR.invalidValue;
    case 'readonly':
      return null;
    default:
      return SETTINGS_ERROR.unknownKey;
  }
}

/**
 * Every error in the draft, keyed by field id.
 *
 * A field the host does not report (`null`) is skipped: there is nothing to
 * validate and nothing to send, and blocking Apply on it would make an older
 * host impossible to use.
 */
export function validateDraft(draft: OptionsDraft | null | undefined): Record<string, string> {
  const errors: Record<string, string> = {};
  if (!draft) return errors;
  for (const field of OPTIONS_FIELDS) {
    if (field.kind === 'readonly' || field.kind === 'locale' || !field.key) continue;
    const value = draft[field.id];
    if (value === undefined || value === null) continue;
    const code = validateField(field.id, value);
    if (code) errors[field.id] = code;
  }
  return errors;
}

/**
 * A comparable form of one value: the wire's canonical shape, or `null` when
 * the value is absent/not representable. Used for both diffing and the
 * "has the host caught up" check, so the two can never disagree.
 */
export function normalizeValue(id: string, value: unknown): string | boolean | number | null {
  const field = fieldById(id);
  if (!field) return null;
  switch (field.kind) {
    case 'bool':
      return typeof value === 'boolean' ? value : null;
    case 'number':
      return parseIntegerText(value);
    case 'color':
      return typeof value === 'string' && HEX_COLOUR.test(value.trim())
        ? value.trim().toLowerCase()
        : null;
    case 'select':
      return typeof value === 'string' && field.choices.includes(value) ? value : null;
    case 'locale':
      return typeof value === 'string' && LOCALE_CHOICES.includes(value) ? value : null;
    default:
      return typeof value === 'string' ? value : null;
  }
}

/**
 * The ids whose draft differs from the host snapshot (in table order).
 *
 * The language control is compared against `locale`, the dialog's current UI
 * language, because it is not a `cad_settings` key. A value that does not
 * normalise (still being typed, or unreported) is never dirty.
 */
export function dirtyFields(
  draft: OptionsDraft | null | undefined,
  settings: SettingsSnapshot | null | undefined,
  locale: string | null = null,
): string[] {
  if (!draft) return [];
  const dirty: string[] = [];
  for (const field of OPTIONS_FIELDS) {
    if (field.kind === 'readonly') continue;
    if (field.kind === 'locale') {
      const next = normalizeValue(field.id, draft[field.id]);
      if (next !== null && locale !== null && next !== locale) dirty.push(field.id);
      continue;
    }
    const next = normalizeValue(field.id, draft[field.id]);
    const current = normalizeValue(field.id, settings?.[field.wire]);
    if (next !== null && next !== current) dirty.push(field.id);
  }
  return dirty;
}

/**
 * The `cad_set_setting` calls for a draft, one per changed settings key.
 *
 * Order is the table order (stable across sessions). Each entry carries the
 * **write** key (`crosshairSize`, not the wire field `crosshairPercent`) and a
 * typed value: boolean, integer or string. The language control and the
 * read-only fonts directory are never part of this payload.
 */
export function applyPayload(
  draft: OptionsDraft | null | undefined,
  settings: SettingsSnapshot | null | undefined,
): SettingWrite[] {
  const dirty = new Set(dirtyFields(draft, settings, null));
  const entries: SettingWrite[] = [];
  for (const field of OPTIONS_FIELDS) {
    if (field.kind === 'readonly' || field.kind === 'locale' || !field.key) continue;
    if (!dirty.has(field.id)) continue;
    const value = normalizeValue(field.id, draft?.[field.id]);
    if (value === null) continue;
    entries.push({ field: field.id, key: field.key, value });
  }
  return entries;
}

/** Whether the staged UI language differs from the running one. */
export function languageChanged(draft: OptionsDraft | null | undefined, locale: string | null): boolean {
  const next = normalizeValue('language', draft?.language);
  return next !== null && locale !== null && next !== locale;
}

/** Extract a stable `settings.*` code from an IPC failure, or the raw text. */
export function settingErrorCode(error: unknown): string {
  const text = String(error ?? '');
  const match = text.match(/settings\.[a-z_]+/);
  return match ? match[0] : text;
}

/**
 * The localized message for one error code, with the code kept separately.
 *
 * The visible text is catalog copy only (`options.error.<code>`); an unknown or
 * host-only message falls back to `options.error.generic`. The code travels
 * alongside for a `title` attribute, never into the visible sentence.
 */
export function optionsErrorView(
  code: unknown,
  t: Translator,
): { code: string; text: string } {
  const key = typeof code === 'string' ? code : '';
  const lookup = `options.error.${key}`;
  const text = t(lookup);
  if (text !== lookup) return { code: key, text };
  return { code: key, text: t(OPTIONS_KEYS.errorGeneric) };
}

/** The host's answer to one `cad_settings` read. */
export type SettingsRead =
  | { kind: 'ok'; settings: SettingsSnapshot }
  | { kind: 'waiting' }
  | { kind: 'failed'; error: string };

/**
 * Read the effective settings snapshot.
 *
 * `cad_waiting` is the one "not ready yet" answer and is reported as its own
 * state; every other throw is a real failure with its text preserved.
 */
export async function readSettings(invoke: Invoke): Promise<SettingsRead> {
  try {
    const wire = await invoke(SETTINGS_READ_COMMAND);
    if (!wire || typeof wire !== 'object' || Array.isArray(wire)) {
      return { kind: 'failed', error: 'SETTINGS_INVALID_PAYLOAD' };
    }
    return { kind: 'ok', settings: wire as SettingsSnapshot };
  } catch (error) {
    const text = String(error ?? '');
    if (text.includes(SETTINGS_WAITING)) return { kind: 'waiting' };
    return { kind: 'failed', error: text };
  }
}

/** The fate of one `cad_set_setting` call. */
export type SettingWriteResult =
  | { kind: 'accepted' }
  | { kind: 'failed'; code: string };

/**
 * Admit one typed setting write.
 *
 * `{accepted:true}` means queued, not completed; the caller re-reads the
 * snapshot. A refusal is the stable code, so it can be mapped to catalog copy
 * and attached to the field that produced it.
 */
export async function writeSetting(
  invoke: Invoke,
  key: string,
  value: string | boolean | number,
): Promise<SettingWriteResult> {
  try {
    const response = await invoke(SETTINGS_WRITE_COMMAND, { key, value });
    const accepted = Boolean(response) && typeof response === 'object'
      && (response as { accepted?: unknown }).accepted === true;
    if (!accepted) return { kind: 'failed', code: SETTINGS_ERROR.invalidValue };
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'failed', code: settingErrorCode(error) };
  }
}

/** The fate of a whole staged payload. */
export type SettingsWriteAllResult =
  | { kind: 'accepted' }
  | { kind: 'failed'; field: string; key: string; code: string };

/** Write the payload in order, stopping at the first refusal. */
export async function writeAll(
  invoke: Invoke,
  entries: readonly SettingWrite[],
): Promise<SettingsWriteAllResult> {
  for (const entry of entries) {
    const result = await writeSetting(invoke, entry.key, entry.value);
    if (result.kind !== 'accepted') {
      return { kind: 'failed', field: entry.field, key: entry.key, code: result.code };
    }
  }
  return { kind: 'accepted' };
}

/** Whether one written entry is visible in a fresh snapshot. */
export function entryVisible(
  settings: SettingsSnapshot | null | undefined,
  entry: SettingWrite,
): boolean {
  const field = fieldById(entry.field);
  if (!field) return true;
  // The language control has no `cad_settings` field; it is never in a payload,
  // so an entry naming it can only read as "not visible".
  if (!('wire' in field)) return normalizeValue(field.id, undefined) === entry.value;
  return normalizeValue(field.id, settings?.[field.wire]) === entry.value;
}

/** The sleep the retry loop uses; injectable so a test never waits. */
export type Sleep = (ms: number) => Promise<void>;

const defaultSleep: Sleep = ms => new Promise(resolve => { setTimeout(resolve, ms); });

/** How hard [`settleSettings`] retries before reporting a stale snapshot. */
export interface SettleOptions {
  readonly attempts?: number;
  readonly delayMs?: number;
  readonly sleep?: Sleep;
}

/** The host's settled answer: the read-back snapshot, or why there is none. */
export type SettingsSettleResult =
  | { kind: 'ok'; settings: SettingsSnapshot }
  | { kind: 'failed'; error: string }
  | { kind: 'stale'; settings: SettingsSnapshot | null };

/**
 * Wait for the queue to drain and return the snapshot the host really has.
 *
 * Writes are admitted onto the owner thread's queue, so an immediate re-read can
 * still be the previous tick's snapshot. Polling compares the normalised values
 * (never a raw string), so canonicalisation cannot look like a lost write. When
 * the budget is spent the result is `stale` with the last snapshot — never a
 * claimed success.
 */
export async function settleSettings(
  invoke: Invoke,
  entries: readonly SettingWrite[] | null | undefined,
  { attempts = 12, delayMs = 100, sleep = defaultSleep }: SettleOptions = {},
): Promise<SettingsSettleResult> {
  const pending = Array.isArray(entries) ? entries : [];
  const rounds = Math.max(1, attempts);
  let last: SettingsSnapshot | null = null;
  for (let round = 0; round < rounds; round += 1) {
    if (round > 0) await sleep(delayMs);
    const read = await readSettings(invoke);
    if (read.kind === 'failed') return { kind: 'failed', error: read.error };
    if (read.kind === 'waiting') continue;
    last = read.settings;
    if (pending.every(entry => entryVisible(read.settings, entry))) {
      return { kind: 'ok', settings: read.settings };
    }
  }
  return { kind: 'stale', settings: last };
}

/**
 * Whether typed command text is the local `OPTIONS` selector.
 *
 * The Rust router has no such command: the chrome opens the dialog directly and
 * never forwards the text, so nothing is admitted or refused on the host side.
 */
export function isOptionsCommand(text: unknown): boolean {
  return String(text ?? '').trim().toUpperCase() === 'OPTIONS';
}
