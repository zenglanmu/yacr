import assert from "node:assert/strict";
import { test } from "node:test";
import zlib from "node:zlib";
import { decodePng } from "./web-pixels.mjs";

function chunk(type, bytes) {
  const result = Buffer.alloc(bytes.length + 12);
  result.writeUInt32BE(bytes.length);
  result.write(type, 4);
  bytes.copy(result, 8);
  return result;
}

test("CAD screenshot decoder reverses all five PNG row filters", () => {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(2);
  header.writeUInt32BE(5, 4);
  header[8] = 8;
  header[9] = 2;
  // Five identical RGB rows encoded with None/Sub/Up/Average/Paeth.
  const row = [10, 20, 30, 40, 50, 60];
  const encoded = Buffer.from([
    0,
    ...row,
    1,
    10,
    20,
    30,
    30,
    30,
    30,
    2,
    0,
    0,
    0,
    0,
    0,
    0,
    3,
    5,
    10,
    15,
    15,
    15,
    15,
    4,
    0,
    0,
    0,
    0,
    0,
    0,
  ]);
  const png = Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", header),
    chunk("IDAT", zlib.deflateSync(encoded)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
  const decoded = decodePng(png);
  assert.equal(decoded.width, 2);
  assert.equal(decoded.height, 5);
  assert.deepEqual(
    [...decoded.pixels],
    Array.from({ length: 5 }, () => row).flat(),
  );
});
