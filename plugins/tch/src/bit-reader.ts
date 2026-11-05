// Shared MSB-first bit reader, text/handle readers and property-bag walker.
// A read past the declared bit length throws `vendor.truncated`.

export const MAX_POINT_ABS = 1e12;
export const MAX_BULGE = 1e6;

export interface Point3 { x: number; y: number; z: number }

/** MSB-first reader bounded by the stream's declared bit length. */
export class BitReader {
  readonly bytes: Uint8Array; readonly bitLength: number; position: number;
  constructor(bytes: Uint8Array, bitLength: number, startBit: number) {
    if (bytes.byteLength * 8 < bitLength || startBit > bitLength) throw new Error('vendor.truncated');
    this.bytes = bytes; this.bitLength = bitLength; this.position = startBit;
  }
  get remaining(): number { return this.bitLength - this.position; }
  readBit(): number {
    if (this.position >= this.bitLength) throw new Error('vendor.truncated');
    const value = (this.bytes[this.position >>> 3] >>> (7 - (this.position & 7))) & 1;
    this.position += 1;
    return value;
  }
  readBits(count: number): number {
    if (count < 0 || count > 32 || this.position + count > this.bitLength) throw new Error('vendor.truncated');
    let value = 0;
    for (let index = 0; index < count; index += 1) {
      value = value * 2 + ((this.bytes[this.position >>> 3] >>> (7 - (this.position & 7))) & 1);
      this.position += 1;
    }
    return value;
  }
  readByte(): number { return this.readBits(8); }
  readRawShort(): number {
    const raw = this.readBits(8) | (this.readBits(8) << 8);
    return raw >= 0x8000 ? raw - 0x10000 : raw;
  }
  readRawLong(): number {
    const b0 = this.readBits(8), b1 = this.readBits(8), b2 = this.readBits(8), b3 = this.readBits(8);
    return (b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) | 0;
  }
  readRawDouble(): number {
    const bytes = new Uint8Array(8);
    for (let index = 0; index < 8; index += 1) bytes[index] = this.readBits(8);
    return new DataView(bytes.buffer).getFloat64(0, true);
  }
  readBitShort(): number {
    const code = this.readBits(2);
    return code === 0 ? this.readRawShort() : code === 1 ? this.readBits(8) : code === 2 ? 0 : 256;
  }
  readBitLong(): number {
    const code = this.readBits(2);
    return code === 0 ? this.readRawLong() : code === 1 ? this.readBits(8) : 0;
  }
  readBitDouble(): number {
    const code = this.readBits(2);
    return code === 0 ? this.readRawDouble() : code === 1 ? 1.0 : 0.0;
  }
  /**
   * DWG `ReadBitDoubleWithDefault`. `def` starts as its little-endian bytes:
   * code 1 replaces the low 4 bytes, code 2 replaces bytes 4/5 then 0..3, and
   * code 3 reads a full raw double.
   */
  readBitDoubleWithDefault(def: number): number {
    const view = new DataView(new ArrayBuffer(8));
    view.setFloat64(0, def, true);
    const bytes = new Uint8Array(view.buffer);
    const code = this.readBits(2);
    if (code === 0) return def;
    if (code === 1) {
      for (let index = 0; index < 4; index += 1) bytes[index] = this.readByte();
    } else if (code === 2) {
      bytes[4] = this.readByte();
      bytes[5] = this.readByte();
      for (let index = 0; index < 4; index += 1) bytes[index] = this.readByte();
    } else {
      return this.readRawDouble();
    }
    return view.getFloat64(0, true);
  }
  read3BitDouble(): Point3 { return { x: this.readBitDouble(), y: this.readBitDouble(), z: this.readBitDouble() }; }
  /** DWG handle: nibble form, big-endian value bytes, reference-relative codes. */
  readHandle(reference: bigint): bigint {
    const form = this.readByte();
    const code = form >>> 4, counter = form & 0xf;
    if (counter > 8) throw new Error('vendor.tch.handle');
    const bytes = (): bigint => {
      let value = 0n;
      for (let index = 0; index < counter; index += 1) value = (value << 8n) | BigInt(this.readByte());
      return value;
    };
    switch (code) {
      case 0: case 1: case 2: case 3: case 4: case 5: return bytes();
      case 6: return reference + 1n;
      case 8: return reference - 1n;
      case 0xa: return reference + bytes();
      case 0xc: return reference - bytes();
      default: throw new Error('vendor.tch.handle');
    }
  }
}

/** Build a reader, rejecting bit metadata that points past the bytes. */
export function reader(bytes: Uint8Array, bitLength: number, startBit: number): BitReader {
  return new BitReader(bytes, bitLength, startBit);
}

export interface PropertyBagCounts { handles: number; texts: number }

/**
 * Consume the property bag; returns the handle and text slots it referenced.
 * `text` non-null (columns) consumes strings as before; `null` (openings/text)
 * only counts them so the caller can consume `texts` strings later.
 */
export function skipPropertyBag(
  main: BitReader,
  text: BitReader | null,
  depth: number,
): PropertyBagCounts {
  if (depth > 8) throw new Error('vendor.tch.layout');
  const count = main.readBitLong();
  if (count < 0 || count > 4096) throw new Error('vendor.tch.layout');
  let handles = 0;
  let texts = 0;
  for (let index = 0; index < count; index += 1) {
    if (text) readText(text); // item name
    texts += 1;
    const type = main.readBitLong();
    switch (type) {
      case 0:
        main.readBit();
        break;
      case 1:
      case 2:
      case 3:
        main.readByte();
        break;
      case 4:
      case 5:
        main.readBitShort();
        break;
      case 6:
      case 7:
        main.readBitLong();
        break;
      case 100: {
        const nested = skipPropertyBag(main, text, depth + 1);
        handles += nested.handles;
        texts += nested.texts;
        break;
      }
      case 0x65:
        if (text) readText(text); // string value
        texts += 1;
        break;
      case 200:
        main.readBitDouble();
        break;
      case 0xc9:
      case 0xcb:
        main.readRawDouble();
        main.readRawDouble();
        break;
      case 0xca:
      case 0xcc:
      case 0xcd:
        main.readBitDouble();
        main.readBitDouble();
        main.readBitDouble();
        break;
      case 0xd0:
      case 0xd1:
        // Handle value lives in the handle stream; count the slot only.
        handles += 1;
        break;
      default:
        throw new Error('vendor.tch.layout');
    }
  }
  return { handles, texts };
}

/** AC1027 text stream: `BitShort utf16_count` + `count` UTF-16LE code units. */
export function readText(text: BitReader): string {
  const count = text.readBitShort();
  if (count < 0 || count > 65535) throw new Error('vendor.tch.layout');
  let value = '';
  for (let index = 0; index < count; index += 1) {
    const low = text.readByte();
    const high = text.readByte();
    value += String.fromCharCode(low | (high << 8));
  }
  return value;
}
