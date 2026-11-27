// The Scenes tray wire: its normalisation and the one read.
//
// The host owns the projection (`cad_scenes` → `kind: "scenes"`): the open
// document's persisted scene rows in save order. This module only validates the
// answer and never re-orders or invents rows. Applying a scene is a separate
// dispatch (`SCENE RESTORE <name>`), so a read can never change the camera.
//
// Wire shape (frozen):
//
// ```jsonc
// { "scenes": [{ "name": "Overview", "is3d": false, "space": "Model",
//                "style": "SHADED" }],
//   "total": 1, "truncated": false }
// ```

import type { Invoke } from '../../host/invoke.ts';

/** The one read command (`kind: "scenes"`). */
export const SCENES_COMMAND = 'cad_scenes';

/** One scene row. */
export interface SceneRow {
  /** Stored name (the key `SCENE RESTORE`/`DELETE` takes). */
  readonly name: string;
  /** Whether the scene captured the 3D observation mode. */
  readonly is3d: boolean;
  /** Display name of the active space. */
  readonly space: string;
  /** `WIREFRAME`/`SHADED` (or the host's raw token). */
  readonly style: string;
}

/** One normalised answer. */
export interface ScenesAnswer {
  readonly scenes: readonly SceneRow[];
  /** The host's own total (may exceed `scenes.length`). */
  readonly total: number;
  readonly truncated: boolean;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function countOf(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

/** Normalise one `cad_scenes` answer; a row without a name is dropped. */
export function parseScenesAnswer(answer: unknown): ScenesAnswer {
  const source = recordOf(answer);
  const rawRows: unknown[] = Array.isArray(source?.scenes) ? source.scenes : [];
  const scenes: SceneRow[] = [];
  const seen = new Set<string>();
  for (const raw of rawRows) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const name = typeof fields.name === 'string' && fields.name !== '' ? fields.name : null;
    if (name === null || seen.has(name)) continue;
    seen.add(name);
    scenes.push({
      name,
      is3d: fields.is3d === true,
      space: typeof fields.space === 'string' ? fields.space : '',
      style: typeof fields.style === 'string' ? fields.style : '',
    });
  }
  const reported = source?.total;
  return {
    scenes,
    total: typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= 0
      ? reported
      : scenes.length,
    truncated: source?.truncated === true,
  };
}

/** The one read. */
export async function readScenes(invoke: Invoke): Promise<ScenesAnswer> {
  return parseScenesAnswer(await invoke(SCENES_COMMAND));
}
