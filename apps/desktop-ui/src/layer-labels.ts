// What the layers palette may say about a refusal, and which rows it may offer to
// delete. Pure data + predicates (no JSX, no React, no DOM), the same shape
// `property-labels.ts` uses for the inspector's machine keys.
//
// Two rules this module exists to keep in one place:
//
//  * **Copy is looked up, never written here.** The host refuses a layer-table
//    write with a stable machine code (`layer.in_use: layer 'WALLS' (id 2) is
//    still referenced by 4 entities; not deleted`), and the catalog already
//    carries one sentence per code under `layers.error.<code>` — the same
//    `options.error.<wire code>` convention the options dialog uses. The switch
//    below is the whole mapping, so an unmapped code can only fall back to the
//    palette's generic sentence; it is never turned into guessed prose.
//  * **The default layer is not offered for deletion at all.** The host refuses
//    it (`layer.default_undeletable`), and a destructive affordance that can only
//    ever be refused must not be shown: hiding it is the honest UI, keeping a
//    disabled button would still say "there is something here you cannot do".
//
// Refusal codes come from `cad-db`'s layer writes; `errorCode()` (in `layers.ts`)
// strips the human sentence after the first `:` so only the code is compared
// here. Unknown, empty and non-string codes answer `null` — the palette then
// shows the host's own words through `layers.write_failed`, exactly as it
// does for every other mutation.

/** The drawing's default layer: the host refuses to delete it. */
export const DEFAULT_LAYER_NAME = '0';

/**
 * The catalog key for one layer refusal code, or `null` when this build has no
 * copy for it (in which case the raw reason is reported instead).
 */
export function layerErrorKey(code: unknown): string | null {
  switch (typeof code === 'string' ? code : '') {
    case 'layer.name_empty': return 'layers.error.layer.name_empty';
    case 'layer.name_illegal_char': return 'layers.error.layer.name_illegal_char';
    case 'layer.duplicate_name': return 'layers.error.layer.duplicate_name';
    case 'layer.default_undeletable': return 'layers.error.layer.default_undeletable';
    case 'layer.in_use': return 'layers.error.layer.in_use';
    // The attribute writes (`cad_set_layer_property`): a key this build cannot
    // store, or a value that key's grammar refuses. Both are reachable from the
    // palette's color control and flag chips, so both get copy rather than the
    // generic sentence.
    case 'layer.property.unsupported': return 'layers.error.layer.property.unsupported';
    case 'layer.property.value': return 'layers.error.layer.property.value';
    default: return null;
  }
}

/**
 * Whether the palette may offer to delete this row.
 *
 * `false` for the default layer (the host's own rule, mirrored here so the
 * affordance is never shown), and `false` for a name that could not address a
 * layer at all — an empty or non-string name is not a row.
 */
export function deletableLayer(name: unknown): boolean {
  return typeof name === 'string' && name !== '' && name !== DEFAULT_LAYER_NAME;
}
