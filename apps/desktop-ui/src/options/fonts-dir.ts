// The fonts-directory rows' host facts and carrier gate (Options > Files).
//
// `options.ts` owns the control table and the staged write; this module owns what
// the row needs from the *snapshot* and from the *carrier*, from the host's one
// authoritative shape (`crates/cad-host/src/settings_snapshot.rs`, mirrored by
// `crates/cad-ui-core/src/session/settings.rs`):
//
//   * `fontsDir` (string) — the directory fonts are actually loaded from. The key
//     is **omitted** (never `null`) when nothing resolves and on the Web carrier,
//     so its absence means "this carrier cannot name a directory".
//   * `fontsDirOverride` (string) — the saved choice the row edits; omitted when
//     it was never set or was cleared.
//   * `fontsDirMissing` (boolean, always present) — `true` only when the saved
//     choice no longer names a usable directory. It is the host's own verdict, so
//     it is the **only** source of the row warning: comparing the two paths here
//     would cry "missing" whenever `--fonts-dir` legitimately wins over a valid
//     saved choice.
//
// A carrier that can read no local directory reports no `fontsDir` and refuses
// the pick and the write (`unsupported-runtime: fontsDir`/`cad_pick_directory`),
// so the row must read as unavailable rather than as an empty editable field.
// The gate is the reported `hostProfile.surfaceMode` (`cad/surface.ts`), never the
// build target (`__YACR_RUNTIME__`).
import type { Invoke } from '../host/invoke.ts';
import { SURFACE_MODE_NATIVE_BELOW, surfaceModeFromShell } from '../cad/surface.ts';
import type { SurfaceModeReading } from '../cad/surface.ts';

/** The IPC that opens the host's directory picker. */
export const PICK_DIRECTORY_COMMAND = 'cad_pick_directory';

/**
 * Whether the carrier can open a local directory picker.
 *
 * Only the exact reported `native-below` carrier qualifies (Linux native,
 * Windows, macOS, Android). `dom-canvas` and an unrecognised mode answer `false`.
 */
export function canPickDirectory(shell: unknown): boolean {
  return surfaceModeFromShell(shell) === SURFACE_MODE_NATIVE_BELOW;
}

/** The same decision for a mode the caller already read. */
export function pickerAvailable(mode: SurfaceModeReading): boolean {
  return mode === SURFACE_MODE_NATIVE_BELOW;
}

/**
 * Open the host's directory picker.
 *
 * Returns the chosen absolute path, or `null` when the user cancelled or the
 * host answered with no path. A refusal (a carrier without a picker, a closed
 * session) is thrown, so the caller reports it instead of pretending the user
 * cancelled — those are different facts.
 */
export async function pickDirectory(invoke: Invoke): Promise<string | null> {
  const answer = await invoke<unknown>(PICK_DIRECTORY_COMMAND);
  if (typeof answer !== 'string') return null;
  const path = answer.trim();
  return path.length > 0 ? path : null;
}

/**
 * Whether the host can name the directory fonts are read from.
 *
 * `fontsDir` is omitted — not `null`, not empty — when the carrier has no local
 * font directory (Web) or nothing resolves. An absent key is therefore "no
 * value", and the row renders in the disabled/missing pattern instead of as an
 * editable field with nothing behind it.
 */
export function fontsDirKnown(
  settings: Readonly<Record<string, unknown>> | null | undefined,
): boolean {
  const value = settings?.fontsDir;
  return typeof value === 'string' && value.trim().length > 0;
}

/**
 * Whether the host reported that the saved choice is no longer usable.
 *
 * The flag is always present in the snapshot and is decisive: it is computed by
 * the host from the real directory, which the chrome cannot see (a `--fonts-dir`
 * value and a valid saved choice are different paths on purpose). Only the
 * explicit `true` counts; an unreadable or missing flag is no warning.
 */
export function fontsDirMissing(
  settings: Readonly<Record<string, unknown>> | null | undefined,
): boolean {
  return settings?.fontsDirMissing === true;
}
