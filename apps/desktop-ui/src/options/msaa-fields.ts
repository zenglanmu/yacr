// The anti-aliasing (MSAA) row of the Options control table.
//
// Split out of `options.ts` so both files stay inside the file-size gate. The
// row is an ordinary `select`: the host reports the effective level
// (`msaaMode`) and accepts exactly the same three tokens on
// `cad_set_setting`. Whether the device can really render a level is the
// renderer's answer — a level it cannot support is downgraded by the host and
// reported in diagnostics, so this row always offers (and writes) all three
// tokens and carries no carrier gate of its own.
import type { OptionField } from './unit-fields.ts';

/**
 * The MSAA levels the host accepts (`"off" | "2x" | "4x"`).
 *
 * A closed token set in the fixed display order; the host's default is `4x` and
 * it reports that itself — the list order is not the default. `off` is a real
 * choice (cheaper VRAM/GPU cost), never the "not reported" state: an absent
 * snapshot field stays `null` in the draft.
 */
export const MSAA_MODES: readonly string[] = Object.freeze(['off', '2x', '4x']);

/** The `display` tab's MSAA row, spliced in right after `gpuBudgetPercent`. */
export const MSAA_FIELDS: readonly OptionField[] = Object.freeze<OptionField[]>([
  {
    id: 'msaaMode',
    tab: 'display',
    kind: 'select',
    wire: 'msaaMode',
    key: 'msaaMode',
    choices: MSAA_MODES,
    choiceKeyPrefix: 'options.msaaMode',
    labelKey: 'options.field.msaaMode',
    helpKey: 'options.help.msaaMode',
  },
]);
