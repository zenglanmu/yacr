// Decode Chromium's 8-bit RGB/RGBA PNG captures without extra dependencies.
import zlib from "node:zlib";

export function decodePng(buffer) {
  assertPng(buffer);
  let offset = 8,
    width = 0,
    height = 0,
    colorType = 0;
  const idat = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString("ascii", offset + 4, offset + 8);
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[12] !== 0)
        throw new Error("unsupported PNG format");
      colorType = data[9];
    } else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    offset += 12 + length;
  }
  if (colorType !== 2 && colorType !== 6)
    throw new Error("unsupported PNG color type");
  const channels = colorType === 6 ? 4 : 3;
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const pixels = Buffer.alloc(height * stride);
  let previous = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const current = Buffer.alloc(stride);
    for (let x = 0; x < stride; x += 1) {
      const left = x >= channels ? current[x - channels] : 0;
      const up = previous[x];
      const upLeft = x >= channels ? previous[x - channels] : 0;
      let predictor;
      switch (filter) {
        case 0:
          predictor = 0;
          break;
        case 1:
          predictor = left;
          break;
        case 2:
          predictor = up;
          break;
        case 3:
          predictor = (left + up) >> 1;
          break;
        case 4: {
          const p = left + up - upLeft;
          const a = Math.abs(p - left),
            b = Math.abs(p - up),
            c = Math.abs(p - upLeft);
          predictor = a <= b && a <= c ? left : b <= c ? up : upLeft;
          break;
        }
        default:
          throw new Error("unsupported PNG filter");
      }
      current[x] = (line[x] + predictor) & 0xff;
    }
    current.copy(pixels, y * stride);
    previous = current;
  }
  return { width, height, channels, pixels, stride };
}

function assertPng(buffer) {
  if (buffer.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
}
