// The BIM panel's one action on the drawing: ask the host to select the
// geometry rows of one logical component.
//
// One component can be several geometry rows (a wall split into meshes). The
// panel always sends **every** `entityIds` entry of the row, so a press selects
// the whole component, never one arbitrary part of it. The host is the
// authority on what "selected" means: this module only sends the request and
// reports the host's own answer.
//
// # No optimistic selection
//
// A resolved promise is an **acknowledgement**, not a selection: the panel does
// not paint a row as selected, and it does not edit any local "selected" list.
// The selection itself is whatever `cad_status.selection` reports on the next
// poll. A refusal (`unsupported-runtime: cad_select_entities` on a carrier that
// does not serve it, or a host-side rejection) is returned unchanged and the
// panel shows the host's words.
import type { Invoke } from './bim-query.ts';
import { reasonText } from './bim-query.ts';

/**
 * The host command that selects a set of entity ids.
 *
 * Named here as the panel's single selection seam: a carrier without this
 * command answers its own `unsupported-runtime: …` refusal, which the panel
 * shows verbatim rather than claiming the row was selected.
 */
export const BIM_SELECT_COMMAND = 'cad_select_entities';

/** The payload the command takes. */
export interface BimSelectPayload {
  readonly entityIds: readonly number[];
}

/** The outcome of one selection request. */
export type BimSelectResult =
  | { readonly kind: 'ok'; readonly count: number }
  | { readonly kind: 'invalid' }
  | { readonly kind: 'error'; readonly reason: string };

/** A non-negative safe integer id (what the wire's `entityIds` carries). */
function isEntityId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

/**
 * The request body for one row's ids, or `null` when there is nothing to
 * select. Duplicates are dropped and the order is kept, so one click is one
 * request of exactly the component's geometry rows.
 */
export function bimSelectionPayload(ids: readonly unknown[] | null | undefined): BimSelectPayload | null {
  if (!Array.isArray(ids)) return null;
  const seen = new Set<number>();
  const entityIds: number[] = [];
  for (const raw of ids) {
    if (!isEntityId(raw) || seen.has(raw)) continue;
    seen.add(raw);
    entityIds.push(raw);
  }
  return entityIds.length > 0 ? { entityIds } : null;
}

/**
 * Ask the host to select these entities.
 *
 * An ack that says `accepted: false` (the layers mutation shape) and a thrown
 * refusal are both returned as `{kind:'error'}`; the panel reports the reason.
 * Nothing is retried behind the user's back.
 */
export async function selectBimEntities(
  invoke: Invoke,
  ids: readonly unknown[] | null | undefined,
): Promise<BimSelectResult> {
  const payload = bimSelectionPayload(ids);
  if (!payload) return { kind: 'invalid' };
  try {
    const ack = await invoke(BIM_SELECT_COMMAND, { entityIds: payload.entityIds });
    const record = ack !== null && typeof ack === 'object'
      ? ack as Record<string, unknown>
      : null;
    if (record?.accepted === false) {
      const reason = typeof record.error === 'string' && record.error.length > 0
        ? record.error
        : 'CAD_COMMAND_NOT_ACCEPTED';
      return { kind: 'error', reason };
    }
    return { kind: 'ok', count: payload.entityIds.length };
  } catch (error) {
    return { kind: 'error', reason: reasonText(error) };
  }
}
