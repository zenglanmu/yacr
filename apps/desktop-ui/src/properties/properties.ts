// The properties palette's write channel and its client-side mirror.
//
// Pure data + validation (no JSX, no React, no DOM), so the contract is
// unit-testable without a window — the same shape `options.ts` and
// `textedit.ts` use for the other two editors.
//
// The host is the authority: `cad_set_selected_property { key, value }` carries
// **no entity id** and resolves the current selection at apply time. Both fields
// are strings; `{accepted:true}` means *queued*, not completed. A refusal is the
// stable code the host write path names (`property.*`, or `popup.modal_open`
// when the modal gate owns input), and it surfaces either synchronously on the
// invoke or through the next `cad_status.error`. This module only:
//
//   * mirrors the value grammars so an obviously bad value is marked inline
//     before it is queued (the host validates again; nothing is clamped);
//   * normalizes a refusal into one stable code + localized copy, with the raw
//     code kept for a diagnostic `title`;
//   * owns the one IPC name.
//
// A successful write round-trips: the host's `selection.rows` are the canonical
// grammar, so the written value (or its canonical form, e.g. `#ff0000` read back
// as `aci:1`) is what the next status shows.
import type { Translator } from '../i18n.ts';

/** The IPC that queues one property write for the current selection. */
export const PROPERTY_WRITE_COMMAND = 'cad_set_selected_property';

/**
 * Stable refusal codes this build can explain. `unconfirmed`, `write_failed`
 * and `mixed_help` are local states, not codes the host returns; they share the
 * same mapping so the palette has one error vocabulary.
 */
export const PROPERTY_ERRORS = Object.freeze({
  unsupportedKey: 'property.unsupported_key',
  invalidValue: 'property.invalid_value',
  unknownLayer: 'property.unknown_layer',
  unknownLinetype: 'property.unknown_linetype',
  ambiguousName: 'property.ambiguous_name',
  notAddressable: 'property.not_addressable',
  selectionEmpty: 'property.selection_empty',
  unchanged: 'property.unchanged',
  targetMissing: 'property.target_missing',
  modalOpen: 'popup.modal_open',
  unconfirmed: 'property.unconfirmed',
  writeFailed: 'property.write_failed',
} as const);

/** One stable `property.*`/`popup.*` refusal code. */
export type PropertyErrorCode = (typeof PROPERTY_ERRORS)[keyof typeof PROPERTY_ERRORS];

/** The row facts one commit needs (a `PropertyRowView` satisfies this). */
export interface PropertyCommitRow {
  readonly key: string;
  readonly editable: boolean;
  readonly mixed: boolean;
  readonly rawValue: string | null;
  readonly options: readonly string[];
}

/** What a committed draft means for one row. */
export type PropertyCommit =
  | { readonly kind: 'skip'; readonly reason: 'read_only' | 'untouched' | 'unchanged' }
  | { readonly kind: 'invalid'; readonly code: PropertyErrorCode }
  | { readonly kind: 'send'; readonly key: string; readonly value: string };

/** The fate of one `cad_set_selected_property` call. */
export type PropertyWriteResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'refused'; readonly code: string };

/** The `invoke` handle this module needs (the Tauri IPC). */
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/**
 * The typed word that opens the docked palette.
 *
 * It is a chrome-local selector, like `OPTIONS`: the command line intercepts it
 * **before** the router, and the ribbon dispatches it as a shell action, so no
 * `cad_command` is ever sent for it.
 */
export function isPropertiesCommand(text: unknown): boolean {
  return String(text ?? '').trim().toUpperCase() === 'PROPERTIES';
}

const COLOR_BY_LAYER = /^(?:ByLayer|ByBlock)$/i;
const COLOR_ACI = /^aci:(\d+)$/i;
const COLOR_HEX = /^#[0-9a-fA-F]{6}$/;
const TRANSPARENCY = /^\d+$/;
const LINEWEIGHT_KEYWORD = /^(?:ByLayer|ByBlock|Default)$/i;
const LINEWEIGHT_MM = /^\d+(?:\.\d+)?$/;

function invalid(): PropertyErrorCode {
  return PROPERTY_ERRORS.invalidValue;
}

/**
 * Check one value against the key's documented grammar.
 *
 * Returns a stable `property.*` code, or `null` when the value is acceptable.
 * The rules mirror `cad-app::property_value`: colour is case-insensitive for
 * `ByLayer`/`ByBlock`, `aci:N` is digits only in `0..=256`, `#RRGGBB` is exactly
 * six hex digits; transparency is digits only in `0..=90`; a layer/linetype
 * written as a table name must be one of the delivered options when the host
 * sent any. Unknown keys are never refused locally — the host owns that answer.
 */
export function validatePropertyValue(
  key: string,
  value: unknown,
  options: readonly string[] = [],
): PropertyErrorCode | null {
  if (typeof value !== 'string' || value.length === 0) return invalid();
  const names: readonly string[] = Array.isArray(options) ? options : [];
  switch (key) {
    case 'color': {
      if (COLOR_BY_LAYER.test(value) || COLOR_HEX.test(value)) return null;
      const aci = COLOR_ACI.exec(value);
      if (!aci) return invalid();
      const index = Number(aci[1]);
      return Number.isSafeInteger(index) && index <= 256 ? null : invalid();
    }
    case 'layer':
      // With a table the palette sends the delivered names; without one the
      // host's own table is the only authority (an empty options list is not
      // permission to invent a name).
      return names.length === 0 || names.includes(value)
        ? null
        : PROPERTY_ERRORS.unknownLayer;
    case 'linetype':
      if (COLOR_BY_LAYER.test(value)) return null;
      return names.length === 0 || names.includes(value)
        ? null
        : PROPERTY_ERRORS.unknownLinetype;
    case 'lineweight':
      if (LINEWEIGHT_KEYWORD.test(value)) return null;
      if (names.length > 0) return names.includes(value) ? null : invalid();
      // The canonical millimetre list is the host's table; the shape is the
      // part this mirror can check without guessing a displayed option.
      return LINEWEIGHT_MM.test(value) ? null : invalid();
    case 'transparency': {
      if (!TRANSPARENCY.test(value)) return invalid();
      const percent = Number(value);
      return Number.isSafeInteger(percent) && percent <= 90 ? null : invalid();
    }
    default:
      return null;
  }
}

/**
 * What a committed draft means for one row.
 *
 * - `skip` — nothing to send: the row is read-only, the user never changed it,
 *   or (for a single value) the draft already equals the host's value;
 * - `invalid` — a stable code for the inline message, nothing is sent;
 * - `send` — the exact `{ key, value }` pair to queue. The value is **not**
 *   trimmed or normalised: the row's canonical string travels verbatim.
 *
 * `null` draft means "untouched" and is different from `''`: an emptied field
 * is a real draft and is refused by the grammar instead of silently ignored.
 */
export function propertyCommit(
  row: PropertyCommitRow | null | undefined,
  draft: unknown,
): PropertyCommit {
  if (!row || row.editable !== true) return { kind: 'skip', reason: 'read_only' };
  if (draft === null || draft === undefined) return { kind: 'skip', reason: 'untouched' };
  if (typeof draft !== 'string') return { kind: 'invalid', code: invalid() };
  if (!row.mixed && draft === row.rawValue) return { kind: 'skip', reason: 'unchanged' };
  const code = validatePropertyValue(row.key, draft, row.options);
  if (code) return { kind: 'invalid', code };
  return { kind: 'send', key: row.key, value: draft };
}

/** The stable `property.*`/`popup.*` code inside an IPC failure, or its text. */
export function propertyErrorCode(error: unknown): string {
  const text = String(error ?? '');
  const match = text.match(/(?:property|popup)\.[a-z_]+/);
  if (match) return match[0];
  return text || PROPERTY_ERRORS.writeFailed;
}

/**
 * The localized message for one code, with the raw code kept separately.
 *
 * The visible sentence is catalog copy only (`properties.edit.<suffix>`); an
 * unknown or host-only code falls back to `properties.edit.failed`. The code
 * travels alongside for a `title` attribute, never into the visible sentence —
 * which is why `failed` has no `{code}` placeholder.
 */
export function propertyEditMessage(
  code: unknown,
  t: Translator,
): { readonly code: string; readonly text: string } {
  const raw = typeof code === 'string' ? code : '';
  const match = raw.match(/^(?:property|popup)\.([a-z_]+)$/);
  const key = match ? `properties.edit.${match[1]}` : null;
  const text = key ? t(key) : null;
  return {
    code: raw,
    text: text && text !== key ? text : t('properties.edit.failed'),
  };
}

/**
 * Queue one property write for the current selection.
 *
 * `{kind:'accepted'}` is queue admission, never completion. A refusal on this
 * call is the modal gate / probe-mode / queue-bound answer; a value or table
 * refusal may arrive later through `cad_status.error`. Callers wait for the
 * host's own rows to confirm the write.
 */
export async function setSelectedProperty(
  invoke: Invoke,
  key: string,
  value: string,
): Promise<PropertyWriteResult> {
  try {
    const response = await invoke(PROPERTY_WRITE_COMMAND, { key, value });
    const accepted = Boolean(response) && typeof response === 'object'
      && (response as { accepted?: unknown }).accepted === true;
    if (!accepted) {
      return { kind: 'refused', code: PROPERTY_ERRORS.invalidValue };
    }
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'refused', code: propertyErrorCode(error) };
  }
}
