// `TCH_TEXT` / `TCH_MTEXT`: base-6 text fields. `TCH_MTEXTNEW` is unsupported.
import { readText, reader, skipPropertyBag } from './bit-reader.ts';
import { ClassText } from './base6.ts';
import { bump } from './diagnostics.ts';
import type { Primitive, VendorRecord } from './types.ts';

const MAX_TEXT_BYTES = 4096;

/** Older plain text uses alignment codes 10..13; map them onto MTEXT 1..9. */
function textAttachment(align: number): number {
  return align >= 10 && align <= 13 ? (align === 13 ? 5 : align - 3) : align;
}

export function decodeText(
  record: VendorRecord,
  reasons: Map<string, number>,
): Primitive[] | null {
  const main = reader(record.main, record.mainBits, record.mainSkipBits);

  const base = main.readByte();
  if (base !== 6) {
    bump(reasons, 'vendor.tch.text_unsupported_base');
    return null;
  }
  const scale = main.readBitDouble();
  main.readBitDouble(); // second value of ReadTchBase (ignored)
  const flags = main.readByte();
  if ((flags & 1) !== 0) main.readByte();
  if ((flags & 2) !== 0) main.readByte();
  if (!Number.isFinite(scale) || scale <= 0) {
    bump(reasons, 'vendor.tch.text_geometry_invalid');
    return null;
  }
  if (record.dwgVersion < 1021) {
    // Pre-R2007 text is inline in `main`; this layout is not verified here.
    bump(reasons, 'vendor.tch.text_layout');
    return null;
  }
  const bag = skipPropertyBag(main, null, 0);

  const version = main.readByte();
  if (version < 1 || version > 2) {
    bump(reasons, 'vendor.tch.text_unsupported_version');
    return null;
  }

  // Base text slots come first; the text value is the next string.
  const text = reader(record.text, record.textBits, record.textSkipBits);
  for (let index = 0; index < bag.texts; index += 1) readText(text);
  const value = new ClassText(text).next();

  const x = main.readBitDouble();
  const y = main.readBitDouble();
  const z = main.readBitDouble();
  const height = main.readBitDouble();
  const rotation = main.readBitDouble();
  const attachment = textAttachment(main.readByte());
  if (version >= 2) main.readByte();

  let wrapWidth = 0;
  if (record.className === 'TCH_MTEXT') {
    const mversion = main.readByte();
    if (mversion > 2) {
      bump(reasons, 'vendor.tch.text_unsupported_version');
      return null;
    }
    if (mversion > 1 && main.readByte() >= 16) {
      bump(reasons, 'vendor.tch.text_encrypted');
      return null;
    }
    const boxWidth = main.readBitDouble();
    main.readBitDouble(); // line spacing is not part of the frame
    if (!Number.isFinite(boxWidth)) {
      bump(reasons, 'vendor.tch.text_geometry_invalid');
      return null;
    }
    if (boxWidth > 0) wrapWidth = boxWidth * scale;
  }

  if (main.remaining !== 0) {
    bump(reasons, 'vendor.tch.text_layout');
    return null;
  }
  if (
    !Number.isFinite(x) ||
    !Number.isFinite(y) ||
    !Number.isFinite(z) ||
    !Number.isFinite(height) ||
    !Number.isFinite(rotation) ||
    !Number.isFinite(wrapWidth) ||
    height <= 0 ||
    attachment < 1 ||
    attachment > 9
  ) {
    bump(reasons, 'vendor.tch.text_geometry_invalid');
    return null;
  }
  if (value.length === 0) {
    bump(reasons, 'vendor.tch.text_geometry_invalid');
    return null;
  }
  const encoded = new TextEncoder().encode(value);
  if (encoded.byteLength === 0 || encoded.byteLength > MAX_TEXT_BYTES) {
    bump(reasons, 'vendor.tch.text_layout');
    return null;
  }
  return [{
    kind: 4,
    origin: { x, y, z },
    height: height * scale,
    rotation,
    wrapWidth,
    attachment,
    text: value,
  }];
}
