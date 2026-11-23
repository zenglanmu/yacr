// The `EXPORT <format>` token grammar, split out of `argument-commands.ts` so
// that table stays inside the 1000-line code gate (the `argument-clip-grammar.ts`
// precedent: a plain sibling module, not a barrel).
//
// `EXPORT <TOKEN>` is one argument form of the `EXPORT` selector, never a
// separate command id: the host opens the same export picker with that format's
// extension suggested, and the extension actually chosen is still what the writer
// follows (a mismatch is refused before writing, by name). So the words below are
// grammar only — nothing here decides about a format, a carrier's connector set or
// a file. A token this predicate admits is still validated by the host, and a
// format a carrier did not compile answers with its own
// `unsupported-runtime: export.<id>` refusal instead of being pre-empted here.
import type { ArgumentSpec } from './argument-commands.ts';

/**
 * The case-insensitive format tokens the host grammar accepts after `EXPORT`.
 *
 * `COFF` is listed so a typed `EXPORT COFF` reaches the host instead of becoming
 * a local "unsupported"; it has no File-panel row (see
 * `ribbon/ribbon-file-export.ts`). Every other word names a format this build
 * writes through its connector registry.
 */
const EXPORT_TOKENS: readonly string[] = Object.freeze([
  'DXF', 'DWG', 'STL', 'OBJ', 'PLY', 'OFF', 'COFF', 'IFC', 'SKP', 'GLB', 'GLTF',
]);

/** Whether `argument` is exactly one export format token (case-insensitive). */
export function isExportArgument(argument: string): boolean {
  return EXPORT_TOKENS.includes(argument.trim().toUpperCase());
}

export const EXPORT_ARGUMENT_SPEC: ArgumentSpec = Object.freeze({ parse: isExportArgument });
