// The plugin module loader: import, validate, assemble, activate, undo.
//
// Loading is the host's job (the package deliberately never imports a module). One
// call does the whole lifecycle up to a running plugin:
//
//   1. resolve the module — a URL through `deps.importer` (default: dynamic
//      `import`) or an already-loaded namespace object (the tests' path);
//   2. validate its default export with the package's `definePlugin`, refusing an
//      incompatible `apiVersion` as `api.host-outdated: <version>` **before** the
//      client is built;
//   3. `openPluginApi` with the command hooks installed, so
//      `ctx.api.runtime.registerCommand` lands in `commands.ts`;
//   4. call `activate(ctx)`.
//
// Failure leaves **nothing** behind: an import or activate failure unregisters
// every command this plugin registered before the error escapes as
// `api.plugin.load_failed: <detail>`. `deactivate()` is idempotent, removes the
// commands first, and calls the plugin's own `deactivate` once.
//
// The plugin is a same-privilege ES module (ADR 0029/0030/0034): this loader is
// interop glue, not a sandbox.

import { API_VERSION, CadApiError, definePlugin } from '../../../../packages/cad-plugin-api/src/index.ts';
import type {
  CadPlugin,
  CommandRegistration,
  PluginContext,
} from '../../../../packages/cad-plugin-api/src/index.ts';
import { registerPluginCommand, unregisterPluginCommand } from './commands.ts';
import { openPluginApi } from './session.ts';
import type { PluginApiDeps } from './session.ts';

/** A loaded, activated plugin and the one call that retires it. */
export interface ActivePlugin {
  readonly plugin: CadPlugin;
  readonly context: PluginContext;
  /** Unregister its commands, then call the plugin's `deactivate` once. */
  deactivate(): Promise<void>;
}

/** What loading needs: the bridge deps plus the module importer. */
export interface LoadPluginDeps extends PluginApiDeps {
  /** Resolve a module URL; defaults to a dynamic `import(url)`. */
  readonly importer?: (url: string) => Promise<unknown>;
}

/** Where the plugin module comes from: a URL, or an already-loaded namespace. */
export type PluginSource = { readonly url: string } | { readonly module: unknown };

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The one stable shape a load failure takes; `detail` names the step. */
function loadFailed(detail: string, cause?: unknown): CadApiError {
  return new CadApiError(
    `api.plugin.load_failed: ${detail}`,
    `plugin load failed: ${detail}`,
    cause === undefined ? undefined : { cause },
  );
}

async function importModule(source: PluginSource, deps: LoadPluginDeps): Promise<unknown> {
  if ('module' in source) return source.module;
  const importer = deps.importer ?? ((url: string) => import(/* @vite-ignore */ url));
  try {
    return await importer(source.url);
  } catch (cause) {
    throw loadFailed('import', cause);
  }
}

/**
 * Load and activate one plugin.
 *
 * Rejects with `api.plugin.load_failed: <detail>` for a failed import, an
 * unusable default export, or a rejected `activate`; an incompatible contract
 * generation rejects as `api.host-outdated: <version>` before anything is wired.
 * On every rejection path the commands registered so far are gone.
 */
export async function loadPluginModule(
  source: PluginSource,
  deps: LoadPluginDeps,
): Promise<ActivePlugin> {
  const module = await importModule(source, deps);

  const candidate = isRecord(module) ? module['default'] : undefined;
  if (!isRecord(candidate)) throw loadFailed('module');
  const declared = candidate['apiVersion'];
  if (typeof declared === 'number' && declared !== API_VERSION) {
    throw new CadApiError(
      `api.host-outdated: ${declared}`,
      `the plugin targets cad api ${declared}, this host serves ${API_VERSION}`,
    );
  }

  let plugin: CadPlugin;
  try {
    plugin = definePlugin(candidate as unknown as CadPlugin);
  } catch (cause) {
    throw loadFailed('module', cause);
  }

  // Commands are registered from inside `activate`, which runs after the context
  // exists; the hook refuses before that rather than installing a run that has no
  // plugin to run against.
  const names = new Set<string>();
  let context: PluginContext | null = null;

  const session = await openPluginApi({
    invoke: deps.invoke,
    status: deps.status,
    pollDelayMs: deps.pollDelayMs,
    maxPolls: deps.maxPolls,
    onCommandRegister: (registration: CommandRegistration): void => {
      const active = context;
      if (active === null) throw loadFailed('command');
      names.add(registration.name);
      registerPluginCommand({
        name: registration.name,
        flags: registration.flags,
        run: async (): Promise<void> => {
          await registration.handler(active);
        },
      });
    },
    onCommandUnregister: (name: string): void => {
      names.delete(name);
      unregisterPluginCommand(name);
    },
  });

  const built: PluginContext = {
    api: session.api,
    host: session.host,
    pluginId: plugin.id,
  };
  context = built;

  try {
    if (plugin.activate !== undefined) await plugin.activate(built);
  } catch (cause) {
    for (const name of [...names]) unregisterPluginCommand(name);
    names.clear();
    throw loadFailed('activate', cause);
  }

  let deactivated = false;
  return {
    plugin,
    context: built,
    async deactivate(): Promise<void> {
      if (deactivated) return;
      deactivated = true;
      // Commands go first: a dispatch that raced the unload must not reach a
      // plugin that is being torn down.
      for (const name of [...names]) unregisterPluginCommand(name);
      names.clear();
      if (plugin.deactivate !== undefined) await plugin.deactivate(built);
    },
  };
}
