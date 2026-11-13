// The plugin `Runtime` namespace: command registration.
//
// Registration is **client-local**. A plugin names a command and hands over a
// handler; this module validates the name, keeps the registration, and reports it
// to the host hook `createCadApi` was given (`onCommandRegister`). No wire request
// is involved: the host's command funnel is the host's, and a plugin command is
// additive to it (the descriptor has no `runtime.registerCommand` row for exactly
// that reason).
//
// When the host supplies **no** hook, registration refuses with
// `unsupported-runtime: cad_api.Runtime.registerCommand`: the carrier has not wired
// a funnel, so accepting the command would promise a dispatch that can never
// happen. That is the one refusal this namespace produces that is not a caller
// error.
//
// `flags` are recorded, not interpreted: the only dispatch path in this build is a
// typed command line, so modal/transparent/pick-set semantics have nothing to hook
// into yet. The set is closed so an unknown flag is a refusal, not a silent no-op.

import { CadApiError, invalidArgument } from './errors.ts';
import type { PluginContext } from './context.ts';

/**
 * How a plugin command may participate in dispatch.
 *
 * Recorded on the registration and not acted on by this build (see the header);
 * the values exist because AutoCAD-style registration names them and because a
 * closed set is what lets an unknown flag be refused.
 */
export const CommandFlags = {
  /** A normal command: runs to completion on the typed command line. */
  Modal: 'modal',
  /** May run while another command is active (a carrier may still refuse it). */
  Transparent: 'transparent',
  /** Receives the current pick set when it is invoked from a selection. */
  UsePickSet: 'usePickSet',
} as const;

/** `'modal' | 'transparent' | 'usePickSet'`. */
export type CommandFlags = (typeof CommandFlags)[keyof typeof CommandFlags];

const COMMAND_FLAGS: readonly string[] = [
  CommandFlags.Modal,
  CommandFlags.Transparent,
  CommandFlags.UsePickSet,
];

/** A plugin command's handler: it may be synchronous or asynchronous. */
export type CommandHandler = (ctx: PluginContext) => void | Promise<void>;

/** One accepted registration, exactly as the plugin declared it. */
export interface CommandRegistration {
  /** The command name (`[A-Z][A-Z0-9_]{0,31}`). */
  readonly name: string;
  /** The declared flag (recorded, not interpreted). */
  readonly flags: CommandFlags;
  /** The plugin's handler; the host calls it with the plugin's own context. */
  readonly handler: CommandHandler;
}

/** The `Runtime` namespace exposed on `CadApi.runtime`. */
export interface RuntimeNamespace {
  /**
   * Register one command with the host's funnel.
   *
   * Refuses a malformed name (`api.invalid_argument: command`), a flag outside the
   * closed set (`api.invalid_argument: flags`), a second registration of the same
   * name (`api.invalid_argument: command.duplicate`) and — when the host supplied
   * no hook — the whole operation (`unsupported-runtime: cad_api.Runtime.registerCommand`).
   */
  registerCommand(name: string, flags: CommandFlags, handler: CommandHandler): CommandRegistration;
  /**
   * Remove a registration.
   *
   * A name that was never registered is `api.invalid_argument: command.unknown`; a
   * malformed name is `api.invalid_argument: command`.
   */
  unregisterCommand(name: string): void;
}

/** The host hooks a runtime namespace reports to; both are optional. */
export interface RuntimeHooks {
  readonly onCommandRegister?: (registration: CommandRegistration) => void;
  readonly onCommandUnregister?: (name: string) => void;
}

/** `[A-Z][A-Z0-9_]{0,31}` — the descriptor's command grammar. */
const COMMAND_NAME = /^[A-Z][A-Z0-9_]{0,31}$/;

function isCommandFlag(value: string): value is CommandFlags {
  return COMMAND_FLAGS.some((flag) => flag === value);
}

/**
 * Build the runtime namespace for one client.
 *
 * Registrations live for the lifetime of this namespace (one `createCadApi`
 * call): a plugin that is deactivated calls `unregisterCommand`, and there is no
 * cross-session table to leak into.
 */
export function createRuntimeNamespace(hooks: RuntimeHooks): RuntimeNamespace {
  const registrations = new Map<string, CommandRegistration>();

  return {
    registerCommand(name, flags, handler): CommandRegistration {
      if (typeof name !== 'string' || !COMMAND_NAME.test(name)) throw invalidArgument('command');
      if (typeof flags !== 'string' || !isCommandFlag(flags)) throw invalidArgument('flags');
      if (typeof handler !== 'function') throw invalidArgument('handler');
      if (hooks.onCommandRegister === undefined) {
        // The host did not wire a funnel: accepting the command would promise a
        // dispatch that can never happen.
        throw new CadApiError(
          'unsupported-runtime: cad_api.Runtime.registerCommand',
          'this host has no plugin command funnel',
        );
      }
      if (registrations.has(name)) throw invalidArgument('command.duplicate');
      const registration: CommandRegistration = Object.freeze({ name, flags, handler });
      registrations.set(name, registration);
      hooks.onCommandRegister(registration);
      return registration;
    },
    unregisterCommand(name): void {
      if (typeof name !== 'string' || !COMMAND_NAME.test(name)) throw invalidArgument('command');
      if (!registrations.has(name)) throw invalidArgument('command.unknown');
      registrations.delete(name);
      hooks.onCommandUnregister?.(name);
    },
  };
}
