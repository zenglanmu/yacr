// The ribbon's pressed-state contract: which `cad_status` flag a toggle reads.
//
// Kept in a sibling module (not a barrel) so `ribbon-model.ts` stays under the
// code-size gate. The model re-exports nothing from here: importers name this
// file directly.
import type { CadStatus } from '../cad/cad-status.ts';

/** The `cad_status` keys whose value is a real on/off flag. */
export type CadStatusFlagKey = {
  [K in keyof CadStatus]-?: CadStatus[K] extends boolean ? K : never;
}[keyof CadStatus];

/** The status flag `item` is a toggle for, or `null` for a one-shot button. */
export interface RibbonToggleItem {
  readonly activeKey?: CadStatusFlagKey;
}

/**
 * The flag `item` is a toggle for, or `null` for a one-shot button.
 *
 * The renderer uses this to decide whether to emit `aria-pressed`: a command
 * button must not claim a pressed state, while a toggle always reports one.
 */
export function ribbonItemActiveKey(item: RibbonToggleItem): CadStatusFlagKey | null {
  return item.activeKey ?? null;
}

/**
 * Whether `item` reads as checked/pressed against the host's status.
 *
 * Only the host's own flag can make a toggle look on: no status (before the
 * first `cad_status`) or an omitted field is `false`, never a guessed checked
 * state. An item without `activeKey` is never active here.
 */
export function ribbonItemActive(
  status: Partial<CadStatus> | null | undefined,
  item: RibbonToggleItem,
): boolean {
  const key = item.activeKey;
  return key !== undefined && status?.[key] === true;
}
