// Recovery snapshots on the start screen.
//
// The same two typed IPC seams as recent drawings, validated here (a malformed
// answer is an explicit `recovery.invalid_payload` failure, never an empty list),
// plus the small hook the start panel reads. The host owns the store and the open
// request; this module only parses, caps defensively, and maps the one carrier
// refusal that means "this carrier cannot offer recovery snapshots"
// (`unsupported-runtime: cad_recovery_files`) to a real unsupported state. Every
// other rejection keeps the host's own reason.
//
// The open payload is exactly `{ key }`: the key is the host store's own hex
// identifier, and the host resolves the path. The UI never receives or sends a
// filesystem path for recovery.
//
// The `invoke` import is the sanctioned transport seam (`host/invoke.ts`), the
// same one Options uses — never `@tauri-apps/api` directly.
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '../host/invoke.ts';
import type { Invoke } from '../host/invoke.ts';

/** The list IPC. */
export const RECOVERY_LIST_COMMAND = 'cad_recovery_files';

/** The open IPC; `{ key }` is the whole payload. */
export const RECOVERY_OPEN_COMMAND = 'cad_open_recovery';

/** The one carrier limitation that means "snapshots are not offered here". */
export const RECOVERY_UNSUPPORTED = 'unsupported-runtime: cad_recovery_files';

/** Stable code for a payload that does not match the wire contract. */
export const RECOVERY_INVALID_PAYLOAD = 'recovery.invalid_payload';

/** The start panel's own defence; the host already bounds its list. */
export const MAX_RECOVERY_ENTRIES = 10;

/** One listed snapshot. `key` is the host's token; the UI never decodes it. */
export interface RecoveryFileEntry {
  /** The store's hex key — the whole open request. */
  readonly key: string;
  /** The document name at snapshot time, or `null` when there was none. */
  readonly displayName: string | null;
  /** The revision the snapshot's bytes hold. */
  readonly revision: number;
  /** The carrier's own timestamp, or `0` when it has no clock. */
  readonly savedAtMs: number;
}

/** A well-formed answer to `cad_recovery_files`. */
export type RecoveryFilesRead =
  | { readonly kind: 'supported'; readonly entries: readonly RecoveryFileEntry[] }
  | { readonly kind: 'unsupported'; readonly reason: string | null };

/** What the start panel renders for the list after the read settles. */
export type RecoveryFilesState = RecoveryFilesRead | { readonly kind: 'failed'; readonly reason: string };

/** A failed open, with the real row name and the host's reason. */
export interface RecoveryOpenFailure {
  readonly name: string;
  readonly reason: string;
}

/** The hook's whole surface. */
export interface RecoveryFilesController {
  readonly state: RecoveryFilesState | { readonly kind: 'loading' };
  readonly openError: RecoveryOpenFailure | null;
  /** Open one row; clears any earlier inline error first. */
  readonly open: (entry: RecoveryFileEntry) => void;
}

/** Throw the one explicit malformed-payload error. */
function invalidPayload(): never {
  throw Error(RECOVERY_INVALID_PAYLOAD);
}

/** A plain JSON object (not `null`, not an array). */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/** One row, strictly typed: a blank key is unusable, so it is invalid. */
function parseEntry(value: unknown): RecoveryFileEntry {
  if (!isRecord(value)) return invalidPayload();
  const { key, displayName, revision, savedAtMs } = value;
  if (typeof key !== 'string' || key.trim() === '') return invalidPayload();
  if (displayName !== null && typeof displayName !== 'string') return invalidPayload();
  if (typeof revision !== 'number' || !Number.isFinite(revision) || revision < 0) return invalidPayload();
  if (typeof savedAtMs !== 'number' || !Number.isFinite(savedAtMs) || savedAtMs < 0) return invalidPayload();
  return { key, displayName, revision, savedAtMs };
}

/**
 * Validate one `cad_recovery_files` answer.
 *
 * `supported`, `entries` and `reason` are all required: a missing field is a
 * malformed payload, not an empty list. Every entry is validated before the
 * defensive cap, so a bad row past the cap cannot hide behind it.
 */
export function parseRecoveryFiles(payload: unknown): RecoveryFilesRead {
  if (!isRecord(payload)) return invalidPayload();
  const { supported, entries, reason } = payload;
  if (typeof supported !== 'boolean') return invalidPayload();
  if (!Array.isArray(entries)) return invalidPayload();
  if (reason !== null && typeof reason !== 'string') return invalidPayload();
  const parsed = entries.map(parseEntry).slice(0, MAX_RECOVERY_ENTRIES);
  if (!supported) return { kind: 'unsupported', reason };
  return { kind: 'supported', entries: parsed };
}

/**
 * Read the list once.
 *
 * A rejected read is either the named carrier limitation (unsupported) or a real
 * failure whose reason is preserved. A resolved but malformed answer becomes a
 * `failed` state with `recovery.invalid_payload`; it is never shown as "no
 * snapshots", because those are different facts.
 */
export async function loadRecoveryFiles(call: Invoke = invoke): Promise<RecoveryFilesState> {
  let payload: unknown;
  try {
    payload = await call(RECOVERY_LIST_COMMAND);
  } catch (error) {
    const reason = String(error ?? '');
    if (reason.includes(RECOVERY_UNSUPPORTED)) return { kind: 'unsupported', reason };
    return { kind: 'failed', reason };
  }
  try {
    return parseRecoveryFiles(payload);
  } catch {
    return { kind: 'failed', reason: RECOVERY_INVALID_PAYLOAD };
  }
}

/**
 * The row's visible name: the recorded document name, else the key's first eight
 * characters (a snapshot of a never-named document still has to be recognisable).
 */
export function recoveryRowName(entry: Pick<RecoveryFileEntry, 'key' | 'displayName'>): string {
  const name = entry.displayName?.trim() ?? '';
  return name === '' ? entry.key.slice(0, 8) : name;
}

/** One open request: admission (resolve) or the host's real refusal. */
export type RecoveryOpenResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'refused'; readonly reason: string };

/**
 * Ask the host to open one snapshot.
 *
 * The payload is exactly `{ key }` — the host resolves the path from its own
 * store, so the UI never sends a path. A resolve is admission only; the real open
 * (and any later failure) arrives through `cad_status`.
 */
export async function openRecoveryFile(
  entry: Pick<RecoveryFileEntry, 'key'>,
  call: Invoke = invoke,
): Promise<RecoveryOpenResult> {
  try {
    await call(RECOVERY_OPEN_COMMAND, { key: entry.key });
    return { kind: 'accepted' };
  } catch (error) {
    return { kind: 'refused', reason: String(error ?? '') };
  }
}

/**
 * The start panel's recovery state.
 *
 * One read on mount (the panel unmounts as soon as a drawing opens, so there is
 * no polling here). A failed open is local inline state and is cleared at the
 * start of the next attempt; the list itself is never optimistically changed.
 */
export function useRecoveryFiles(call: Invoke = invoke): RecoveryFilesController {
  const [state, setState] = useState<RecoveryFilesState | { readonly kind: 'loading' }>({ kind: 'loading' });
  const [openError, setOpenError] = useState<RecoveryOpenFailure | null>(null);
  useEffect(() => {
    let cancelled = false;
    void loadRecoveryFiles(call).then(next => {
      if (!cancelled) setState(next);
    });
    return () => {
      cancelled = true;
    };
  }, [call]);
  const open = useCallback((entry: RecoveryFileEntry) => {
    setOpenError(null);
    void openRecoveryFile(entry, call).then(result => {
      if (result.kind === 'refused') setOpenError({ name: recoveryRowName(entry), reason: result.reason });
    });
  }, [call]);
  return { state, openError, open };
}
