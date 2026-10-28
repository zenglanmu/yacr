// The one builder for the command line's message line.
//
// The sentence under the field is built here, once, so its two readers can never
// disagree: the line the user reads (`ShellValue.commandMessage`) and the entry
// the history log records (`historyEntries`) are the *same string* from the
// *same call*. A second builder would let the log word a refusal differently from
// the line that was on screen when it happened — one catalog edit, two sentences.
//
// The rule is unchanged from the App-level memo this was extracted from: the
// refusal sentence is the catalog key the admission itself reported (`reasonKey`),
// so a blocked command reads the same in the command line as in the disabled
// button that offers it — the busy sentence, "no drawing is open", the gesture
// hint, or "not supported yet".
import type { CadCommandResult } from '../cad/cad-command.ts';
import type { Translator } from '../i18n.ts';
import { unavailableReasonKey } from './commands.ts';

/** One line of the command line's message line (only refusals are reported). */
export interface CommandMessage {
  readonly kind: string;
  readonly text: string;
}

/**
 * The message line for `result`, or `null` when there is nothing to report.
 *
 * An accepted (or chrome-local) result produces no line: the host's next status
 * is the confirmation, and a fabricated "done" would claim completion the queue
 * admission never promised.
 */
export function commandMessageFor(result: CadCommandResult | null, t: Translator): CommandMessage | null {
  if (!result) return null;
  if (result.kind === 'unsupported') {
    return {
      kind: 'error',
      text: t(result.reasonKey ?? unavailableReasonKey(result.command), { command: result.command }),
    };
  }
  if (result.kind === 'failed') {
    if (result.reasonKey) return { kind: 'error', text: t(result.reasonKey, { command: result.command }) };
    return { kind: 'error', text: t('command.failed', { error: result.error }) };
  }
  return null;
}
