// Popup layer contract (ADR 0021 D1/D2, WP3).
//
// Popups are **in-DOM overlays** in the single UI tree. The host owns the state,
// not a window: `cad_status.popup` is the open/closed source of truth
// (`{ kind, label, modal }`), and `cad_open_popup` answers with an **additive**
// `payload` on a fresh open:
//
//   { accepted: true, kind: "about", popup_label: "popup-1", existing: false,
//     payload: { kind: "about", version: "0.1.2", build: "release" } }
//
//   { accepted: true, kind: "textEdit", popup_label: "popup-1", existing: false,
//     payload: { kind: "textEdit", text: "…", multiline: true } }
//
// `options` has no payload (the dialog reads `cad_settings` itself), and an
// `existing: true` answer carries no replacement payload either, so an already
// rendered dialog is never overwritten with guessed content.
//
// The `textEdit` payload is derived by the host from the current single
// selection — the UI never sends content, and `text` is shown verbatim. Every
// payload field is read defensively: a missing, malformed or mismatched tag
// degrades to the explicit unknown state, never to an editor with invented
// content.
//
// Catalog keys are addressed through [`POPUP_KEYS`] (data, not inline literals)
// so the dialog copy has exactly one declaration site. The keys live in the
// shared catalogs (`crates/cad-ui-core/i18n`).
import type { Translator } from '../i18n.ts';

/** The IPC that opens a dialog, sent by the main window. */
export const POPUP_OPEN_COMMAND = 'cad_open_popup';

/** The IPC that closes the overlay. Idempotent on the host side. */
export const POPUP_CLOSE_COMMAND = 'cad_close_popup';

/** The dialog kinds this bundle implements. */
export type PopupKind = 'about' | 'textEdit' | 'options';

/**
 * Dialog kinds this bundle implements. An unknown kind is rendered as an
 * explicit "unknown dialog" state with the persistent close affordance, never as
 * a blank overlay.
 *
 * `textEdit` is the modal editor for one text entity's raw DXF string
 * (ADR 0021 D4). Its payload is *derived by the host* from the current single
 * selection, so the UI can never supply content it should not control.
 *
 * `options` is the settings dialog. It carries no payload (the settings are read
 * from `cad_settings` after mount).
 */
export const POPUP_KINDS: readonly PopupKind[] = Object.freeze<PopupKind[]>([
  'about',
  'textEdit',
  'options',
]);

/**
 * Every catalog key the popup layer's shared chrome renders. The per-dialog
 * bodies own their own key maps (`POPUP_TEXT_KEYS`, `OPTIONS_KEYS`).
 */
export const POPUP_KEYS = Object.freeze({
  title: 'dialog.about.title',
  subtitle: 'dialog.about.subtitle',
  version: 'dialog.about.version',
  close: 'dialog.popup.close',
  bodyFailed: 'dialog.popup.body_failed',
  ribbonEntry: 'file.about',
});

/**
 * The project's "no data" glyph. `dialog.about.version` is a template, so a
 * missing `build` must still resolve its placeholder to *something* explicit:
 * `—` is the marker the diagnostics palette already uses for an absent value.
 */
export const POPUP_BLANK = '\u2014';

/** The `textEdit` content the host derives from the current selection. */
export interface TextEditContent {
  readonly text: string;
  readonly multiline: boolean;
}

/** The validated `about` payload of a fresh open. */
export interface AboutPopupPayload {
  readonly kind: 'about';
  readonly version: string;
  readonly build: string | null;
}

/** The validated `textEdit` payload of a fresh open. */
export interface TextEditPopupPayload extends TextEditContent {
  readonly kind: 'textEdit';
}

/** A payload this bundle understands; `options` carries none. */
export type PopupPayload = AboutPopupPayload | TextEditPopupPayload;

/** The two payload fields the About copy renders, each possibly absent. */
export interface AboutPopupFields {
  readonly version?: string | null;
  readonly build?: string | null;
}

/**
 * The dialog view for the host's open state.
 *
 * `about` carries the two payload strings the catalog template consumes (`version`
 * and `build`, each `null` when absent/blank/non-string); `textEdit` carries the
 * entity's **raw** string verbatim plus the source class's line rule; `options`
 * carries only the envelope. Any other kind is the explicit `unknown` state.
 */
export type PopupView =
  | { readonly kind: 'unknown'; readonly requestedKind: string | null }
  | {
    readonly kind: 'about';
    readonly requestedKind: string;
    readonly version: string | null;
    readonly build: string | null;
  }
  | { readonly kind: 'options'; readonly requestedKind: string }
  | {
    readonly kind: 'textEdit';
    readonly requestedKind: string;
    readonly text: string;
    readonly multiline: boolean;
  };

/** The fate of one `cad_open_popup` request. */
export type OpenPopupResult =
  | { readonly kind: 'refused'; readonly error: string }
  | { readonly kind: 'failed'; readonly error: string }
  | {
    readonly kind: 'accepted';
    readonly requestedKind: string;
    readonly popupLabel: string | null;
    readonly existing: boolean;
    readonly payload: PopupPayload | null;
  };

/** The fate of one `cad_close_popup` request. */
export type ClosePopupResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'failed'; readonly error: string };

/** The `invoke` handle this module needs (the Tauri IPC). */
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

/** Whether `value` names a dialog kind this bundle implements. */
export function isPopupKind(value: unknown): value is PopupKind {
  return typeof value === 'string' && POPUP_KINDS.some(kind => kind === value);
}

function payloadString(payload: Record<string, unknown>, field: string): string | null {
  const value = payload[field];
  return nonEmptyString(value) ? value.trim() : null;
}

/**
 * The `textEdit` content, or `null` when it is not exactly what the host
 * promises: `{ kind: 'textEdit', text: string, multiline: boolean }`.
 *
 * Rejection rules are deliberate and narrow, because a half-understood text
 * editor is worse than no editor at all:
 *
 * 1. the tag must be the requested `kind` — one kind's fields are never read as
 *    another's;
 * 2. `text` must be a **string** — not missing, not `null`, not a number. It is
 *    returned **verbatim**: an empty string is legal data (nothing to guess, no
 *    trim, no `\P`/`\n` mapping, no MTEXT markup interpretation);
 * 3. `multiline` must be a real **boolean** — a truthy string or number means
 *    the host and this bundle disagree about the source class, and guessing
 *    would offer (or forbid) line breaks for the wrong entity.
 *
 * Extra payload fields are ignored.
 */
export function textEditPayload(payload: unknown): TextEditContent | null {
  if (!isRecord(payload) || payload.kind !== 'textEdit') return null;
  const { text, multiline } = payload;
  if (typeof text !== 'string') return null;
  if (typeof multiline !== 'boolean') return null;
  return Object.freeze({ text, multiline });
}

/**
 * Validate one `cad_open_popup` payload against the kind that requested it.
 *
 * Returns a frozen payload or `null`. The tag is checked against `kind` on
 * purpose: a payload that disagrees with the requested kind is rejected instead
 * of rendering guessed content.
 */
export function popupPayload(kind: string, payload: unknown): PopupPayload | null {
  if (!isRecord(payload) || payload.kind !== kind) return null;
  if (kind === 'about') {
    const version = payloadString(payload, 'version');
    // The host always sends both, but a missing/malformed `build` must not
    // erase a real version: the view degrades that one field to `null` and
    // `aboutVersionText` substitutes the blank marker.
    if (!version) return null;
    return Object.freeze({ kind: 'about', version, build: payloadString(payload, 'build') });
  }
  if (kind === 'textEdit') {
    const content = textEditPayload(payload);
    return content ? Object.freeze({ kind: 'textEdit', ...content }) : null;
  }
  // `options` has no payload by design.
  return null;
}

/**
 * The dialog view for the host's open state.
 *
 * `kind: 'about'` carries the two payload strings the catalog template
 * consumes: `version` and `build`. Each is `null` when absent/blank/non-string;
 * nothing is guessed. [`aboutVersionText`] is the only place they are turned
 * into copy.
 *
 * `kind: 'textEdit'` carries the entity's **raw** string verbatim and the
 * source class's line rule: `text` (never trimmed or converted) and
 * `multiline`. Without a valid payload the view degrades to the explicit
 * unknown state rather than to an editor with invented content.
 *
 * `kind: 'options'` is the settings dialog: it has no payload fields (it reads
 * `cad_settings` itself), so the view carries only the envelope.
 *
 * Any other kind yields `{ kind: 'unknown', requestedKind }`, which the overlay
 * renders as an explicit state with the persistent close affordance.
 */
export function popupView(kind: unknown, payload: unknown): PopupView {
  const requestedKind = nonEmptyString(kind) ? kind : null;
  if (!requestedKind || !isPopupKind(requestedKind)) {
    return { kind: 'unknown', requestedKind };
  }
  if (requestedKind === 'about') {
    // `popupPayload` only ever answers an `about` request with an `about`
    // payload, but the union is narrowed explicitly so no other member's fields
    // can be read as this one's.
    const content = popupPayload('about', payload);
    const about = content?.kind === 'about' ? content : null;
    return {
      kind: 'about',
      requestedKind,
      version: about?.version ?? null,
      build: about?.build ?? null,
    };
  }
  if (requestedKind === 'options') {
    return { kind: 'options', requestedKind };
  }
  const content = popupPayload('textEdit', payload);
  if (!content || content.kind !== 'textEdit') return { kind: 'unknown', requestedKind };
  return {
    kind: 'textEdit',
    requestedKind,
    text: content.text,
    multiline: content.multiline,
  };
}

/**
 * The About dialog's one version line.
 *
 * `dialog.about.version` is a **template** carrying both `{version}` and
 * `{build}` (zh-CN: `版本 {version}（{build}）`, en: `Version {version} ({build})`),
 * so it is rendered through the translator with both payload values rather than
 * treated as a label.
 *
 * - valid `version` + valid `build` → the template with both values;
 * - valid `version`, no usable `build` → the template with `—` substituted for
 *   `{build}`, so a raw placeholder can never reach the screen (the version is
 *   real data and is still shown);
 * - no usable `version` → a plain `—` line. The template is not rendered at
 *   all: with no version there is nothing honest to say, and substituting both
 *   placeholders would invent a "Version — (—)" reading.
 */
export function aboutVersionText(
  view: AboutPopupFields | null | undefined,
  t: Translator,
): string {
  const version = view?.version ?? null;
  if (!version) return POPUP_BLANK;
  return t(POPUP_KEYS.version, { version, build: view?.build ?? POPUP_BLANK });
}

/**
 * Ask the host to close the open overlay.
 *
 * The overlay never hides itself: the host owns the gate and `cad_status.popup`.
 * `{kind:'accepted'}` means the host took the request; a failure is returned
 * explicitly (never a silent success) so the caller can report it and keep the
 * persistent close affordance usable for a retry. The IPC is idempotent, so a
 * second Escape/click is safe.
 */
export async function closePopup(invoke: Invoke): Promise<ClosePopupResult> {
  try {
    const response = await invoke(POPUP_CLOSE_COMMAND);
    const accepted = Boolean(response) && typeof response === 'object'
      && (response as { accepted?: unknown }).accepted === true;
    if (!accepted) throw Error('POPUP_CLOSE_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'failed', error: String(error) };
  }
}

/**
 * Ask the host to open (or focus) a dialog from the main window.
 *
 * `{kind:'accepted'}` is admission only; `cad_status.popup` is what makes the
 * overlay appear. `payload` is the validated host-derived content of a fresh
 * open, or `null` when the host reports `existing: true` (the caller must keep
 * the content already on screen instead of overwriting it) or the kind carries
 * none (`options`).
 */
export async function openPopup(invoke: Invoke, kind: unknown): Promise<OpenPopupResult> {
  if (!nonEmptyString(kind) || !isPopupKind(kind)) {
    return { kind: 'refused', error: 'POPUP_KIND_UNKNOWN' };
  }
  try {
    const response = await invoke(POPUP_OPEN_COMMAND, { kind });
    if (!isRecord(response) || response.accepted !== true) throw Error('POPUP_OPEN_NOT_ACCEPTED');
    const existing = response.existing === true;
    return {
      kind: 'accepted',
      requestedKind: kind,
      popupLabel: typeof response.popup_label === 'string' ? response.popup_label : null,
      existing,
      payload: existing ? null : popupPayload(kind, response.payload),
    };
  } catch (error) {
    return { kind: 'failed', error: String(error) };
  }
}
