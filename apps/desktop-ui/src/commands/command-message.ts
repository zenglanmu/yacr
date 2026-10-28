// The one builder for a refused command's sentence.
//
// It is built here, once, because the history row (`ShellValue.historyEntries`)
// is the **only** place that sentence is shown: the red line under the field is
// gone, so a second builder would be a second copy of the same fact. Every
// refusal path — a command refused by admission, a host-side failure, a text
// step `cad_set_tool_text` refused — reads its words from this one call.
//
// The rule is unchanged from the App-level memo this was extracted from: the
// refusal sentence is the catalog key the admission itself reported (`reasonKey`),
// so a blocked command reads the same in the command line as in the disabled
// button that offers it — the busy sentence, "no drawing is open", the gesture
// hint, or "not supported yet".
import type { CadCommandResult } from '../cad/cad-command.ts';
import type { Translator } from '../i18n.ts';
import { unavailableReasonKey } from './commands.ts';

/** One refusal sentence (an admitted result reports nothing here). */
export interface CommandMessage {
  readonly kind: string;
  readonly text: string;
}

/**
 * The refusal sentence for `result`, or `null` when there is nothing to report.
 *
 * An accepted (or chrome-local) result produces no sentence: the host's next
 * status is the confirmation, and a fabricated "done" would claim completion the
 * queue admission never promised — and an empty row in the log.
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
