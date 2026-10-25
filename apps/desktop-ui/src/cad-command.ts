// The command funnel: admission of one typed line, the Enter funnel and the
// `cad_status` poll. Split out of `cad.ts` (file-size ceiling); `cad.ts`
// re-exports every symbol here so the chrome's import surface is unchanged.

import { admitCommandText, isLocalChromeCommand } from './commands.ts';
import { parseCadStatus } from './cad-status.ts';
import type { CadStatus, Invoke } from './cad-status.ts';

/**
 * Whether the host capability gates allow `command` right now.
 *
 * A thin delegation to [`admitCommandText`] — host vocabulary, pending/loading
 * state and every capability boolean (`canOpen`, `canNew`, `canSave`,
 * `canPlot`, `canFit`) — so there is exactly one admission rule. The chrome's
 * buttons use the shell's `canCommand`, which is this same call.
 */
export function commandEnabled(
  status: Partial<CadStatus> | null | undefined,
  pending: boolean,
  command: string,
): boolean {
  return admitCommandText(command, { status, pending }).kind === 'admitted';
}

/** What the `cad_command` funnel did with one line (admission, never completion). */
export type CadCommandResult =
  | { readonly kind: 'accepted'; readonly command: string }
  | { readonly kind: 'local'; readonly command: string }
  | { readonly kind: 'unsupported'; readonly command: string; readonly reasonKey: string | null }
  | { readonly kind: 'failed'; readonly command?: string; readonly error: string; readonly reasonKey?: string | null };

/**
 * Admit `text` through `cad_command`.
 *
 * The command vocabulary is the one loaded from the host (`admitCommandText`),
 * so an interactive tool (`LINE`, `MOVE`, `CONFIRM`, …) reaches the router; only
 * a name the host does not know, a gesture-only name, a document-scoped selector
 * without a drawing, or a false host capability (`canNew`/`canSave`/`canPlot`/
 * `canOpen`/`canFit`) is refused before IPC. The refusal carries the catalog
 * `reasonKey` so the command line shows the real sentence. `{kind:'accepted'}`
 * is queue admission, never completion; a host-side refusal is an explicit
 * `failed`, never a silent success.
 */
export async function queueCadCommand(
  invoke: Invoke,
  text: string,
  status: Partial<CadStatus> | null | undefined,
  pending: boolean,
): Promise<CadCommandResult> {
  const admission = admitCommandText(text, { status, pending });
  const command = admission.command;
  if (admission.kind !== 'admitted') {
    return admission.kind === 'unavailable'
      ? { kind: 'failed', error: 'CAD_COMMAND_UNAVAILABLE', command, reasonKey: admission.reasonKey }
      : { kind: 'unsupported', command, reasonKey: admission.reasonKey };
  }
  // A chrome-local selector has no router command. Returning it as `local`
  // keeps this funnel honest instead of sending a name the router would refuse
  // (the command line intercepts it before it ever gets here).
  if (isLocalChromeCommand(command)) return { kind: 'local', command };
  try {
    const response = await invoke('cad_command', { command });
    if (!response || (response as { accepted?: unknown }).accepted !== true) throw Error('CAD_COMMAND_NOT_ACCEPTED');
    // Acceptance is only queue admission, never completion or a database update.
    return { kind: 'accepted', command };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * The keyboard fields the Enter funnel reads.
 *
 * Structural on purpose: the shell hands it a React synthetic keyboard event,
 * whose own type does not declare the native IME flag, while the two flags are
 * read defensively anyway (an absent flag means "not composing"/"no legacy 229").
 */
export interface CommandKeyEvent {
  readonly key: string;
  readonly isComposing?: boolean;
  readonly keyCode?: number;
  readonly repeat?: boolean;
  preventDefault(): void;
}

export function handleCommandKey(event: CommandKeyEvent, submit: () => void): void {
  // `isComposing` covers the standard IME report; `keyCode === 229` is the
  // legacy signal some IMEs send for the Enter that ends/commits composition.
  // Either way the press belongs to the IME, so Enter must not submit — for a
  // command *and* for a tool's text step.
  if (event.key !== 'Enter' || event.isComposing || event.keyCode === 229 || event.repeat) return;
  event.preventDefault();
  void submit();
}

/** A timer handle of whichever clock drives the poll (browser or a test double). */
type TimerHandle = ReturnType<typeof setTimeout>;

/** The clock the poll schedules on (the global one, or a test double). */
export interface PollClock {
  setTimeout(callback: () => void, delay: number): TimerHandle;
  clearTimeout(timer?: TimerHandle): void;
}

export function startCadPolling(
  invoke: Invoke,
  onStatus: (status: CadStatus | null) => void,
  onError: (error: string) => void,
  clock: PollClock = globalThis,
): () => void {
  let stopped = false;
  let timer: TimerHandle | undefined;
  let failures = 0;
  async function poll(): Promise<void> {
    try {
      const status = parseCadStatus(await invoke('cad_status'));
      if (!stopped) { failures = 0; onStatus(status); }
    } catch (error) {
      if (!stopped) { failures++; onStatus(null); onError(String(error)); }
    }
    if (!stopped) timer = clock.setTimeout(poll, failures ? Math.min(4000, 500 * 2 ** Math.min(failures, 3)) : 250);
  }
  void poll();
  return () => { stopped = true; clock.clearTimeout(timer); };
}
