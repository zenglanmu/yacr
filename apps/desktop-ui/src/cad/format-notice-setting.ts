// The user's "don't show this again" preference for the open-time format
// notice (ADR-0041 D5).
//
// One host setting owns the whole behaviour: `formatNoticeEnabled`, a
// **root-level** boolean preference the host reports in `cad_settings` and
// accepts through `cad_set_setting`, default `true`. It is deliberately global:
// there is no per-format record, so turning it off silences DWG, DGN, SKP and
// RVT alike, and turning it back on restores the notice for every file.
//
// Nothing here invents state. A snapshot that does not carry the key (an older
// host, or a carrier whose settings are read-only) reads as **enabled**: the
// notice is information nobody asked to suppress, so the honest failure mode is
// showing it, never silently hiding it.
//
// A write is reported as successful only after the host's own re-read shows
// `false` — the same read-back confirmation the Options dialog applies
// (`settleSettings`) — so the card is never closed on an unconfirmed write. On a
// `dom-canvas` carrier the preference is in-memory, so "off" lasts for the
// session; the UI never promises more than that.
import { readSettings, settleSettings, writeSetting } from '../options/options.ts';
import type { SettingsSnapshot, SettingWrite, SettleOptions } from '../options/options.ts';

/** The host's read field and write key: the same camelCase spelling. */
export const FORMAT_NOTICE_SETTING = 'formatNoticeEnabled';

/** The one write this feature makes: turn the notice off for every format. */
export const FORMAT_NOTICE_OFF: SettingWrite = Object.freeze({
  field: 'formatNoticeEnabled',
  key: 'formatNoticeEnabled',
  value: false,
});

/**
 * The user's stated preference from a settings snapshot.
 *
 * Absent, `null` or any non-boolean reads as `true`: only the host's own literal
 * `false` suppresses the notice, so a malformed snapshot can never hide it.
 */
export function formatNoticeEnabled(settings: SettingsSnapshot | null | undefined): boolean {
  const value = settings?.[FORMAT_NOTICE_SETTING];
  return typeof value === 'boolean' ? value : true;
}

/** What the host says about the preference, as the notice host needs it. */
export type FormatNoticePreference =
  /** The user wants the notice. */
  | 'enabled'
  /** The user turned it off; nothing renders. */
  | 'disabled'
  /** No answer yet (`cad_waiting`, a refusal): treated as enabled. */
  | 'unknown';

/**
 * Ask the host for the preference.
 *
 * A waiting or failed read is `unknown`, never `disabled`: an unavailable
 * snapshot is not the user's decision, and hiding the card on it would be a
 * silent suppression nobody asked for.
 */
export async function readFormatNoticePreference(
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>,
): Promise<FormatNoticePreference> {
  const read = await readSettings(invoke);
  if (read.kind !== 'ok') return 'unknown';
  return formatNoticeEnabled(read.settings) ? 'enabled' : 'disabled';
}

/**
 * Whether the card may render at all.
 *
 * Only the host's confirmed `false` silences it. `unknown` (a waiting or failed
 * read) shows it, and `null` — the read has not answered yet — does not: waiting
 * one round-trip avoids flashing a card the user turned off, and the answer
 * arrives with the same `cad_status` poll that produced the open.
 */
export function formatNoticeVisible(preference: FormatNoticePreference | null): boolean {
  return preference === 'enabled' || preference === 'unknown';
}

/** The fate of the "don't show again" write. */
export type FormatNoticeDisableResult =
  /** The host accepted the write **and** its re-read shows `false`. */
  | { readonly kind: 'ok' }
  /** Nothing was confirmed; the card stays up with this reason beside it. */
  | { readonly kind: 'failed'; readonly error: string };

/**
 * Turn the notice off for good, then confirm the host really has it.
 *
 * `writeSetting` only reports the queue acknowledgement, so the `settleSettings`
 * read-back is what decides: a stale snapshot (a durable-store refusal the host
 * rolls back) is a failure, not a success. The caller closes the card only on
 * `ok`.
 */
export async function disableFormatNotice(
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>,
  options?: SettleOptions,
): Promise<FormatNoticeDisableResult> {
  const entry = FORMAT_NOTICE_OFF;
  const written = await writeSetting(invoke, entry.key, entry.value);
  if (written.kind !== 'accepted') return { kind: 'failed', error: written.code };
  const settled = await settleSettings(invoke, [entry], options);
  if (settled.kind === 'ok') return { kind: 'ok' };
  if (settled.kind === 'failed') return { kind: 'failed', error: settled.error };
  // `stale`: the write was queued but no re-read ever showed it. Reporting
  // success here is exactly the false close this helper exists to prevent.
  return { kind: 'failed', error: 'settings.not_visible' };
}
