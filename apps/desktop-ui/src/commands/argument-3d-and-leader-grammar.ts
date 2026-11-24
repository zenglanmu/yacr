// The typed `PYRAMID` n-gon base and the extended `QLEADER`/`LEADER` lines
// (split out of `argument-commands.ts` for the code-size gate).
//
// Every predicate is a local **token shape** only: the host owns the side-count
// range, radius/height validation, the leader style/content resolution and the
// `MAXPOINTS` bound, and answers them with its own named refusals. The free text
// tail of a leader line is content, so it is not inspected here.
import { isSignedDecimal, tokens } from './argument-clip-grammar.ts';

/** Whether `text` is one `x,y` pair of finite signed decimals. */
function isCoordinatePair(text: string): boolean {
  const parts = text.split(',');
  return parts.length === 2 && parts.every(isSignedDecimal);
}

/** `PYRAMID [INSCRIBED|I|CIRCUMSCRIBED|C] <sides> <cx> <cy> <radius> <height> [<top_radius>]`. */
export function isPyramidArgument(argument: string): boolean {
  const parts = tokens(argument);
  const first = (parts[0] ?? '').toUpperCase();
  const offset = ['INSCRIBED', 'I', 'CIRCUMSCRIBED', 'C'].includes(first) ? 1 : 0;
  const values = parts.slice(offset);
  return (values.length === 5 || values.length === 6)
    && /^[0-9]+$/.test(values[0])
    && values.slice(1).every(isSignedDecimal);
}

/** `LAYERSTATESAVE [<name>|?]`: `?` alone is the list spelling, anything else a name. */
export function isLayerStateSaveArgument(argument: string): boolean {
  const text = argument.trim();
  if (text.length === 0) return false;
  return text === '?' || !text.startsWith('?');
}

/**
 * `QLEADER`/`LEADER`: the shared `CONTENT MTEXT|BLOCK <name>|TOLERANCE` prefix,
 * `QLEADER`'s own `ARROW ON|OFF`/`MAXPOINTS <n>`, then two `x,y` pairs and an
 * optional (possibly empty) text tail.
 *
 * The text tail is free content, so it is not inspected here; the host owns every
 * semantic refusal (the shared MULTILEADER content/style answers and
 * `qleader.invalid_max_points`).
 */
export function isLeaderArgument(argument: string, qleader: boolean): boolean {
  const parts = tokens(argument);
  let index = 0;
  let content = false;
  let arrow = false;
  let maxPoints = false;
  while (index < parts.length) {
    const word = (parts[index] ?? '').toUpperCase();
    if (word === 'CONTENT') {
      if (content) return false;
      content = true;
      const kind = (parts[index + 1] ?? '').toUpperCase();
      if (kind === 'BLOCK') {
        if (parts.length <= index + 2) return false;
        index += 3;
      } else if (kind === 'MTEXT' || kind === 'TOLERANCE') {
        index += 2;
      } else {
        return false;
      }
    } else if (qleader && word === 'ARROW') {
      if (arrow) return false;
      const value = (parts[index + 1] ?? '').toUpperCase();
      if (value !== 'ON' && value !== 'OFF') return false;
      arrow = true;
      index += 2;
    } else if (qleader && word === 'MAXPOINTS') {
      if (maxPoints) return false;
      if (!/^[0-9]+$/.test(parts[index + 1] ?? '')) return false;
      maxPoints = true;
      index += 2;
    } else {
      break;
    }
  }
  const coordinates = parts.slice(index);
  return coordinates.length >= 2
    && isCoordinatePair(coordinates[0]) && isCoordinatePair(coordinates[1]);
}
