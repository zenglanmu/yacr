// The built-in flat material palette (SketchUp's colour wheel equivalent).
//
// The authority is `crates/cad-domain/src/material.rs` (`BUILTIN_MATERIALS`);
// this list exists only so the tray can draw a swatch and dispatch
// `PAINT <name>`. The command resolves the name against the document table first
// and this palette second, so a name that drifted here would be a named
// `paint.unknown_material` refusal, never a silent no-op. The document's own
// rows come from the `cad_materials` host read below.
import type { Invoke } from '../../host/invoke.ts';

export interface BuiltinMaterial {
  readonly name: string;
  readonly color: string;
  readonly transparency?: number;
}

export const BUILTIN_MATERIALS: readonly BuiltinMaterial[] = Object.freeze([
  { name: 'Red', color: '#b22222' },
  { name: 'Orange', color: '#e67e22' },
  { name: 'Yellow', color: '#f1c40f' },
  { name: 'Green', color: '#27ae60' },
  { name: 'Cyan', color: '#1abc9c' },
  { name: 'Blue', color: '#2980b9' },
  { name: 'Purple', color: '#8e44ad' },
  { name: 'Gray', color: '#7f8c8d' },
  { name: 'White', color: '#ecf0f1' },
  { name: 'Black', color: '#2c3e50' },
  { name: 'Wood', color: '#a06e3c' },
  { name: 'Glass', color: '#aadcf0', transparency: 0.6 },
]);

/** One editable document material row. */
export interface MaterialDraft {
  readonly name: string;
  /** `#rrggbb`. */
  readonly color: string;
  /** `0.0` opaque … `1.0` transparent. */
  readonly transparency: number;
}

function safeMaterialName(name: unknown): string | null {
  const trimmed = typeof name === 'string' ? name.trim() : '';
  if (trimmed === '' || trimmed.includes('"')
    || trimmed.includes('\n') || trimmed.includes('\r')) {
    return null;
  }
  return trimmed;
}

/**
 * The `MATERIAL` commands for one edited row, in rename-then-value order.
 *
 * Only changed fields are emitted; each command is one ordinary write/undo step.
 * A later color/transparency command addresses the new name after a rename.
 * Unsafe names/colours/transparency produce `null`, never a guessed command.
 */
export function materialEditCommands(
  current: MaterialDraft,
  draft: MaterialDraft,
): readonly string[] | null {
  const oldName = safeMaterialName(current.name);
  const newName = safeMaterialName(draft.name);
  const color = typeof draft.color === 'string' && /^#[0-9a-fA-F]{6}$/.test(draft.color)
    ? draft.color.toLowerCase()
    : null;
  if (oldName === null || newName === null || color === null) return null;
  if (!Number.isFinite(draft.transparency)
    || draft.transparency < 0 || draft.transparency > 1) {
    return null;
  }
  const commands: string[] = [];
  if (newName !== oldName) {
    commands.push(`MATERIAL "${oldName}" RENAME "${newName}"`);
  }
  if (color !== current.color.toLowerCase()) {
    const r = Number.parseInt(color.slice(1, 3), 16);
    const g = Number.parseInt(color.slice(3, 5), 16);
    const b = Number.parseInt(color.slice(5, 7), 16);
    commands.push(`MATERIAL "${newName}" COLOR ${r} ${g} ${b}`);
  }
  if (draft.transparency !== current.transparency) {
    commands.push(`MATERIAL "${newName}" TRANSPARENCY ${draft.transparency}`);
  }
  return commands;
}

/** `MATERIAL "<name>" DELETE` for a document material, or null when unsafe. */
export function materialDeleteCommand(name: unknown): string | null {
  const trimmed = typeof name === 'string' ? name.trim() : '';
  if (trimmed === '' || trimmed.includes('"')
    || trimmed.includes('\n') || trimmed.includes('\r')) {
    return null;
  }
  return `MATERIAL "${trimmed}" DELETE`;
}

/** The command line one swatch dispatches. */
export function paintCommand(name: string): string {
  return `PAINT ${name}`;
}

/** The one read command (`kind: "materials"`). */
export const MATERIALS_COMMAND = 'cad_materials';

/** One document material row. */
export interface DocumentMaterial {
  /** Stored name (the key `PAINT` takes). */
  readonly name: string;
  /** `#rrggbb` sRGB colour. */
  readonly color: string;
  /** `0.0` opaque … `1.0` fully transparent. */
  readonly transparency: number;
}

/** One normalised answer. */
export interface MaterialsAnswer {
  readonly materials: readonly DocumentMaterial[];
  /** The host's own total (may exceed `materials.length`). */
  readonly total: number;
  readonly truncated: boolean;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** `[r, g, b]` bytes to `#rrggbb`; anything else is not a colour. */
function hexColor(value: unknown): string | null {
  if (!Array.isArray(value) || value.length !== 3) return null;
  const bytes: number[] = [];
  for (const channel of value) {
    if (typeof channel !== 'number' || !Number.isInteger(channel) || channel < 0 || channel > 255) {
      return null;
    }
    bytes.push(channel);
  }
  return `#${bytes.map(channel => channel.toString(16).padStart(2, '0')).join('')}`;
}

/** Normalise one `cad_materials` answer; a malformed row is dropped. */
export function parseMaterialsAnswer(answer: unknown): MaterialsAnswer {
  const source = recordOf(answer);
  const rawRows: unknown[] = Array.isArray(source?.materials) ? source.materials : [];
  const materials: DocumentMaterial[] = [];
  const seen = new Set<string>();
  for (const raw of rawRows) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const name = typeof fields.name === 'string' && fields.name !== '' ? fields.name : null;
    const color = hexColor(fields.color);
    const transparency = fields.transparency;
    if (name === null || color === null || seen.has(name)) continue;
    if (typeof transparency !== 'number' || !Number.isFinite(transparency)
      || transparency < 0 || transparency > 1) {
      continue;
    }
    seen.add(name);
    materials.push({ name, color, transparency });
  }
  const reported = source?.total;
  return {
    materials,
    total: typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= 0
      ? reported
      : materials.length,
    truncated: source?.truncated === true,
  };
}

/** The one read. */
export async function readMaterials(invoke: Invoke): Promise<MaterialsAnswer> {
  return parseMaterialsAnswer(await invoke(MATERIALS_COMMAND));
}
