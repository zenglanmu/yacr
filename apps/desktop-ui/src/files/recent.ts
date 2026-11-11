// Recent drawings on the start screen.
//
// Two typed IPC seams, validated here (a malformed answer is an explicit
// `recent.invalid_payload` failure, never an empty list), plus the small hook the
// start panel reads. The host owns the list and the open request; this module
// only parses, caps at ten rows defensively, and maps the one carrier refusal
// that means "this carrier cannot track recents" (`unsupported-runtime:
// cad_recent_files`, e.g. the Web build) to a real unsupported state. Every other
// rejection keeps the host's own reason.
//
// The `invoke` import is the sanctioned transport seam (`host/invoke.ts`), the
// same one Options uses — never `@tauri-apps/api` directly.
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '../host/invoke.ts';
import type { Invoke } from '../host/invoke.ts';

/** The list IPC. */
export const RECENT_LIST_COMMAND = 'cad_recent_files';

/** The open IPC; `{ opaque, displayName }` is the whole payload. */
export const RECENT_OPEN_COMMAND = 'cad_open_recent';

/** The one carrier limitation that means "recents are not tracked here". */
export const RECENT_UNSUPPORTED = 'unsupported-runtime: cad_recent_files';

/** Stable code for a payload that does not match the wire contract. */
export const RECENT_INVALID_PAYLOAD = 'recent.invalid_payload';

/** The start panel's own defence; the host already caps its list. */
export const MAX_RECENT_ENTRIES = 10;

/** One recent drawing. `opaque` is the host's token; the UI never decodes it. */
export interface RecentFileEntry {
  readonly displayName: string;
  readonly opaque: string;
  /** A real location when the host has one, otherwise `null` (never invented). */
  readonly location: string | null;
  /** Host tick, in milliseconds. */
  readonly openedAtMs: number;
}

/** A well-formed answer to `cad_recent_files`. */
export type RecentFilesRead =
  | { readonly kind: 'supported'; readonly entries: readonly RecentFileEntry[] }
  | { readonly kind: 'unsupported'; readonly reason: string | null };

/** What the start panel renders for the list after the read settles. */
export type RecentFilesState = RecentFilesRead | { readonly kind: 'failed'; readonly reason: string };

/** A failed reopen, with the real row name and the host's reason. */
export interface RecentOpenFailure {
  readonly name: string;
  readonly reason: string;
}

/** The hook's whole surface. */
export interface RecentFilesController {
  readonly state: RecentFilesState | { readonly kind: 'loading' };
  readonly openError: RecentOpenFailure | null;
  /** Reopen one row; clears any earlier inline error first. */
  readonly open: (entry: RecentFileEntry) => void;
}

/** Throw the one explicit malformed-payload error. */
function invalidPayload(): never {
  throw Error(RECENT_INVALID_PAYLOAD);
}

/** A plain JSON object (not `null`, not an array). */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/** One row, strictly typed: blank names/tokens are unusable, so they are invalid. */
function parseEntry(value: unknown): RecentFileEntry {
  if (!isRecord(value)) return invalidPayload();
  const { displayName, opaque, location, openedAtMs } = value;
  if (typeof displayName !== 'string' || displayName.trim() === '') return invalidPayload();
  if (typeof opaque !== 'string' || opaque.trim() === '') return invalidPayload();
  if (location !== null && typeof location !== 'string') return invalidPayload();
  if (typeof openedAtMs !== 'number' || !Number.isFinite(openedAtMs)) return invalidPayload();
  return { displayName, opaque, location, openedAtMs };
}

/**
 * Validate one `cad_recent_files` answer.
 *
 * `supported`, `entries` and `reason` are all required: a missing field is a
 * malformed payload, not an empty list. Every entry is validated before the
 * defensive ten-row cap, so a bad eleventh row cannot hide behind the cap.
 */
export function parseRecentFiles(payload: unknown): RecentFilesRead {
  if (!isRecord(payload)) return invalidPayload();
  const { supported, entries, reason } = payload;
  if (typeof supported !== 'boolean') return invalidPayload();
  if (!Array.isArray(entries)) return invalidPayload();
  if (reason !== null && typeof reason !== 'string') return invalidPayload();
  const parsed = entries.map(parseEntry).slice(0, MAX_RECENT_ENTRIES);
  if (!supported) return { kind: 'unsupported', reason };
  return { kind: 'supported', entries: parsed };
}

/**
 * Read the list once.
 *
 * A rejected read is either the named carrier limitation (unsupported) or a real
 * failure whose reason is preserved. A resolved but malformed answer becomes a
 * `failed` state with `recent.invalid_payload`; it is never shown as "no recent
 * drawings", because those are different facts.
 */
export async function loadRecentFiles(call: Invoke = invoke): Promise<RecentFilesState> {
  let payload: unknown;
  try {
    payload = await call(RECENT_LIST_COMMAND);
  } catch (error) {
    const reason = String(error ?? '');
    if (reason.includes(RECENT_UNSUPPORTED)) return { kind: 'unsupported', reason };
    return { kind: 'failed', reason };
  }
  try {
    return parseRecentFiles(payload);
  } catch {
    return { kind: 'failed', reason: RECENT_INVALID_PAYLOAD };
  }
}

/** One reopen request: admission (resolve) or the host's real refusal. */
export type RecentOpenResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'refused'; readonly reason: string };

/**
 * Ask the host to reopen one row.
 *
 * The payload is exactly `{ opaque, displayName }` — the host derives the path
 * from its own opaque token, so the UI never sends a decoded path. A resolve is
 * admission only; the real open (and any later failure) arrives through
 * `cad_status`.
 */
export async function openRecentFile(
  entry: Pick<RecentFileEntry, 'opaque' | 'displayName'>,
  call: Invoke = invoke,
): Promise<RecentOpenResult> {
  try {
    await call(RECENT_OPEN_COMMAND, { opaque: entry.opaque, displayName: entry.displayName });
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'refused', reason: String(error ?? '') };
  }
}

/**
 * The start panel's recent-files state.
 *
 * One read on mount (the panel unmounts as soon as a drawing opens, so there is
 * no polling here). A reopen failure is local inline state and is cleared at the
 * start of the next attempt; the list itself is never optimistically changed —
 * a successful open closes this screen through the host's own status.
 */
export function useRecentFiles(call: Invoke = invoke): RecentFilesController {
  const [state, setState] = useState<RecentFilesState | { readonly kind: 'loading' }>({ kind: 'loading' });
  const [openError, setOpenError] = useState<RecentOpenFailure | null>(null);
  useEffect(() => {
    let cancelled = false;
    void loadRecentFiles(call).then(next => {
      if (!cancelled) setState(next);
    });
    return () => {
      cancelled = true;
    };
  }, [call]);
  const open = useCallback((entry: RecentFileEntry) => {
    setOpenError(null);
    void openRecentFile(entry, call).then(result => {
      if (result.kind === 'refused') setOpenError({ name: entry.displayName, reason: result.reason });
    });
  }, [call]);
  return { state, openError, open };
}
