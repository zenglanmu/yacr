// The Shadows/Fog tray wire: its normalisation and the one read.
//
// The host owns the projection (`cad_display` → `kind: "display"`): the open
// document's one display record (`tables/display.json`). This module only
// validates the answer and never invents values; writes are separate dispatches
// (`SHADOWS ...` / `FOG ...`), so a read can never change the document.
//
// Wire shape (frozen):
//
// ```jsonc
// { "shadows": { "enabled": true, "azimuth": 90, "elevation": 45 },
//   "fog": { "enabled": false, "start": 100, "end": 1000,
//            "color": [200, 210, 220] } }
// ```

import type { Invoke } from '../../host/invoke.ts';

/** The one read command (`kind: "display"`). */
export const DISPLAY_COMMAND = 'cad_display';

/** The Shadows panel's values. */
export interface ShadowsValues {
  readonly enabled: boolean;
  /** Degrees clockwise from world `+Y`. */
  readonly azimuth: number;
  /** Degrees above the horizon. */
  readonly elevation: number;
}

/** The Fog panel's values. */
export interface FogValues {
  readonly enabled: boolean;
  /** Model units. */
  readonly start: number;
  /** Model units (`> start`). */
  readonly end: number;
  /** sRGB bytes. */
  readonly color: readonly [number, number, number];
}

/** One normalised record. */
export interface DisplayAnswer {
  readonly shadows: ShadowsValues;
  readonly fog: FogValues;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function finiteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function channelOf(value: unknown, fallback: number): number {
  const number = finiteNumber(value, fallback);
  return Math.min(255, Math.max(0, Math.round(number)));
}

/**
 * Normalise one `cad_display` answer.
 *
 * A non-object answer is a transport fault and throws; a missing field falls
 * back to the record's documented default so the panel always shows a usable
 * form (the host's answer is authoritative when it is present).
 */
export function parseDisplayAnswer(answer: unknown): DisplayAnswer {
  const source = recordOf(answer);
  if (source === null) throw new Error('display.invalid_answer: expected an object');
  const shadows = recordOf(source.shadows) ?? {};
  const fog = recordOf(source.fog) ?? {};
  const rawColor: unknown[] = Array.isArray(fog.color) ? fog.color : [];
  return {
    shadows: {
      enabled: shadows.enabled === true,
      azimuth: finiteNumber(shadows.azimuth, 135),
      elevation: finiteNumber(shadows.elevation, 45),
    },
    fog: {
      enabled: fog.enabled === true,
      start: finiteNumber(fog.start, 100),
      end: finiteNumber(fog.end, 1000),
      color: [
        channelOf(rawColor[0], 200),
        channelOf(rawColor[1], 210),
        channelOf(rawColor[2], 220),
      ],
    },
  };
}

/** The one read. */
export async function readDisplay(invoke: Invoke): Promise<DisplayAnswer> {
  return parseDisplayAnswer(await invoke(DISPLAY_COMMAND));
}
