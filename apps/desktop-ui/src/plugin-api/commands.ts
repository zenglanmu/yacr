// The plugin command registry: the one table the typed-command funnel consults.
//
// A plugin command is **additive** to the host's vocabulary. The router owns the
// host commands; this table only ever holds names a plugin registered through
// `api.runtime.registerCommand`, and the funnel asks it before the host router.
// A plugin can therefore shadow a host name, but only by declaring that exact name
// and having its registration accepted — which is the deliberate, reviewable form
// of shadowing, not an accident.
//
// Failure is isolated: `runPluginCommand` reports a thrown handler through the
// shared notice (`plugin.command.failed`) and never rethrows. A plugin bug must
// not break the command funnel for every other command.
//
// There is no wire here. Registration is client-local; the host's funnel hook is
// installed by `loader.ts` through `createCadApi`'s options.

import { showPluginNotice } from '../plugins/host.ts';

/** One registered plugin command, as the loader installed it. */
export interface PluginCommand {
  readonly name: string;
  /** The declared flag; recorded, not interpreted by the funnel. */
  readonly flags: string;
  /** Run the plugin's handler. */
  readonly run: () => Promise<void>;
}

/** What `pluginCommandFor` reports: the command and the function that runs it. */
export interface PluginCommandMatch {
  readonly name: string;
  readonly run: () => Promise<void>;
}

interface Entry {
  readonly command: PluginCommand;
  readonly registeredAt: number;
}

/**
 * A re-registration this soon after the first is a duplicate load, not a reload.
 *
 * `registerPluginCommand` keeps the first registration inside the window (a double
 * `activate` must not silently flip what a command does mid-session) and replaces
 * it after that (an intentional reload).
 */
const REPLACE_AFTER_MS = 5000;

const commands = new Map<string, Entry>();

/** Register one command; see [`REPLACE_AFTER_MS`] for the re-registration rule. */
export function registerPluginCommand(command: PluginCommand): void {
  const existing = commands.get(command.name);
  const now = Date.now();
  if (existing !== undefined && now - existing.registeredAt < REPLACE_AFTER_MS) return;
  commands.set(command.name, { command: Object.freeze({ ...command }), registeredAt: now });
}

/** Remove one command; removing a name that is not registered is a no-op. */
export function unregisterPluginCommand(name: string): void {
  commands.delete(name);
}

/**
 * The plugin command a typed line names, or `null`.
 *
 * The line is trimmed of leading spaces and matched **exactly** and
 * case-insensitively against the registered names — this build passes no
 * arguments, so a line with anything after the command does not match. Nothing
 * here consults the host's vocabulary: a host command only matches when a plugin
 * registered that same name.
 */
export function pluginCommandFor(text: string): PluginCommandMatch | null {
  const trimmed = text.replace(/^\s+/u, '');
  if (trimmed.length === 0) return null;
  const entry = commands.get(trimmed.toUpperCase());
  if (entry === undefined) return null;
  return { name: entry.command.name, run: entry.command.run };
}

/**
 * Run one plugin command, isolating its failure.
 *
 * A thrown handler (sync or async) is logged and surfaced as the shared
 * `plugin.command.failed` notice — with the command name as `{command}` — and this
 * function never rethrows, so the funnel stays usable.
 */
export async function runPluginCommand(run: () => Promise<void>, name: string): Promise<void> {
  try {
    await run();
  } catch (failure) {
    console.error(`plugin command ${name} failed`, failure);
    showPluginNotice('plugin.command.failed', { command: name });
  }
}

/** Test seam: forget every registration (the real app registers once per load). */
export function resetPluginCommands(): void {
  commands.clear();
}
