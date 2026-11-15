// The `SSX` typed-argument grammar (the Express-Tool selection-filter builder),
// split out of `argument-commands.ts` when this batch pushed that table against
// the 1000-line code gate (the `argument-clip-grammar.ts` pattern).
//
// The predicate mirrors the **host** parser
// `crates/cad-ui-core/src/session/commands/filters.rs` `parse_ssx_argument` line
// for line:
//
//   * `BLOCK|COLOR|ENTITY|LAYER|LTYPE|STYLE <value>` — the value is the whole
//     trimmed remainder (spaces allowed, must be non-empty);
//   * `FLAG` and `PICK` — no value at all;
//   * `THICKNESS <value>` and `VECTOR <x> <y> <z>` — one resp. three finite
//     numbers;
//   * `CLEAR <criterion>` — exactly one of the nine criterion words.
//
// Two properties matter and are deliberate:
//
//   * `FLAG`, `THICKNESS` and `VECTOR` are **admitted** here even though the
//     entity-flags/thickness/extrusion criteria have no model in this build:
//     applying them is the host's named `ssx.*` refusal. Refusing them locally
//     would flatten that named refusal into the chrome's generic
//     `command.unsupported` — the rule every sibling row follows.
//   * a number is what Rust's `f64::from_str` takes and `is_finite` accepts
//     (sign, `.5`, `5.`, an exponent), because that is what the host parses; the
//     finite check only excludes `inf`/`nan`, which is the host's own rule.
//
// The matching itself (`ssx.selection_required`, the empty-filter rule, the
// unmodelled-criterion refusals) is the application layer's.

import { tokens } from './argument-clip-grammar.ts';

/** The six criteria whose value is the trimmed remainder. */
const SSX_NAMED: ReadonlySet<string> = new Set([
  'BLOCK', 'COLOR', 'ENTITY', 'LAYER', 'LTYPE', 'STYLE',
]);

/** The nine criteria `CLEAR` accepts. */
const SSX_CRITERIA: ReadonlySet<string> = new Set([
  'BLOCK', 'COLOR', 'ENTITY', 'FLAG', 'LAYER', 'LTYPE', 'STYLE', 'THICKNESS', 'VECTOR',
]);

/**
 * Whether `text` is one finite number in the shape Rust's `f64::from_str`
 * accepts: an optional sign, digits with an optional fraction (a bare `.5` and a
 * trailing `5.` included), and an optional exponent. `inf`/`nan` are the host's
 * own `is_finite` refusal, so they are not this shape.
 */
function isFiniteNumber(text: string): boolean {
  if (!/^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$/.test(text)) return false;
  return Number.isFinite(Number(text));
}

/**
 * `SSX <criterion> …`: one selection-filter criterion line.
 *
 * Keywords are ASCII case-insensitive and the split is the first whitespace
 * (exactly like the host's `find(char::is_whitespace)`), so a value with spaces
 * is never mistaken for a second criterion. A bare `SSX` never reaches this
 * predicate: it keeps the name-only funnel.
 */
export function isSsxArgument(argument: string): boolean {
  const trimmed = argument.trim();
  const split = trimmed.search(/[ \t]/);
  const keyword = (split < 0 ? trimmed : trimmed.slice(0, split)).toUpperCase();
  const rest = split < 0 ? '' : trimmed.slice(split).trim();
  if (SSX_NAMED.has(keyword)) return rest.length > 0;
  if (keyword === 'FLAG' || keyword === 'PICK') return rest.length === 0;
  if (keyword === 'THICKNESS') return isFiniteNumber(rest);
  if (keyword === 'VECTOR') {
    const parts = tokens(rest);
    return parts.length === 3 && parts.every(isFiniteNumber);
  }
  if (keyword === 'CLEAR') return SSX_CRITERIA.has(rest.toUpperCase());
  return false;
}
