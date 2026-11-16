// The typed-argument grammars this wave added to the command line: `LINETYPE` /
// `-LINETYPE`, `MLINE`, `LAYOUT` and `TABLEEXPORT`, plus the quote-aware
// tokenizer those predicates share with `argument-commands.ts`'s `RENAME`. They
// live here because the batch tables filled that file to the 1000-line code gate.
//
// Nothing here decides anything by itself: every predicate mirrors the **token
// shape** of its host parser and nothing else
// (`session/commands/extras.rs::parse_linetype_argument`,
// `commands/draw2.rs::parse_mline_argument`,
// `commands/layout.rs::parse_layout_argument`,
// `session/table_export.rs::parse_tableexport_argument`). The host keeps every
// semantic refusal **by name** — `linetype.exists` / `linetype.bad_pattern`,
// `mline.point_count`, `layout.template_not_implemented`, `tableexport.path` /
// `.no_selection` / `.empty` / `unsupported-runtime: table-export` — so no line
// whose shape is admitted may be refused here. A line outside the grammar keeps
// the chrome's existing `command.unsupported`; nothing here invents a reason.
//
// This module imports only the shared token helpers, so the dependency stays
// one-way: `argument-commands.ts` imports from here, never the reverse.

import { isPositiveDecimal, isSignedDecimal, tokens } from './argument-clip-grammar.ts';

/** One leading option word and the whole remainder (trimmed at both ends). */
interface SplitWord {
  readonly word: string;
  readonly rest: string;
}

/**
 * Split `word <rest>` like the host's `split_word`: the first whitespace-delimited
 * run, then everything after it with only its leading whitespace dropped.
 *
 * Returns `null` for an empty argument. The remainder keeps its internal spacing
 * because `LINETYPE SET`/`DELETE` and `LAYOUT`/`TABLEEXPORT` treat it as one name
 * or one path.
 */
function splitWord(argument: string): SplitWord | null {
  const trimmed = argument.trim();
  if (trimmed.length === 0) return null;
  const separated = trimmed.search(/\s/);
  if (separated < 0) return { word: trimmed, rest: '' };
  return {
    word: trimmed.slice(0, separated),
    rest: trimmed.slice(separated).trim(),
  };
}

/**
 * Split whitespace-separated tokens while keeping double-quoted runs together.
 * Returns `null` for an unbalanced quote. Unquoted empty runs are ignored.
 *
 * The same tokenizer the host's `split_quoted_tokens` uses for `RENAME` and
 * `LINETYPE CREATE`: a quoted run is one token whose quotes are dropped, so a
 * name with a space has one unambiguous boundary.
 */
export function splitQuotedTokens(text: string): string[] | null {
  const parts: string[] = [];
  let current = '';
  let quoted = false;
  let started = false;
  for (const ch of text.trim()) {
    if (ch === '"') {
      quoted = !quoted;
      started = true;
    } else if (/\s/.test(ch) && !quoted) {
      if (started || current.length > 0) {
        parts.push(current);
        current = '';
        started = false;
      }
    } else {
      current += ch;
      started = true;
    }
  }
  if (quoted) return null;
  if (started || current.length > 0) parts.push(current);
  return parts;
}

/**
 * The `CREATE` half of the `LINETYPE` grammar: `CREATE <name> <pattern>
 * [DESCRIPTION <text>]`.
 *
 * `<name>` is one token or one `"quoted"` run. `<pattern>` is **one** token of
 * comma-separated signed decimals (`2,-1`, `3,-1,1,-3`): an empty entry, a
 * non-decimal entry and a missing entry are outside the grammar. A trailing
 * `DESCRIPTION` takes at least one word. The pattern's dash/gap ordering, a name
 * that already exists and the definition table's limits are the host's named
 * refusals (`linetype.bad_pattern` / `linetype.exists`), never decided here.
 */
function isLinetypeCreate(rest: string): boolean {
  const parts = splitQuotedTokens(rest);
  if (parts === null || parts.length < 2) return false;
  const [name, pattern, ...remaining] = parts;
  if (name.trim().length === 0) return false;
  const entries = pattern.split(',').map(entry => entry.trim());
  if (entries.length === 0) return false;
  if (!entries.every(entry => entry.length > 0 && isSignedDecimal(entry))) return false;
  if (remaining.length === 0) return true;
  return remaining[0].toUpperCase() === 'DESCRIPTION' && remaining.length > 1;
}

/**
 * `LINETYPE LIST`, `LINETYPE SET <name>`, `LINETYPE DELETE <name>` or
 * `LINETYPE CREATE <name> <pattern> [DESCRIPTION <text>]` (and the `-LINETYPE`
 * spelling, which the host dispatches to the same parser).
 *
 * `SET`/`DELETE` take the whole non-empty remainder as the name, exactly like the
 * host, so a name with a space is not refused locally. `CREATE` uses the
 * quote-aware shape above. Whether the name exists, whether the pattern is a
 * valid dash/gap sequence and whether the table has room are `linetype.*`
 * refusals and stay the host's.
 */
export function isLinetypeArgument(argument: string): boolean {
  const split = splitWord(argument);
  if (split === null) return false;
  const word = split.word.toUpperCase();
  if (word === 'LIST') return split.rest.length === 0;
  if (word === 'SET' || word === 'DELETE') return split.rest.length > 0;
  return word === 'CREATE' && isLinetypeCreate(split.rest);
}

/**
 * Whether this `LINETYPE` line is a `CREATE … DESCRIPTION <text>` form, whose
 * description is **stored content**.
 *
 * The description becomes the linetype's description, so its spelling is content
 * and admission forwards the tail with the typed case (the
 * `CASE_PRESERVING_ARGUMENTS` mechanism `TOLERANCE` uses). Only the `DESCRIPTION`
 * form qualifies: `SET`/`DELETE` tails are names the host matches, and a
 * `CREATE` without a description has no content tail at all.
 */
export function linetypeCreateCarriesDescription(argument: string): boolean {
  const split = splitWord(argument);
  if (split === null || split.word.toUpperCase() !== 'CREATE') return false;
  const parts = splitQuotedTokens(split.rest);
  if (parts === null || parts.length < 3) return false;
  return parts[2].toUpperCase() === 'DESCRIPTION';
}

/** Whether `text` is one `x,y` pair of plain signed decimals. */
function isCoordinatePair(text: string): boolean {
  const parts = text.split(',');
  return parts.length === 2 && parts.every(isSignedDecimal);
}

/**
 * `MLINE <spacing> <offset> <x1,y1> <x2,y2> [<x3,y3> …]`.
 *
 * A positive spacing, a signed offset, then **every** remaining token must be one
 * `x,y` pair; at least two vertices are required, which is exactly the host
 * parser's `tokens.len() < 4` cut. The vertex count is deliberately unbounded
 * here: the command layer owns the declared `2..=1024` bound and answers
 * `mline.point_count` by name, so imposing it locally would mask that sentence.
 */
export function isMlineArgument(argument: string): boolean {
  const parts = tokens(argument);
  if (parts.length < 4) return false;
  if (!isPositiveDecimal(parts[0]) || !isSignedDecimal(parts[1])) return false;
  return parts.slice(2).every(isCoordinatePair);
}

/**
 * `LAYOUT LIST|NEW <name>|RENAME <old> <new>|DELETE <name>|SET <name>|
 * COPY <source> <new-name>|TEMPLATE [<file>]`.
 *
 * `TEMPLATE` is admitted on purpose even though this build has no cross-drawing
 * template import: the host answers `layout.template_not_implemented`, and
 * pre-empting it here would replace that named sentence with the chrome's
 * generic `command.unsupported`. `COPY`/`RENAME` need two non-empty names (the
 * host's second word takes the rest of the line, so a spaced name is legal);
 * `NEW`/`DELETE`/`SET` take a non-empty remainder.
 */
export function isLayoutArgument(argument: string): boolean {
  const split = splitWord(argument);
  if (split === null) return false;
  const word = split.word.toUpperCase();
  if (word === 'LIST') return split.rest.length === 0;
  if (word === 'NEW' || word === 'DELETE' || word === 'SET') return split.rest.length > 0;
  if (word === 'TEMPLATE') return true;
  if (word === 'COPY' || word === 'RENAME') return tokens(split.rest).length >= 2;
  return false;
}

/**
 * `TABLEEXPORT <path.csv>`: one non-empty destination path.
 *
 * The host takes the whole trimmed remainder as the path (so a path with spaces
 * is legal) and requires a `.csv` extension. The extension check is the host's
 * `tableexport.path` refusal and the writer is the host's
 * `unsupported-runtime: table-export` / `tableexport.write_failed` path, so
 * nothing is resolved, stat'ed or extension-checked here. Admission only refuses
 * the empty destination, which is outside the grammar.
 */
export function isTableExportArgument(argument: string): boolean {
  return argument.trim().length > 0;
}
