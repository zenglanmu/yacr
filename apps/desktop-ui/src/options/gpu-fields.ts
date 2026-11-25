// The GPU storage / memory rows of the Options control table.
//
// Split out of `options.ts` so both files stay inside the file-size gate. The
// rows, their dependency rule and the visibility helper live here; the table
// itself (`OPTIONS_FIELDS`) splices `GPU_FIELDS` in at the `display` position
// before MSAA, so the rendering order is still declared in exactly one place.

import type { OptionField } from './unit-fields.ts';

/**
 * The switch a row depends on, or absent for an always-rendered row.
 *
 * `gpuBudgetPercent` sizes the accelerated GPU-resident copy, so it is only
 * meaningful while that copy is enabled (`gpuStorageDisabled` false). The
 * dependency lives with the rows instead of in the dialog so the renderer, the
 * diff and the validator all read the same rule.
 */
const FIELD_DEPENDENCIES: Readonly<Record<string, string>> = Object.freeze({
  gpuBudgetPercent: 'gpuStorageDisabled',
});

/**
 * Whether one row is shown for the current draft.
 *
 * A hidden row is not rendered, not diffed and not validated: it describes an
 * inapplicable value, and staging a write nobody can see would be a silent
 * change. An unreported dependency (`null`) keeps the row visible, because the
 * host never said the feature was off.
 */
export function fieldVisible(
  field: OptionField,
  draft: Readonly<Record<string, unknown>> | null | undefined,
): boolean {
  const dependency = FIELD_DEPENDENCIES[field.id];
  if (!dependency) return true;
  return draft?.[dependency] !== true;
}

/** The `display` tab's GPU rows, in rendering order (before MSAA). */
export const GPU_FIELDS: readonly OptionField[] = Object.freeze<OptionField[]>([
  {
    // The accelerated GPU-resident copy of the convertible scene (the "pulled"
    // organisation: one draw per class from vertex-stage storage buffers). On by
    // default (`disabled: true`): no second GPU copy is allocated and the
    // universal per-run path draws the same pixels. Turning it off lets a capable
    // device build the accelerated copy for large drawings.
    //
    // The draw tier is chosen once per device, so the host applies a change at
    // the next device attach; the help sentence says so rather than pretending a
    // live switch. Wire field and write key are the same camelCase spelling.
    id: 'gpuStorageDisabled',
    tab: 'display',
    kind: 'bool',
    wire: 'gpuStorageDisabled',
    key: 'gpuStorageDisabled',
    labelKey: 'options.field.gpuStorageDisabled',
    helpKey: 'options.help.gpuStorageDisabled',
  },
  {
    // The share of the graphics card's memory CAD may use. The renderer lane
    // reads this persisted value; neither the wire field, the write key nor the
    // config leaf is renamed here, so the three spellings cannot drift apart.
    // The row exists only while GPU storage is enabled (see `fieldVisible`):
    // with the accelerated copy disabled there is no pulled allocation to size.
    id: 'gpuBudgetPercent',
    tab: 'display',
    kind: 'number',
    wire: 'gpuBudgetPercent',
    key: 'gpuBudgetPercent',
    min: 10,
    max: 80,
    unitKey: 'options.unit.percent',
    labelKey: 'options.field.gpuBudgetPercent',
    helpKey: 'options.help.gpuBudgetPercent',
  },
]);
