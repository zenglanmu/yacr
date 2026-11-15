// The clip-family typed-argument grammar (`CLIP`, `XCLIP`) plus the shared token
// helpers, split out of `argument-commands.ts` when the clipping batch pushed
// that table past the 1000-line code gate.
//
// Nothing here decides anything by itself: every predicate is a local **syntax**
// aid, and the host stays the authority for semantics (rectangle ordering,
// polygon validity, the world/block-local mapping and every declared render
// limit).
//
// The helpers are exported so the table and its sibling grammar modules keep
// exactly one definition of "one plain signed decimal", "one plain **positive**
// decimal", "one integer token" and of whitespace tokenisation; this module never
// imports from its parent or its siblings, so the dependency stays one-way.

/** Split a typed argument into whitespace-separated tokens. */
export function tokens(argument: string): readonly string[] {
  return argument.trim().split(/\s+/).filter(part => part.length > 0);
}

/**
 * Whether `text` is one integer token: digits only, with an optional explicit
 * sign (`1`, `0`, `256`, `+030`, `-1`; never `1.0`, `1e3` or `0x1`).
 *
 * The **magnitude/range** is never judged here: a token too long for a JS number
 * is still the integer shape the host parses (and refuses by its own domain), so
 * it must travel. This is the `COLOR` ACI's shape and the session tokens'
 * (`WIPEOUTFRAME`) shape, defined once for both.
 */
export function isIntegerToken(text: string): boolean {
  return /^[+-]?[0-9]+$/.test(text);
}

/**
 * Whether `text` is a finite decimal with an optional explicit sign (`45`,
 * `22.5`, `0`, `-90`, `+30`).
 *
 * `TORIENT` accepts an absolute rotation in either direction, so the host's
 * grammar admits an explicit `+`; this mirrors it for local admission. It is
 * otherwise the same single plain decimal (no exponent, no bare `.`/`5.`, no
 * embedded spaces).
 */
export function isSignedDecimal(text: string): boolean {
  if (!/^[+-]?[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  return Number.isFinite(Number(text));
}

/**
 * Whether `text` is one plain positive decimal (`2`, `0.5`, `1.25`; never `0`,
 * `-1` or `+2`).
 *
 * This is the host's `parse_positive_decimal`, shared by every selector whose
 * value is a scale factor or spacing: no sign of any kind, no exponent, no bare
 * `.`/`5.`. The magnitude is never judged beyond `> 0` — a value outside a
 * documented range is the host's named refusal, not a local one.
 */
export function isPositiveDecimal(text: string): boolean {
  if (!/^[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  const value = Number(text);
  return Number.isFinite(value) && value > 0;
}

/**
 * `XCLIP RECT <minx> <miny> <maxx> <maxy>`, `XCLIP POLY <x1> <y1> …`,
 * `XCLIP ON|OFF|DELETE|INVERT` or `XCLIP ?`.
 *
 * This is the local **syntax** gate only: `RECT` takes exactly four signed
 * decimals, `POLY` an even count of at least six signed decimals (three point
 * pairs), and the four switch words plus `?` stand alone. Coordinates are
 * block-local; rectangle ordering, polygon validity, the axis-aligned world
 * mapping and every declared render limit stay the host's (`xclip.*` refusals),
 * so nothing is rejected or clamped here for semantics.
 */
export function isXclipArgument(argument: string): boolean {
  const parts = tokens(argument);
  const head = parts[0];
  if (!head) return false;
  if (head === '?') return parts.length === 1;
  const word = head.toUpperCase();
  if (word === 'ON' || word === 'OFF' || word === 'DELETE' || word === 'INVERT') {
    return parts.length === 1;
  }
  const coordinates = parts.slice(1);
  if (word === 'RECT') {
    return coordinates.length === 4 && coordinates.every(isSignedDecimal);
  }
  if (word === 'POLY') {
    return coordinates.length >= 6
      && coordinates.length % 2 === 0
      && coordinates.every(isSignedDecimal);
  }
  return false;
}

/**
 * `CLIP RECT <minx> <miny> <maxx> <maxy>`, `CLIP POLY <x1> <y1> …` or
 * `CLIP ON|OFF|DELETE`: the generic boundary form over the same two clip models
 * `IMAGECLIP` and `XCLIP` write.
 *
 * This is the local **syntax** gate only. `RECT` takes exactly four signed
 * decimals (in the target's own clip space: image pixels, block-local for a
 * reference), `POLY` an even count of at least six world-space signed decimals,
 * and the three state words stand alone. The target kind, the rectangle
 * ordering, the polygon validity, the "a block reference has no renderable
 * polygon clip" refusal and the "images have no ON/OFF" rule are all host
 * decisions (`clip.*`, `imageclip.*`, `xclip.*`), so nothing is rejected here
 * for semantics.
 *
 * A bare `CLIP` stays name-only and the host answers its payload refusal.
 * `CLIPIT` has no grammar at all (its input is the selection), so it is not a
 * typed-argument selector.
 */
export function isClipArgument(argument: string): boolean {
  const parts = tokens(argument);
  const head = parts[0];
  if (!head) return false;
  const word = head.toUpperCase();
  if (word === 'ON' || word === 'OFF' || word === 'DELETE') {
    return parts.length === 1;
  }
  const coordinates = parts.slice(1);
  if (word === 'RECT') {
    return coordinates.length === 4 && coordinates.every(isSignedDecimal);
  }
  if (word === 'POLY') {
    return coordinates.length >= 6
      && coordinates.length % 2 === 0
      && coordinates.every(isSignedDecimal);
  }
  return false;
}
