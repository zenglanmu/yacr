// Popup-window boot contract (ADR 0021 D1).
//
// The host opens a dialog by creating a *second* native window that loads this
// same bundle (`index.html`) and injects a boot literal before the page runs:
//
//   window.__YACR_POPUP__ = {
//     kind: "about", modal: true, label: "popup-1",
//     payload: { version: "0.1.2", build: "release" },
//   }
//
// The main window never sets that global, so its absence is the *only* switch
// between "normal shell" and "this document is a dialog". Every field is read
// defensively: a missing, malformed or unexpected literal must fall back to the
// normal shell — never a half-rendered popup and never a thrown error.
//
// Catalog keys are addressed through [`POPUP_KEYS`] (data, not inline literals)
// so the dialog copy has exactly one declaration site. The keys live in the
// shared catalogs (`crates/cad-ui-core/i18n`); they are added by the Rust host
// lane and are *not* written here.

/** The IPC that opens a dialog, sent by the main window. */
export const POPUP_OPEN_COMMAND = 'cad_open_popup';

/** The IPC that closes this dialog window. Idempotent on the host side. */
export const POPUP_CLOSE_COMMAND = 'cad_close_popup';

/** The boot literal the host injects before page load. */
export const POPUP_GLOBAL = '__YACR_POPUP__';

/** Dialog kinds this bundle implements. An unknown kind is not a shell boot. */
export const POPUP_KINDS = Object.freeze(['about']);

/** Every catalog key the popup layer renders. Nothing else may be invented. */
export const POPUP_KEYS = Object.freeze({
  title: 'dialog.about.title',
  subtitle: 'dialog.about.subtitle',
  version: 'dialog.about.version',
  close: 'dialog.about.close',
  ribbonEntry: 'file.about',
});

/**
 * The project's "no data" glyph. `dialog.about.version` is a template, so a
 * missing `build` must still resolve its placeholder to *something* explicit:
 * `—` is the marker the diagnostics palette already uses for an absent value.
 */
export const POPUP_BLANK = '\u2014';

function isRecord(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function nonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

/**
 * Read and validate the boot literal, without deciding whether this build
 * implements its `kind`.
 *
 * Returns a frozen `{ kind, modal, label, payload }`, or `null`. Rejection
 * rules (each one falls back to the normal shell):
 *
 * 1. the scope is absent/not an object, or the global is absent/`null`;
 * 2. reading the global throws (a hostile/descoped getter never escapes);
 * 3. the global is not a plain object (array, string, number, function);
 * 4. `kind` is missing, not a string, or blank;
 * 5. `modal` is missing or not a boolean;
 * 6. `label` is missing, not a string, or empty;
 * 7. `payload` is missing or not a plain object (content is per kind).
 *
 * Unknown extra fields are ignored, so a newer host may add some without
 * breaking this bundle. `kind` is intentionally *not* checked against
 * [`POPUP_KINDS`] here: [`popupBoot`] does that, and [`popupView`] needs the
 * requested kind to render the explicit "unknown dialog" state.
 */
export function readPopupBoot(scope) {
  let raw;
  try {
    raw = scope?.[POPUP_GLOBAL];
  } catch {
    return null;
  }
  if (!isRecord(raw)) return null;
  const { kind, modal, label, payload } = raw;
  if (!nonEmptyString(kind)) return null;
  if (typeof modal !== 'boolean') return null;
  if (!nonEmptyString(label)) return null;
  if (!isRecord(payload)) return null;
  return Object.freeze({ kind, modal, label, payload });
}

/**
 * The boot switch: `null` means "render the normal shell".
 *
 * This is the strict form used by `App`. A well-formed literal for a kind this
 * bundle does not implement is rejected here on purpose: a stray or misspelled
 * global must never replace the whole main window with a dialog. The popup
 * window's own "unknown dialog" state (see [`popupView`]) covers the opposite
 * direction — a host that opens a kind a future bundle stops shipping.
 */
export function popupBoot(scope) {
  const boot = readPopupBoot(scope);
  if (!boot || !POPUP_KINDS.includes(boot.kind)) return null;
  return boot;
}

function payloadString(payload, field) {
  if (!isRecord(payload)) return null;
  const value = payload[field];
  return nonEmptyString(value) ? value.trim() : null;
}

/**
 * The dialog view for a validated boot descriptor.
 *
 * `kind: 'about'` carries the two payload strings the catalog template
 * consumes: `version` and `build`. Each is `null` when absent/blank/non-string;
 * nothing is guessed. [`aboutVersionText`] is the only place they are turned
 * into copy.
 *
 * Any other kind yields `{ kind: 'unknown', requestedKind }`, which the popup
 * root renders as an explicit state with a Close action rather than a blank
 * window.
 */
export function popupView(boot) {
  const requestedKind = isRecord(boot) && nonEmptyString(boot.kind) ? boot.kind : null;
  if (!requestedKind || !POPUP_KINDS.includes(requestedKind)) {
    return { kind: 'unknown', requestedKind };
  }
  if (requestedKind === 'about') {
    return {
      kind: 'about',
      requestedKind,
      modal: boot.modal === true,
      version: payloadString(boot.payload, 'version'),
      build: payloadString(boot.payload, 'build'),
    };
  }
  return { kind: 'unknown', requestedKind };
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
export function aboutVersionText(view, t) {
  const version = view?.version ?? null;
  if (!version) return POPUP_BLANK;
  return t(POPUP_KEYS.version, { version, build: view?.build ?? POPUP_BLANK });
}

/**
 * Ask the host to close this dialog window.
 *
 * The popup never hides itself: the host owns the window. `{kind:'accepted'}`
 * means the host took the request; a failure is returned explicitly (never a
 * silent success) so the caller can report it and keep the Close affordance
 * usable for a retry. The IPC is idempotent, so a second Escape/click is safe.
 */
export async function closePopup(invoke) {
  try {
    const response = await invoke(POPUP_CLOSE_COMMAND);
    if (!response || response.accepted !== true) throw Error('POPUP_CLOSE_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'failed', error: String(error) };
  }
}

/**
 * Ask the host to open (or focus) a dialog from the main window.
 *
 * `{kind:'accepted'}` is admission only. The host answers `existing: true` when
 * the same dialog is already open and focuses that window instead of creating a
 * duplicate; the caller must treat that as the success it is (a no-op in the
 * main window) rather than as a refusal or a second window.
 */
export async function openPopup(invoke, kind) {
  if (!nonEmptyString(kind) || !POPUP_KINDS.includes(kind)) {
    return { kind: 'refused', error: 'POPUP_KIND_UNKNOWN' };
  }
  try {
    const response = await invoke(POPUP_OPEN_COMMAND, { kind });
    if (!response || response.accepted !== true) throw Error('POPUP_OPEN_NOT_ACCEPTED');
    return {
      kind: 'accepted',
      requestedKind: kind,
      popupLabel: typeof response.popup_label === 'string' ? response.popup_label : null,
      existing: response.existing === true,
    };
  } catch (error) {
    return { kind: 'failed', error: String(error) };
  }
}
