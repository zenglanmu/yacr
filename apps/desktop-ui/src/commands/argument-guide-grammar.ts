// The local syntax gate for `GUIDE <argument>`.
//
// This mirrors the host's `parse_guide_argument` (`cad-ui-core`, ADR-0084/0085/
// ADR-0101) so a typed or panel-dispatched line is not refused before IPC. The
// **host remains the authority** for semantics: this only says whether the
// tokens are one of the admitted shapes.

/** A finite `f64` in the same spelling Rust's `f64::from_str` accepts. */
function isFiniteNumber(token: string): boolean {
  if (token === '') return false;
  const value = Number(token);
  return Number.isFinite(value);
}

/** A `u8` decimal (no sign, no fraction, 0..=255). */
function isChannel(token: string): boolean {
  if (!/^\d{1,3}$/.test(token)) return false;
  return Number(token) <= 255;
}

/** A positive `usize` decimal (the 1-based row index). */
function isRowIndex(token: string): boolean {
  return /^\d+$/.test(token) && Number(token) > 0 && Number.isSafeInteger(Number(token));
}

/** Whether `argument` is one admitted `GUIDE` argument. */
export function isGuideArgument(argument: string): boolean {
  const tokens = argument.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 1) {
    const word = tokens[0].toUpperCase();
    return word === 'LIST' || word === 'CLEAR';
  }
  if (tokens.length === 2) {
    return tokens[0].toUpperCase() === 'DELETE' && isRowIndex(tokens[1]);
  }
  const head = tokens[0]?.toUpperCase() ?? '';
  if (head === 'POINT' && tokens.length === 4) {
    return tokens.slice(1).every(isFiniteNumber);
  }
  if (head !== 'STYLE' || tokens.length < 3) return false;
  const kind = tokens[1].toUpperCase();
  if (kind === 'COLOR' && tokens.length === 5) {
    return tokens.slice(2).every(isChannel);
  }
  if (kind === 'DASH' && tokens.length === 3) {
    const value = tokens[2].toUpperCase();
    return value === 'ON' || value === 'OFF' || tokens[2] === '1' || tokens[2] === '0';
  }
  return false;
}
