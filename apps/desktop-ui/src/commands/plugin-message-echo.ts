// The plugin message echo: the last lines a plugin wrote through
// `editor.writeMessage`, shown in the command line's echo area.
//
// # The lane
//
// `editor.writeMessage` is client-local (it has no descriptor row and no wire
// call), so `plugin-api/messages.ts` is its store: the plugin client appends
// through it (`plugin-api/session.ts` wires `appendPluginMessage` as the sink) and
// this module is one reader of its bounded buffer. Nothing here creates a message
// and nothing here re-words one.
//
// # What this module adds
//
//  * **A tighter bound for the command line.** The store keeps eight lines; the
//    echo shows the newest three, because the strip floats over the drawing and
//    must stay a glance, not a log (the full record is the history popup / sheet).
//  * **Validation, not coercion.** Only non-blank strings are echoes: a number,
//    an object or an empty line is dropped rather than stringified into copy the
//    plugin never wrote.
//  * **A snapshot for React.** The store is mirrored into a module-level list so
//    the command line reads the same value through `useSyncExternalStore`, and a
//    4 Hz status poll cannot re-render it for an unchanged echo. The snapshot
//    outlives a remount (closing and reopening the command line keeps the lines,
//    exactly as the command record does).
import { pluginMessages, subscribePluginMessages } from '../plugin-api/messages.ts';

/** The most plugin lines the echo keeps. Bounded so the strip stays one glance. */
export const PLUGIN_ECHO_MAX_LINES = 3;

/** The bounded echo snapshot (oldest first) every presentation reads. */
let echo: readonly string[] = [];

const listeners = new Set<() => void>();

/**
 * The echo lines from whatever the message store reported.
 *
 * The list keeps the **newest** entries (the store is oldest-first), capped at
 * `limit`; `limit <= 0` is an empty echo rather than "unbounded".
 */
export function pluginEchoLines(messages: unknown, limit: number = PLUGIN_ECHO_MAX_LINES): string[] {
  if (!Array.isArray(messages) || limit <= 0) return [];
  const lines = messages.filter(
    (message: unknown): message is string => typeof message === 'string' && message.trim().length > 0,
  );
  return lines.slice(-limit);
}

/** The current bounded echo (oldest first); empty until a plugin writes. */
export function pluginEchoSnapshot(): readonly string[] {
  return echo;
}

/** Subscribe to echo changes; returns the unsubscribe. */
export function subscribePluginEcho(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Publish one reported list, notifying only when the bounded echo changed. */
function publish(messages: unknown): void {
  const next = pluginEchoLines(messages);
  if (next.length === echo.length && next.every((line, index) => line === echo[index])) return;
  echo = next;
  for (const listener of listeners) listener();
}

/**
 * Install the echo for one command-line mount and return its disposer.
 *
 * The first read is immediate (a plugin that already wrote something shows it the
 * moment the command line mounts) and the subscription re-reads on every store
 * change. One mounted presentation at a time installs this; the snapshot itself
 * lives here, so a remount neither loses the last lines nor duplicates a lane.
 */
export function startPluginEcho(): () => void {
  publish(pluginMessages());
  return subscribePluginMessages(messages => { publish(messages); });
}
