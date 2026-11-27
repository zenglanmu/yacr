// The built-in flat material palette (SketchUp's colour wheel equivalent).
//
// The authority is `crates/cad-domain/src/material.rs` (`BUILTIN_MATERIALS`);
// this list exists only so the tray can draw a swatch and dispatch
// `PAINT <name>`. The command resolves the name against the document table first
// and this palette second, so a name that drifted here would be a named
// `paint.unknown_material` refusal, never a silent no-op. Document materials
// (created by painting a built-in) are listed by the `MATERIALS` report.
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

/** The command line one swatch dispatches. */
export function paintCommand(name: string): string {
  return `PAINT ${name}`;
}
