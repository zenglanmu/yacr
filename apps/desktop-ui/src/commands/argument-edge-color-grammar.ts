// Local syntax gate for `EDGECOLOR [DEFAULT|<r> <g> <b>]`.
//
// The host owns the session state; this only keeps a typed or tray-dispatched
// line from being refused before IPC.

/** Whether `argument` is one admitted `EDGECOLOR` argument. */
export function isEdgeColorArgument(argument: string): boolean {
  const tokens = argument.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 1) return tokens[0].toUpperCase() === 'DEFAULT';
  return tokens.length === 3
    && tokens.every(token => /^\d{1,3}$/.test(token) && Number(token) <= 255);
}
