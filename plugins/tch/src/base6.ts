// Shared base-6 header helpers.
//
// Every real `TCH_*` record in the fixture starts its private class body with
// base version **6**. Its layout is `ReadTchBase`'s v5 branch (scale, second
// value, flags + optional base handle counts, property bag), so the message
// helpers below are the single definition of that header for the annotation /
// dimension / stair decoders. Wall, column, opening and text keep their own
// inline copies (they predate this module and must not change behavior).
import { readText, reader, skipPropertyBag } from './bit-reader.ts';
import type { BitReader } from './bit-reader.ts';
import type { VendorRecord } from './types.ts';

/** Base-6 header fields every class body shares. */
export interface TchBase6 {
  /** Drawing scale (for example 100). */
  scale: number;
  /** Handle-stream slots the base already consumed. */
  handles: number;
  /** Text-stream strings the base property bag referenced. */
  texts: number;
}

/** Parse the base-6 header and return the slots it referenced. */
export function readBase6(main: BitReader): TchBase6 {
  const version = main.readByte();
  if (version !== 6) throw new Error('vendor.tch.unsupported_base');
  const scale = main.readBitDouble();
  main.readBitDouble(); // second value of ReadTchBase (ignored)
  const flags = main.readByte();
  let handles = 0;
  if ((flags & 1) !== 0) handles += main.readByte();
  if ((flags & 2) !== 0) handles += main.readByte();
  const bag = skipPropertyBag(main, null, 0);
  if (!Number.isFinite(scale) || scale <= 0) {
    throw new Error('vendor.tch.invalid_scale');
  }
  return { scale, handles: handles + bag.handles, texts: bag.texts };
}

/** `ReadTchSymbolBase`: base-6 plus the symbol text height and extra handles. */
export interface SymbolBase6 extends TchBase6 {
  symbolVersion: number;
  textHeight: number;
}

export function readSymbolBase6(main: BitReader): SymbolBase6 {
  const base = readBase6(main);
  const symbolVersion = main.readByte();
  if (symbolVersion < 1 || symbolVersion > 3) {
    throw new Error('vendor.tch.symbol_base_version');
  }
  const textHeight = main.readBitDouble();
  let handles = base.handles + 1;
  if (symbolVersion >= 2) {
    main.readBitDouble();
    handles += 1;
  }
  if (symbolVersion > 2) main.readBitShort();
  if (!Number.isFinite(textHeight) || textHeight <= 0) {
    throw new Error('vendor.tch.invalid_text_height');
  }
  return { ...base, symbolVersion, textHeight, handles };
}

/** The record's text stream, positioned after common entity data. */
export function textStream(record: VendorRecord): BitReader {
  return reader(record.text, record.textBits, record.textSkipBits);
}

/** Consume `count` strings (the base property bag's slots) from the text stream. */
export function consumeTexts(text: BitReader, count: number): void {
  for (let index = 0; index < count; index += 1) readText(text);
}

const DATA_MAP_BEGIN = 'TDBXDATAMAP_BLOCK_BEGIN_';
const DATA_MAP_END = 'TDBXDATAMAP_BLOCK_END_';

/**
 * Cursor over a class's own strings.
 *
 * After the base property-bag strings, the real AC1027 text stream carries a
 * shared `TDBXDATAMAP_BLOCK_BEGIN_ … TDBXDATAMAP_BLOCK_END_` block (Tianzheng's
 * xdata map) before the class's own labels. Reading raw strings would hand the
 * sentinel to the caller, which is how the first pass emitted
 * `TDBXDATAMAP_BLOCK_BEGIN_` as drawing text. One cursor per record skips that
 * block exactly once and then returns the real strings in order.
 *
 * A record without the block (synthetic frames, older layouts) is unchanged:
 * the first string is buffered and returned as-is. A stream that ends before the
 * next string yields `''` instead of throwing, so a class with no label of its
 * own still decodes; a block that never terminates is a layout error.
 */
export class ClassText {
  private readonly text: BitReader;
  private pending: string | null = null;
  private skipped = false;

  constructor(text: BitReader) {
    this.text = text;
  }

  next(): string {
    if (!this.skipped) {
      this.skipped = true;
      this.skipDataMap();
    }
    if (this.pending !== null) {
      const value = this.pending;
      this.pending = null;
      return value;
    }
    return this.text.remaining < 8 ? '' : readText(this.text);
  }

  private skipDataMap(): void {
    if (this.text.remaining < 8) return;
    const first = readText(this.text);
    if (first !== DATA_MAP_BEGIN) {
      this.pending = first;
      return;
    }
    for (let index = 0; index < 4096; index += 1) {
      if (this.text.remaining < 8) break;
      if (readText(this.text) === DATA_MAP_END) return;
    }
    throw new Error('vendor.tch.datamap_layout');
  }
}
