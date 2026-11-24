// `NEWVIEW <name>` / `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>` grammar.
//
// Split out of `argument-commands.ts` for the code-size gate. This is only the
// local syntax gate: the host resolves the viewport, validates the rectangle and
// writes the persisted named-view table.
import { isSignedDecimal } from './argument-clip-grammar.ts';

/**
 * Whether `text` is a `NEWVIEW` argument: a non-empty name, or
 * `WINDOW <name> <x1> <y1> <x2> <y2>` with four plain signed decimals.
 */
export function isNewViewArgument(text: string): boolean {
  const key = text.trim();
  if (key.length === 0) return false;
  const parts = key.split(/\s+/);
  if (!parts[0] || parts[0].toUpperCase() !== 'WINDOW') return true;
  return parts.length >= 6 && parts.slice(-4).every(isSignedDecimal);
}

export const NEWVIEW_ARGUMENT_SPEC = Object.freeze({ parse: isNewViewArgument });
