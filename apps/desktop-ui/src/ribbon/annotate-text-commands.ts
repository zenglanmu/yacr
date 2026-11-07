// The three Express-tool text commands (2026-11-07), kept in their own small
// module so `ribbon-model.ts` stays inside the file-size gate.
//
// TORIENT and TXT2MTXT are plain selection-based router buttons (the bare
// TORIENT is Most Readable; the absolute form is the typed `TORIENT <angle>`
// line). TCASE exposes its five explicit modes as popover rows; the base click
// stays the bare Title form the host applies by default.
import type { RibbonCommandItem } from './ribbon-model.ts';

export const ANNOTATE_TEXT_COMMAND_ITEMS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'TORIENT', labelKey: 'draw.kind.torient', icon: 'rotate', command: 'TORIENT' },
  {
    id: 'TCASE',
    labelKey: 'draw.kind.tcase',
    icon: 'pencil',
    command: 'TCASE',
    variants: [
      { id: 'TCASE TITLE', labelKey: 'draw.kind.tcase.title', icon: 'pencil', command: 'TCASE T' },
      { id: 'TCASE UPPER', labelKey: 'draw.kind.tcase.upper', icon: 'pencil', command: 'TCASE U' },
      { id: 'TCASE LOWER', labelKey: 'draw.kind.tcase.lower', icon: 'pencil', command: 'TCASE L' },
      { id: 'TCASE SENTENCE', labelKey: 'draw.kind.tcase.sentence', icon: 'pencil', command: 'TCASE S' },
      { id: 'TCASE TOGGLE', labelKey: 'draw.kind.tcase.toggle', icon: 'pencil', command: 'TCASE G' },
    ],
  },
  { id: 'TXT2MTXT', labelKey: 'draw.kind.txt2mtxt', icon: 'copy', command: 'TXT2MTXT' },
  // TEXTMASK/TEXTUNMASK (2026-11-07) are bare, selection-based commands (no
  // argument), so a plain button is a real affordance. The mask is an
  // independent `AcDbWipeout` entity behind the estimated text extents and the
  // unmask heuristic only ever deletes **selected** masks.
  { id: 'TEXTMASK', labelKey: 'draw.kind.textmask', icon: 'hatch', command: 'TEXTMASK' },
  { id: 'TEXTUNMASK', labelKey: 'draw.kind.textunmask', icon: 'hatch', command: 'TEXTUNMASK' },
]);
