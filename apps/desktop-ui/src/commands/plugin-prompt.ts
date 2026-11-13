// While a plugin's `Editor` prompt is pending, the command line is **answer
// data**: the typed line completes the prompt, it is not a CAD command.
//
// The host reports the prompt on `cad_status.pendingPrompt`
// (`cad/cad-status.ts`) and resolves the answer from the ordinary interaction
// lane, so the chrome owns exactly two decisions:
//
// * **where a submitted line goes.** A non-empty answer travels on the existing
//   text channel (`cad_set_tool_text`, `tools/text.ts`) — never `cad_command`,
//   and never through `admitCommandText`/`queueCadCommand`, which would judge an
//   answer against the command vocabulary. An empty answer is Enter with nothing
//   typed, which the host reads as `CONFIRM` (its prompt default, or `None`).
//   Escape is `CANCEL` and is mapped by the shell's existing key table
//   (`toolCommandForKey`), not here.
// * **that no host-command affordance competes with it.** The command-line model
//   takes the controller's `promptMode` boolean (the block is not null), so the
//   suggestion list and the command rows are off for the line
//   (`command-line-model.ts`).
//
// Nothing here invents an answer: the text is submitted exactly as typed
// (whitespace-only counts as the empty line), and every refusal the text channel
// reports is handed back unchanged for the one refusal surface (the history
// row). A prompt that is not pending yields `ignored`, so the caller can keep
// using its normal command lane without a second "is a prompt open?" check.

import type { CadPendingPrompt } from '../cad/cad-status.ts';
import type { ToolTextAnswerResult } from '../tools/text.ts';

/**
 * The catalog key of the field's placeholder while a prompt is pending.
 *
 * The question itself is the chip beside the field (`prompt.message`, the
 * plugin's own sentence and never a catalog string); the placeholder only says
 * how to answer, which is the one part the chrome can word.
 */
export const PROMPT_ANSWER_PLACEHOLDER_KEY = 'command.plugin_prompt.placeholder';

/** The catalog key naming the keyword chip group (its accessible label). */
export const PROMPT_KEYWORDS_LABEL_KEY = 'command.plugin_prompt.keywords';

/** Where one submitted command-line draft belongs while a prompt is pending. */
export type PluginPromptIntent =
  | { readonly kind: 'command' }
  | { readonly kind: 'confirm' }
  | { readonly kind: 'answer'; readonly text: string };

/**
 * Route one command-line draft against the pending prompt.
 *
 * * no prompt → `command` (the ordinary command funnel is unchanged);
 * * an empty/whitespace-only line → `confirm` (Enter with no answer);
 * * anything else → `answer` with the text **exactly as typed**: the host
 *   matches keywords and parses numbers itself, in its own spelling rules, so a
 *   chrome-side trim or rewrite could accept input the host would refuse (or the
 *   reverse for `getString`, which keeps its spaces).
 */
export function pluginPromptIntent(
  prompt: CadPendingPrompt | null | undefined,
  line: unknown,
): PluginPromptIntent {
  if (prompt === null || prompt === undefined) return { kind: 'command' };
  if (typeof line !== 'string' || line.trim().length === 0) return { kind: 'confirm' };
  return { kind: 'answer', text: line };
}

/**
 * What one prompt answer did, in the command line's own vocabulary.
 *
 * `ignored` means "no prompt was pending", which the caller answers by running
 * the line as a command. Everything else is the lane's real result: `accepted`
 * is dispatch admission (the prompt itself clears on a later status), `busy` a
 * re-entrant submit, and a refusal/failure carries its error so the caller can
 * name it.
 */
export type PluginPromptOutcome = ToolTextAnswerResult | { readonly kind: 'ignored' };

/** The two lanes one answer may take, injected so this module owns no IPC. */
export interface PluginPromptDeps {
  /** Answer Enter: the ordinary `CONFIRM` selector through the shell funnel. */
  readonly confirm: () => void | Promise<void>;
  /** Answer typed text: the one `cad_set_tool_text` lane. */
  readonly answer: (text: string) => Promise<ToolTextAnswerResult>;
}

/**
 * Submit one prompt answer on the lane its intent names.
 *
 * This is deliberately the **only** place a pending-plugin line is dispatched,
 * and it can reach no command funnel: `confirm` is the shell's own submit of the
 * `CONFIRM` selector, and `answer` is `setToolText`, whose single IPC is
 * `cad_set_tool_text`. A dispatched `CONFIRM` reports its own outcome through the
 * shell (banner/history) — this module does not claim to have observed it.
 */
export async function submitPluginPrompt(
  prompt: CadPendingPrompt | null | undefined,
  line: unknown,
  deps: PluginPromptDeps,
): Promise<PluginPromptOutcome> {
  const intent = pluginPromptIntent(prompt, line);
  if (intent.kind === 'command') return { kind: 'ignored' };
  if (intent.kind === 'confirm') {
    // `confirm` reports through the shell's banner, exactly like any other
    // refused `CONFIRM`; the answer lane has no second error surface.
    await deps.confirm();
    return { kind: 'accepted' };
  }
  return deps.answer(intent.text);
}
