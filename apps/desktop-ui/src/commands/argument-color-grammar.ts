// The `COLOR` / `-COLOR` typed-argument grammar, split out of
// `argument-commands.ts` when this row pushed that table towards the 1000-line
// code gate.
//
// One predicate serves both spellings: `-COLOR` is AutoCAD's prompt form of the
// **same** session token, so it must never grow a second grammar. Nothing here
// decides colour semantics — no palette table, no clamping, no defaulting. The
// shape mirrors the host parser
// (`crates/cad-ui-core/src/session/commands/extras.rs::parse_color_argument`)
// token for token, while the closed ACI **domain** stays the host's: `COLOR 257`
// is the router's `color.invalid_value` refusal, so this gate admits the integer
// shape and lets that named refusal speak (the same split `IMAGEADJUST` uses for
// its percentages, whose `0..=100` range is the host's too).

/** The three symbolic words the host admits, matched case-insensitively. */
const COLOR_SYMBOLS: readonly string[] = Object.freeze(['BYLAYER', 'BYNAME', 'BYBLOCK']);

/**
 * Whether `argument` is one `COLOR` value: a symbolic word or one integer.
 *
 * Mirrors the host grammar's token shapes exactly:
 *
 * * one trimmed token — `1 2` is not one value;
 * * `BYLAYER`, `BYNAME` or `BYBLOCK`, any case;
 * * otherwise one integer with an optional explicit sign (`1`, `0`, `256`,
 *   `+030`, `-1`): digits only, so `1.0`, `1e3`, `0x1` and `aci:1` are not
 *   integers here.
 *
 * The domain (`0..=256`) is deliberately **not** checked: an out-of-domain
 * integer is the host's `color.invalid_value` answer, not a local "unsupported",
 * so the line must reach the router. No `Number()` conversion is used either — a
 * digit run too long for a JS number is still the integer shape the host parses
 * (and refuses by domain), so it travels instead of becoming a shape refusal.
 */
export function isColorArgument(argument: string): boolean {
  const text = argument.trim();
  if (COLOR_SYMBOLS.includes(text.toUpperCase())) return true;
  return /^[+-]?[0-9]+$/.test(text);
}
