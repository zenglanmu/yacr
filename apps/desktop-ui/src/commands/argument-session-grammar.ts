// The session-token / session-vocabulary typed-argument grammars
// (`WIPEOUTFRAME`, `APERTURE`, `PTYPE`), split out of `argument-commands.ts` to
// stay inside the 1000-line code gate (the `argument-color-grammar.ts` pattern).
//
// Each predicate is the **token shape** only. The closed domains are the host's
// named refusals (`wipeoutframe.out_of_range`, `aperture.out_of_range`,
// `ptype.invalid_argument`, and the vocabulary's own refusals), so an
// out-of-range value travels to the router instead of becoming the chrome's
// generic `command.unsupported`. Nothing is clamped, defaulted or mapped to a
// different value, and no range test is duplicated here.
//
// A selector whose grammar is a **keyword table** (not a value) is deliberately
// absent from this module: a local copy of the host's words would be a second
// grammar, so those rows are not wired at all (see `argument-commands.ts`).

import { isIntegerToken, isSignedDecimal, tokens } from './argument-clip-grammar.ts';

/**
 * `WIPEOUTFRAME <0|1|2>`: the session wipeout-frame state (the sysvar spelling of
 * `WIPEOUT Frames`).
 *
 * One integer token. `0`/`1`/`2` are the documented states, and an integer
 * outside them (`9`, `-1`) still has the integer shape, so it travels and the
 * host names it (`wipeoutframe.out_of_range`). A word (`ON`) or a second token is
 * not the shape at all and stays a local refusal — the host answers those with
 * its generic argument sentence, not a code.
 */
export function isWipeoutFrameArgument(argument: string): boolean {
  return isIntegerToken(argument.trim());
}

/**
 * `APERTURE <n>`: the object-snap aperture in screen pixels.
 *
 * Digits only, mirroring the host parser's unsigned shape (no sign, no fraction,
 * no exponent). `0` and a value above the view range are still that shape, so
 * they travel and the host names them (`aperture.out_of_range`).
 */
export function isApertureArgument(argument: string): boolean {
  return /^[0-9]+$/.test(argument.trim());
}

/**
 * `PTYPE <mode> [<size>]`: the POINT display style (`$PDMODE`, `$PDSIZE`).
 *
 * One or two numeric tokens, the same two grammars the dedicated `PDMODE`/
 * `PDSIZE` rows use. A fractional mode (`PTYPE 3.5`), an out-of-`i16` mode
 * (`PTYPE 40000`) and a non-finite size are the host's `ptype.invalid_argument`,
 * so the shape is admitted here (a decimal is a decimal) and the value's
 * integrality stays the host's — exactly like the `PDMODE` row.
 */
export function isPtypeArgument(argument: string): boolean {
  const parts = tokens(argument);
  if (parts.length < 1 || parts.length > 2) return false;
  return parts.every(isSignedDecimal);
}
