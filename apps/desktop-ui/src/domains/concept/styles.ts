// The Styles panel's two real tokens.
//
// `VSCURRENT <token>` is the existing command-line selector; this module only
// names the two admitted tokens, their i18n labels, and the exact dispatch line.
// Every other SketchUp style token is refused by the host (`vscurrent.
// style_not_implemented`), never mapped onto these two.

/** The two real body display tokens, in panel order. */
export const STYLE_TOKENS = Object.freeze(['WIREFRAME', 'SHADED'] as const);

/** One admitted token. */
export type StyleToken = (typeof STYLE_TOKENS)[number];

/** Whether `value` is one of the two admitted tokens. */
export function isStyleToken(value: unknown): value is StyleToken {
  return typeof value === 'string' && (STYLE_TOKENS as readonly string[]).includes(value);
}

/** The exact command line for one token. */
export function styleCommand(token: StyleToken): `VSCURRENT ${StyleToken}` {
  return `VSCURRENT ${token}`;
}

/** The catalog label key for one token. */
export function styleLabelKey(token: StyleToken): string {
  return token === 'WIREFRAME' ? 'concept.styles.wireframe' : 'concept.styles.shaded';
}

/** The document's shared guide style, as the host reports it. */
export interface GuideStyle {
  readonly color: readonly [number, number, number];
  readonly dashed: boolean;
}

/** The host/ADR-0101 default (the old overlay constant). */
export const DEFAULT_GUIDE_COLOR: readonly [number, number, number] = Object.freeze([51, 115, 217] as const);
export const DEFAULT_GUIDE_DASHED = true;

/** Clamp one channel into the command grammar's `u8` range. */
function channel(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(255, Math.round(value)));
}

/** The exact `GUIDE STYLE COLOR` line for one sRGB triplet. */
export function guideStyleColorCommand(color: readonly [number, number, number]): string {
  return `GUIDE STYLE COLOR ${channel(color[0])} ${channel(color[1])} ${channel(color[2])}`;
}

/** The exact `GUIDE STYLE DASH` line for the shared line switch. */
export function guideStyleDashCommand(dashed: boolean): `GUIDE STYLE DASH ${'ON' | 'OFF'}` {
  return `GUIDE STYLE DASH ${dashed ? 'ON' : 'OFF'}`;
}

/** `#rrggbb` for a colour input's `value`. */
export function guideStyleHex(color: readonly [number, number, number]): string {
  return `#${color.map(part => channel(part).toString(16).padStart(2, '0')).join('')}`;
}

/** Decode a `#rrggbb` colour input value, or `null` for anything else. */
export function parseGuideStyleHex(value: string): [number, number, number] | null {
  if (!/^#[0-9a-fA-F]{6}$/.test(value)) return null;
  return [
    Number.parseInt(value.slice(1, 3), 16),
    Number.parseInt(value.slice(3, 5), 16),
    Number.parseInt(value.slice(5, 7), 16),
  ];
}
