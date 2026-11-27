// Local syntax gate for `SECTIONPLANE <argument>`.
//
// Mirrors cad-ui-core's `parse_section_argument` so the command line and the View
// menu can dispatch the line before IPC. The host owns normalization/validation.

function isFiniteNumber(token: string): boolean {
  if (token === '') return false;
  return Number.isFinite(Number(token));
}

/** Whether `argument` is one admitted `SECTIONPLANE` argument. */
export function isSectionPlaneArgument(argument: string): boolean {
  const tokens = argument.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 1) {
    return ['ON', 'OFF', 'TOGGLE', 'CLEAR', 'LIST'].includes(tokens[0].toUpperCase());
  }
  return tokens.length === 7
    && tokens[0].toUpperCase() === 'SET'
    && tokens.slice(1).every(isFiniteNumber);
}
