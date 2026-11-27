// The Match Photo / Overlays tray wires.
//
// Match Photo answers a capability statement (`modeled: false` plus the stable
// reason); Overlays answers the renderer-owned channel rows. Neither read writes
// anything; the panel actions dispatch `MATCHPHOTO` / `OVERLAYS`, which the host
// answers with the same named refusals.
//
// Wire shapes (frozen):
//
// ```jsonc
// { "modeled": false, "reason": "matchphoto.not_implemented: ..." }
// { "rows": [{ "key": "axes", "active": true }] }
// ```

import type { Invoke } from '../../host/invoke.ts';

/** The Match Photo read command. */
export const MATCH_PHOTO_COMMAND = 'cad_match_photo';
/** The Overlays read command. */
export const OVERLAYS_COMMAND = 'cad_overlays';

/** One Match Photo answer. */
export interface MatchPhotoAnswer {
  readonly modeled: boolean;
  readonly reason: string;
}

/** One overlay channel. */
export interface OverlayRow {
  readonly key: string;
  readonly active: boolean;
}

/** One Overlays answer. */
export interface OverlaysAnswer {
  readonly rows: readonly OverlayRow[];
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Normalise one `cad_match_photo` answer. */
export function parseMatchPhotoAnswer(answer: unknown): MatchPhotoAnswer {
  const source = recordOf(answer);
  if (source === null) throw new Error('matchphoto.invalid_answer: expected an object');
  return {
    modeled: source.modeled === true,
    reason: typeof source.reason === 'string' ? source.reason : '',
  };
}

/** Normalise one `cad_overlays` answer; a row without a key is dropped. */
export function parseOverlaysAnswer(answer: unknown): OverlaysAnswer {
  const source = recordOf(answer);
  if (source === null) throw new Error('overlays.invalid_answer: expected an object');
  const rows: OverlayRow[] = [];
  for (const raw of Array.isArray(source.rows) ? source.rows : []) {
    const row = recordOf(raw);
    if (!row) continue;
    const key = typeof row.key === 'string' && row.key !== '' ? row.key : null;
    if (key === null || rows.some(existing => existing.key === key)) continue;
    rows.push({ key, active: row.active === true });
  }
  return { rows };
}

/** The Match Photo read. */
export async function readMatchPhoto(invoke: Invoke): Promise<MatchPhotoAnswer> {
  return parseMatchPhotoAnswer(await invoke(MATCH_PHOTO_COMMAND));
}

/** The Overlays read. */
export async function readOverlays(invoke: Invoke): Promise<OverlaysAnswer> {
  return parseOverlaysAnswer(await invoke(OVERLAYS_COMMAND));
}
