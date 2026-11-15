// The `HATCHEDIT <option>` typed-argument grammar, split out of
// `argument-commands.ts` when the option table pushed that file against the
// 1000-line code gate (the same reason `argument-clip-grammar.ts` exists).
//
// This predicate mirrors the **host** parser
// `crates/cad-ui-core/src/session/hatchedit.rs` token for token, so the line the
// chrome admits is the line the host accepts:
//
//   * the option word is matched ASCII case-insensitively;
//   * a number is one plain signed decimal (`45`, `-22.5`, `+30`; no exponent,
//     no bare `.`/`5.`, no embedded spaces) — the host's `parse_signed_decimal`;
//   * `SCALE` additionally requires strictly greater than zero;
//   * `COLOR`/`TRANSPARENCY` take what Rust's `str::parse::<u8>()` takes (an
//     optional `+` and digits), bounded to `1..=255` / `0..=90` respectively;
//   * `LAYER` takes the whole remainder, spaces included;
//   * `PATTERNFILE` takes `<path.pat> [<name>]` — one or two tokens, and a second
//     token carrying a path separator is a spaced path the grammar cannot split
//     from the name (the host refuses it as `hatchedit.pattern_file_path`).
//
// Everything **semantic** stays the host's, and is never second-guessed here:
//
//   * the built-in pattern-name table (`PATTERN <name>` forwards any single token;
//     an unknown name is the host's `hatchedit.unknown_pattern`, so the chrome
//     owns no second name list);
//   * the pattern-fill requirement of `ANGLE`/`SCALE`/`ORIGIN`/`PATTERN`;
//   * the named refusals `ASSOCIATE`/`DISASSOCIATE`/`ANNOTATIVE` (no
//     associativity/annotative model) and `DRAWORDER BOUNDARY` (no stored
//     boundary record) — all three option words are forwarded so the **host**
//     names the missing model instead of the chrome pretending the word is not
//     part of the grammar;
//   * reading a `PATTERNFILE` path: the chrome never touches a file and never
//     checks that the path exists (the host's resource strategy reads it).
//
// The predicate also marks the one **case-sensitive** tail
// ([`hatchEditArgumentCarriesPath`]: a `PATTERNFILE` path), so admission can
// forward it with the spelling the user typed instead of the shared upper-cased
// normal form.

import { isSignedDecimal, tokens } from './argument-clip-grammar.ts';

/**
 * The option forms that take **no** argument: the fill switches, the two
 * boundary edits, and the three named refusals the host answers by name.
 *
 * `ORIGIN`, `PATTERNFILE` and every other form carry a value and are handled
 * below.
 */
const STANDALONE: ReadonlySet<string> = new Set([
  'SOLID',
  'GRADIENT',
  'RECREATEBOUNDARY',
  'SEPARATE',
  'ADDBOUNDARIES',
  'REMOVEBOUNDARIES',
  // Named refusals: the host names the missing model
  // (`hatchedit.no_association` / `hatchedit.annotative_unsupported`).
  'ASSOCIATE',
  'DISASSOCIATE',
  'ANNOTATIVE',
]);

/** The one-keyword options and their accepted words (host `eq_ignore_ascii_case`). */
const KEYWORDS: Readonly<Record<string, readonly string[]>> = Object.freeze({
  PATTERNTYPE: Object.freeze(['USER', 'PREDEFINED', 'CUSTOM']),
  STYLE: Object.freeze(['IGNORE', 'OUTER', 'NORMAL']),
  // `BOUNDARY` is admitted: the host owns that named refusal
  // (`hatchedit.draw_order_boundary`), so it must reach the router.
  DRAWORDER: Object.freeze(['BEHIND', 'FRONT', 'BOUNDARY']),
});

/**
 * Whether `text` is what Rust's `str::parse::<u8>()` accepts: an optional `+`
 * followed by ASCII digits, within a `u8` (`0..=255`).
 *
 * The host parses the `COLOR` ACI index and the `TRANSPARENCY` percentage
 * through `u8`, so `COLOR +30` and `COLOR 030` are admissible there and here,
 * while `COLOR 1.0`, `COLOR 1e1` and `COLOR -1` are not.
 */
function isByte(text: string): boolean {
  if (!/^\+?[0-9]+$/.test(text)) return false;
  const value = Number(text);
  return Number.isInteger(value) && value >= 0 && value <= 255;
}

/**
 * Whether `argument` (the text after `HATCHEDIT`) is one admitted option form.
 *
 * A bare `HATCHEDIT` never reaches this predicate: it keeps the name-only funnel
 * and the host answers its explicit `hatchedit.needs_option` refusal.
 */
export function isHatchEditArgument(argument: string): boolean {
  // The option words and keywords are ASCII case-insensitive on both sides, hence
  // the fold. The predicate only answers true/false: it never rewrites the line,
  // so a `PATTERNFILE` path is not resolved, normalised, quoted or stat-ed here.
  // (The shared `normalizeCommandText` funnel normalises the case of **every**
  // typed line before IPC, `PATTERNFILE` included; that is one rule for the whole
  // command line, not a second one for this option.)
  const parts = tokens(argument.toUpperCase());
  const word = parts[0];
  if (!word) return false;
  const rest = parts.slice(1);

  if (STANDALONE.has(word)) return rest.length === 0;
  if (word === 'ANGLE') return rest.length === 1 && isSignedDecimal(rest[0]);
  if (word === 'SCALE') {
    // Strictly positive, exactly like the host's `parse_signed_decimal` + `> 0`.
    return rest.length === 1 && isSignedDecimal(rest[0]) && Number(rest[0]) > 0;
  }
  // A pattern name is one whitespace-free token. Which names exist is the host's
  // one library table, so nothing is resolved (or rejected) here.
  if (word === 'PATTERN') return rest.length === 1;
  if (word === 'PATTERNFILE') {
    // `<path.pat> [<name>]`, split exactly like the host: a third token is
    // malformed, and a second token carrying a path separator is a path with
    // spaces the grammar cannot tell from the name. The path is never resolved,
    // normalised or stat-ed here.
    if (rest.length === 0 || rest.length > 2) return false;
    return rest.length === 2 ? !/[\\/]/.test(rest[1]) : true;
  }
  if (word === 'ORIGIN') {
    // Two signed decimals: the pattern-origin displacement.
    return rest.length === 2 && rest.every(isSignedDecimal);
  }
  const keywords = KEYWORDS[word];
  if (keywords) return rest.length === 1 && keywords.includes(rest[0]);
  if (word === 'COLOR') {
    // `BYLAYER` or an ACI index in `1..=255` (0 and 256 are symbolic on the host).
    return rest.length === 1
      && (rest[0] === 'BYLAYER' || (isByte(rest[0]) && Number(rest[0]) >= 1));
  }
  if (word === 'TRANSPARENCY') {
    // `BYLAYER` or the host's entity bound: 0..=90 percent transparent.
    return rest.length === 1
      && (rest[0] === 'BYLAYER' || (isByte(rest[0]) && Number(rest[0]) <= 90));
  }
  // The layer name is the whole remainder: an empty one is the only refusal here
  // (an unknown layer is the host's `hatchedit.unknown_layer`).
  if (word === 'LAYER') return rest.length >= 1;
  return false;
}

/**
 * Whether this `HATCHEDIT` argument tail carries text whose **spelling** is
 * load-bearing: a `PATTERNFILE` `.pat` path, which the host's resource strategy
 * reads from the filesystem.
 *
 * Admission forwards such a tail with the case the user typed (`commands.ts`'s
 * `PATH_ARGUMENTS`) instead of the shared upper-cased normal form; every other
 * option is a keyword, a number or a name the host matches case-insensitively and
 * keeps the normal form. The predicate only classifies the option word — the path
 * itself is never read, resolved or checked here.
 */
export function hatchEditArgumentCarriesPath(argument: string): boolean {
  return tokens(argument.toUpperCase())[0] === 'PATTERNFILE';
}
