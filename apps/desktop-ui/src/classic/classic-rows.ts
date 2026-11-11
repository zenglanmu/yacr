// Pure projections for the three classic panels.
//
// Every row comes from `cad_status` and nothing is synthesized: a block without
// stored metadata stays unnamed (its Insert action is disabled with a stable
// reason), a group lists exactly the captured `EntityGroups` members, and an
// image row is one real `IMAGE` entity in its real space.

import { blocksOf, groupsOf, imagesOf } from '../cad/cad-status.ts';
import type {
  CadBlockStatus, CadGroupStatus, CadImageStatus, CadStatus,
} from '../cad/cad-status.ts';

/** One block row plus the action availability the host data itself proves. */
export interface BlockRow extends CadBlockStatus {
  /** Whether the stored name/base point make the INSERT command addressable. */
  readonly insertable: boolean;
}

/** The drawing's block definitions, in the authoritative order. */
export function blockRows(status: Partial<CadStatus> | null | undefined): readonly BlockRow[] {
  return blocksOf(status).map(block => ({
    ...block,
    // INSERT resolves a block by stored name/base point; a missing half is a
    // real refusal, never a zero-point guess.
    insertable: block.name !== null && block.basePoint !== null,
  }));
}

/** The session's entity groups, in creation order. */
export function groupRows(status: Partial<CadStatus> | null | undefined): readonly CadGroupStatus[] {
  return groupsOf(status);
}

/** The drawing's IMAGE entities, in entity-id order. */
export function imageRows(status: Partial<CadStatus> | null | undefined): readonly CadImageStatus[] {
  return imagesOf(status);
}
