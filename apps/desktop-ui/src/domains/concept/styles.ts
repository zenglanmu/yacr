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
