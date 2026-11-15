// The three short "one typed record" grammars the React command line had no row
// for: `ADDSELECTED`, `MVSETUP` and `ALIASEDIT`. They live in one sibling module
// because each is a single short record line (a point, a grid triple, an alias
// row) rather than a multi-option submenu, and each mirrors a different host
// parser:
//
//   * `crates/cad-ui-core/src/session/commands/shape_block.rs` —
//     `parse_addselected_argument`;
//   * `crates/cad-ui-core/src/session/commands/paper_arguments.rs` —
//     `parse_mvsetup_argument`;
//   * `crates/cad-ui-core/src/session/command_overrides.rs` —
//     `CadSession::aliasedit_command`.
//
// Only the token shape is decided here. Every semantic answer stays the host's:
// which source entity type `ADDSELECTED` can copy (`addselected.refusal` for
// ARC/POLYLINE/INSERT/…), whether the layout is a paper layout, the viewport
// total bound and the scale-name resolver (`mvsetup.invalid_argument` /
// `resolve_viewport_scale`), the alias target vocabulary, the essential-command
// refusal and the runtime-only row bookkeeping (`aliasedit.malformed` /
// `redefine.essential`). Nothing is clamped, defaulted or repaired.

import { isSignedDecimal, tokens } from './argument-clip-grammar.ts';

/**
 * `ADDSELECTED <x> <y>`: exactly two signed decimals on the flat work plane.
 *
 * The current selection is the source entity; the command layer decides whether
 * that source's type/properties are representable and names the refusal
 * otherwise (this build copies LINE/CIRCLE/POINT/TEXT/MTEXT).
 */
export function isAddSelectedArgument(argument: string): boolean {
  const parts = tokens(argument);
  return parts.length === 2 && parts.every(isSignedDecimal);
}

/**
 * `MVSETUP <rows> <cols> <scale>`: two positive whole grid counts plus one scale
 * token.
 *
 * The counts are digits only (`VPORTS`' shape, the same host `u32` parse), and
 * the third token is kept verbatim because the application resolves it as a
 * `VPSCALE` name, a `1:100` ratio or a plain number and refuses an unusable one
 * by name. The AutoCAD-Standard configuration, the viewport-total cap and the
 * Align/Create/Options/title-block prompts it does not offer are the host's
 * declared boundaries, not this predicate's.
 */
export function isMvSetupArgument(argument: string): boolean {
  const parts = tokens(argument);
  return parts.length === 3
    && /^[0-9]+$/.test(parts[0]) && /^[0-9]+$/.test(parts[1])
    && Number(parts[0]) >= 1 && Number(parts[1]) >= 1;
}

/**
 * `ALIASEDIT ADD <alias> <target…>`, `ALIASEDIT REMOVE <alias>` or
 * `ALIASEDIT LIST`.
 *
 * A bare `ALIASEDIT` never reaches this predicate: it is `LIST` on the name-only
 * funnel. `ADD`'s target is the whole trimmed remainder (the host accepts a
 * multi-token remainder and validates it), and `REMOVE` takes exactly one alias.
 * The alias/target vocabulary, shadowing rules and `redefine.essential` are the
 * host's.
 */
export function isAliasEditArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'LIST') return parts.length === 1;
  if (word === 'ADD') return parts.length >= 3;
  return word === 'REMOVE' && parts.length === 2;
}
