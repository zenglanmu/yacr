// Local syntax predicates for the block-attribute command lines (ADR-0055 M3).
//
// These mirror `cad-ui-core`'s parser (`session::commands::attribute`) only far
// enough to decide whether a line *looks like* the documented form. The host is
// the semantic authority: it resolves the block/tag, refuses constants and
// locked positions, and owns every write. A line admitted here can still be
// refused by the router, and a malformed line never reaches it.

import { splitQuotedTokens } from './argument-extras-grammar.ts';

const DEFINITION_OPTIONS = new Set([
  'PROMPT', 'POSITION', 'HEIGHT', 'ROTATION', 'ORDER', 'WIDTH', 'WRAP', 'OBLIQUE',
  'CONSTANT', 'INVISIBLE', 'VERIFY', 'PRESET', 'LOCK', 'LOCKPOSITION', 'ANNOTATIVE',
]);

function number(token: string | undefined): boolean {
  if (token === undefined || token.length === 0) return false;
  return Number.isFinite(Number(token));
}

/** `ATTDEF`/`-ATTDEF`/`ATTREDEF <block> <tag> <default> [options]`. */
export function isAttributeDefinitionArgument(text: string): boolean {
  const tokens = splitQuotedTokens(text);
  if (!tokens || tokens.length < 3) return false;
  if (!tokens[0].trim() || !tokens[1].trim()) return false;
  let index = 3;
  while (index < tokens.length) {
    const option = tokens[index].toUpperCase();
    index += 1;
    switch (option) {
      case 'PROMPT':
        if (index >= tokens.length) return false;
        index += 1;
        break;
      case 'POSITION':
        if (!number(tokens[index]) || !number(tokens[index + 1])) return false;
        index += 2;
        if (index < tokens.length && !DEFINITION_OPTIONS.has(tokens[index].toUpperCase())) {
          if (!number(tokens[index])) return false;
          index += 1;
        }
        break;
      case 'HEIGHT':
      case 'ROTATION':
      case 'ORDER':
      case 'WIDTH':
      case 'WRAP':
      case 'OBLIQUE':
        if (!number(tokens[index])) return false;
        index += 1;
        break;
      case 'CONSTANT':
      case 'INVISIBLE':
      case 'VERIFY':
      case 'PRESET':
      case 'LOCK':
      case 'LOCKPOSITION':
      case 'ANNOTATIVE':
        break;
      default:
        return false;
    }
  }
  return true;
}

/** `ATTEDIT`/`-ATTEDIT`/`ATTIPEDIT`/`GATTE <block> <tag> <value>`. */
export function isAttributeEditArgument(text: string): boolean {
  const tokens = splitQuotedTokens(text);
  return !!tokens && tokens.length === 3 && !!tokens[0].trim() && !!tokens[1].trim();
}

/** `EATTEDIT <block> <tag>=<value> ... [POSITION x y] [HEIGHT h] [ROTATION deg]`. */
export function isEnhancedAttributeEditArgument(text: string): boolean {
  const tokens = splitQuotedTokens(text);
  if (!tokens || tokens.length < 2 || !tokens[0].trim()) return false;
  let assignments = 0;
  let index = 1;
  while (index < tokens.length) {
    const option = tokens[index].toUpperCase();
    if (option === 'POSITION') {
      if (!number(tokens[index + 1]) || !number(tokens[index + 2])) return false;
      index += 3;
      if (index < tokens.length && !tokens[index].includes('=')) {
        if (!number(tokens[index])) return false;
        index += 1;
      }
      continue;
    }
    if (option === 'HEIGHT' || option === 'ROTATION') {
      if (!number(tokens[index + 1])) return false;
      index += 2;
      continue;
    }
    const token = tokens[index];
    if (!token.includes('=') || !token.split('=')[0].trim()) return false;
    assignments += 1;
    index += 1;
  }
  return assignments > 0;
}

/** `ATTSYNC <block>`. */
export function isAttributeSyncArgument(text: string): boolean {
  const tokens = splitQuotedTokens(text);
  return !!tokens && tokens.length === 1 && !!tokens[0].trim();
}

/** `BATTORDER <block> <tag>,<tag>,...`. */
export function isAttributeOrderArgument(text: string): boolean {
  const tokens = splitQuotedTokens(text);
  if (!tokens || tokens.length < 2 || !tokens[0].trim()) return false;
  return tokens.slice(1).every(token => token.split(',').every(tag => !!tag.trim()));
}

/** `ATTDISP NORMAL|ON|OFF`. */
export function isAttributeDisplayArgument(text: string): boolean {
  return ['NORMAL', 'N', 'ON', 'OFF'].includes(text.trim().toUpperCase());
}

/** The typed-argument spec rows `argument-commands.ts` spreads into its table. */
export const ATTRIBUTE_ARGUMENT_COMMANDS = Object.freeze({
  ATTDEF: Object.freeze({ parse: isAttributeDefinitionArgument }),
  '-ATTDEF': Object.freeze({ parse: isAttributeDefinitionArgument }),
  ATTREDEF: Object.freeze({ parse: isAttributeDefinitionArgument }),
  ATTEDIT: Object.freeze({ parse: isAttributeEditArgument }),
  '-ATTEDIT': Object.freeze({ parse: isAttributeEditArgument }),
  ATTIPEDIT: Object.freeze({ parse: isAttributeEditArgument }),
  GATTE: Object.freeze({ parse: isAttributeEditArgument }),
  EATTEDIT: Object.freeze({ parse: isEnhancedAttributeEditArgument }),
  ATTSYNC: Object.freeze({ parse: isAttributeSyncArgument }),
  BATTORDER: Object.freeze({ parse: isAttributeOrderArgument }),
  ATTDISP: Object.freeze({ parse: isAttributeDisplayArgument }),
});
