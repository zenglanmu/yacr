// The Components panel's command helpers.
//
// The panel wires the two block actions to the ordinary typed selectors:
// `INSERT <name>` starts the insertion capture with the block name supplied up
// front (one click then places it), and `BLOCK <name>` starts the definition
// capture from the current selection. The command layer owns every semantic
// refusal (unknown block, empty selection, duplicate name, …).

/** Trim a user/block name to the one-line form the typed grammar accepts. */
function commandName(name: unknown): string | null {
  if (typeof name !== 'string') return null;
  const trimmed = name.trim();
  if (trimmed === '' || trimmed.includes('\n') || trimmed.includes('\r')) return null;
  return trimmed;
}

/** `INSERT <block>` for a listed block definition, or `null` when unnamed. */
export function componentInsertCommand(name: unknown): string | null {
  const trimmed = commandName(name);
  return trimmed === null ? null : `INSERT ${trimmed}`;
}

/** `BLOCK <name>` for the "create from selection" action, or `null` when blank. */
export function componentDefineCommand(name: unknown): string | null {
  const trimmed = commandName(name);
  return trimmed === null ? null : `BLOCK ${trimmed}`;
}
