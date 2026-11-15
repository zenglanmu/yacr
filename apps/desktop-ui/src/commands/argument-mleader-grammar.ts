// The MULTILEADER / MLEDIT typed-argument grammars (`MLEDIT`, `-MLEDIT`,
// `MLEADERALIGN`, `MLEADEREDIT`, `TOLERANCE`), split out of
// `argument-commands.ts` when this batch pushed that table against the
// 1000-line code gate (the `argument-clip-grammar.ts` / `argument-hatchedit.ts`
// pattern).
//
// Every predicate mirrors the **host** parser in
// `crates/cad-ui-core/src/session/commands/mleader.rs` token for token, so the
// line the chrome admits is the line the host accepts:
//
//   * `MLEDIT MERGE|CLOSED|CROSS|TEE|OPEN <x> <y>` — one crossing pick;
//   * `MLEDIT BREAK <x1> <y1> <x2> <y2>` — the two band corners;
//   * `-MLEDIT …` — the same grammar under the command-line spelling;
//   * `MLEADERALIGN LEFT|L|RIGHT|R|CENTER|C [<spacing>]` — the optional spacing
//     is a plain positive decimal (the host's `parse_positive_decimal`, which
//     refuses any sign, so `+2` is outside the grammar);
//   * `MLEADEREDIT ADD <x> <y>` / `MLEADEREDIT REMOVE <index>` — the index is
//     ASCII digits only (`usize::from_str` behind a digit guard, so no sign);
//   * `TOLERANCE <x> <y> [<text…>]` — two signed decimals, then free text.
//
// Nothing **semantic** is decided here and nothing is clamped, defaulted or
// repaired: the two selected lines (`mledit.*`), the selected MULTILEADERs and
// their line list (`mleaderalign.*` / `mleaderedit.*`), the frame's compartment
// text and the declared axis-aligned/≤6-compartment limits (`tolerance.*`) are
// all the command layer's named refusals. A malformed shape stays the chrome's
// existing `command.unsupported` sentence, never a new reason vocabulary.

import { isPositiveDecimal, isSignedDecimal, tokens } from './argument-clip-grammar.ts';

/** The five clean-up options that take one crossing pick. */
const MLEDIT_PICKS: ReadonlySet<string> = new Set(['MERGE', 'CLOSED', 'CROSS', 'TEE', 'OPEN']);

/** The three `MLEADERALIGN` modes and their single-letter forms. */
const MLEADER_ALIGN_MODES: ReadonlySet<string> = new Set([
  'LEFT', 'L', 'RIGHT', 'R', 'CENTER', 'C',
]);

/**
 * `MLEDIT <option> <x> <y>` / `MLEDIT BREAK <x1> <y1> <x2> <y2>`, and the
 * `-MLEDIT` spelling (one predicate for both rows: the host dispatches them to
 * the same parser and only the recorded selector differs).
 *
 * A clean-up option with four coordinate tokens or a `BREAK` with two is outside
 * the grammar and is refused locally; which two lines the pick addresses and
 * whether they are two independent LINEs is the host's call
 * (`mledit.selection` / `mledit.residual`).
 */
export function isMlEditArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'BREAK') {
    return parts.length === 5 && parts.slice(1).every(isSignedDecimal);
  }
  return MLEDIT_PICKS.has(word) && parts.length === 3 && parts.slice(1).every(isSignedDecimal);
}

/**
 * `MLEADERALIGN LEFT|L|RIGHT|R|CENTER|C [<spacing>]`.
 *
 * The spacing is the host's positive plain decimal, so a zero, a negative, a
 * `+2` or an exponent is outside the grammar. The selected leaders, the anchor
 * mode's geometry and the declared "no distribute/column options" boundary stay
 * the host's (`mleaderalign.*`).
 */
export function isMLeaderAlignArgument(argument: string): boolean {
  const parts = tokens(argument);
  if (parts.length < 1 || parts.length > 2) return false;
  if (!MLEADER_ALIGN_MODES.has(parts[0].toUpperCase())) return false;
  return parts.length === 1 || isPositiveDecimal(parts[1]);
}

/**
 * `MLEADEREDIT ADD <x> <y>` / `MLEADEREDIT REMOVE <index>`.
 *
 * The arrow is a pair of signed decimals on the flat work plane; the index is
 * ASCII digits only (0-based, `usize`), so a negative or fractional index is
 * refused here. Whether the index addresses a line of the one selected leader,
 * and whether the root line may be removed, is the host's (`mleaderedit.*`).
 */
export function isMLeaderEditArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'ADD') {
    return parts.length === 3 && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2]);
  }
  return word === 'REMOVE' && parts.length === 2 && /^[0-9]+$/.test(parts[1]);
}

/**
 * `TOLERANCE <x> <y> [<text…>]`: the frame's lower-left corner plus its
 * `|`-separated compartment text.
 *
 * The host splits the two plain signed decimals off and keeps **everything after
 * them** as the text: `|` separators, internal spaces and every other token are
 * preserved verbatim. The text is therefore never tokenised,
 * counted or matched here — a `|` count, a compartment count and the declared
 * axis-aligned/estimated-width limits are `tolerance.*` refusals.
 *
 * An **empty** tail is inside the grammar: the host keeps it as an empty string
 * so the command layer can answer with its own `tolerance.text` refusal instead
 * of a grammar error. Refusing it here would mask that named sentence.
 *
 * The tail's spelling is load-bearing (it becomes TEXT entities), so admission
 * forwards it with the typed case — the selector is listed in `commands.ts`'s
 * case-preserving argument table exactly like `IMAGEATTACH`'s path.
 */
export function isToleranceArgument(argument: string): boolean {
  const match = /^\s*([^ \t]+)[ \t]+([^ \t]+)(?:[ \t]+[\s\S]*)?$/.exec(argument);
  if (match === null) return false;
  return isSignedDecimal(match[1]) && isSignedDecimal(match[2]);
}
