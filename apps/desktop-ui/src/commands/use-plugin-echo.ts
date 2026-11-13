// The command line's subscription to the plugin echo (`plugin-message-echo.ts`).
//
// Kept in its own file so the echo model stays React-free (and testable with
// plain Node): the hook only installs one subscription for the mounted
// presentation and hands back the bounded snapshot. The snapshot lives in the
// model module, so a remount (closing and reopening the command line) neither
// loses the last lines nor installs a second lane.
import { useEffect, useSyncExternalStore } from 'react';
import { pluginEchoSnapshot, startPluginEcho, subscribePluginEcho } from './plugin-message-echo.ts';

/** The last plugin `writeMessage` lines, oldest first (at most three). */
export function usePluginEcho(): readonly string[] {
  const lines = useSyncExternalStore(subscribePluginEcho, pluginEchoSnapshot, pluginEchoSnapshot);
  useEffect(() => startPluginEcho(), []);
  return lines;
}
