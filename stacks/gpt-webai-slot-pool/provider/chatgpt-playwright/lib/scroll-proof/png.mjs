import { deflateSync, inflateSync } from 'node:zlib';

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
function bytesPerPixel(colorType) {
  switch (colorType) {
    case 0:
      return 1;
    case 2:
      return 3;
    case 4:
      return 2;
    case 6:
      return 4;
    default:
      return 0;
  }
}

function channelBytesForColor(colorType) {
  switch (colorType) {
    case 0:
      return 1;
    case 2:
      return 3;
    case 4:
      return 2;
    case 6:
      return 4;
    default:
      return 0;
  }
}


function crc32(buffer) {
  let crc = 0xffffffff;
  for (let index = 0; index < buffer.length; index += 1) {
    crc ^= buffer[index];
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data = Buffer.alloc(0)) {
  const typeBuffer = Buffer.from(type, 'ascii');
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])), 0);
  return Buffer.concat([length, typeBuffer, data, crc]);
}

function normalizedCropRect(decoded, rect = {}) {
  const sourceWidth = Math.max(0, Number(decoded?.width || 0));
  const sourceHeight = Math.max(0, Number(decoded?.height || 0));
  const width = Math.max(1, Math.min(sourceWidth, Math.round(Number(rect.width || sourceWidth || 0))));
  const height = Math.max(1, Math.min(sourceHeight, Math.round(Number(rect.height || sourceHeight || 0))));
  const x = Math.max(0, Math.min(sourceWidth - width, Math.round(Number(rect.x || 0))));
  const y = Math.max(0, Math.min(sourceHeight - height, Math.round(Number(rect.y || 0))));
  return { x, y, width, height };
}

function unfilterScanline(filter, raw, prior, bytesPerPixelValue) {
  const output = Buffer.alloc(raw.length);
  for (let index = 0; index < raw.length; index += 1) {
    const left = index >= bytesPerPixelValue ? output[index - bytesPerPixelValue] : 0;
    const up = prior ? prior[index] : 0;
    const upLeft = prior && index >= bytesPerPixelValue ? prior[index - bytesPerPixelValue] : 0;
    let predictor = 0;
    if (filter === 1) predictor = left;
    else if (filter === 2) predictor = up;
    else if (filter === 3) predictor = Math.floor((left + up) / 2);
    else if (filter === 4) {
      const p = left + up - upLeft;
      const pa = Math.abs(p - left);
      const pb = Math.abs(p - up);
      const pc = Math.abs(p - upLeft);
      predictor = pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft;
    }
    output[index] = (raw[index] + predictor) & 0xff;
  }
  return output;
}

export function decodePng(buffer) {
  if (!Buffer.isBuffer(buffer) || buffer.length < PNG_SIGNATURE.length) {
    throw new Error('png.invalid_buffer');
  }
  if (!buffer.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE)) {
    throw new Error('png.invalid_signature');
  }
  let offset = PNG_SIGNATURE.length;
  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  let interlace = 0;
  const idat = [];
  while (offset + 8 <= buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.subarray(offset + 4, offset + 8).toString('ascii');
    const dataStart = offset + 8;
    const dataEnd = dataStart + length;
    if (dataEnd + 4 > buffer.length) throw new Error('png.truncated_chunk');
    const data = buffer.subarray(dataStart, dataEnd);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
      interlace = data[12];
    } else if (type === 'IDAT') {
      idat.push(data);
    } else if (type === 'IEND') {
      break;
    }
    offset = dataEnd + 4;
  }
  if (!width || !height) throw new Error('png.missing_ihdr');
  if (bitDepth !== 8) throw new Error('png.unsupported_bit_depth');
  if (interlace !== 0) throw new Error('png.unsupported_interlace');
  const pixelBytes = bytesPerPixel(colorType);
  const channelBytes = channelBytesForColor(colorType);
  if (!pixelBytes || !channelBytes) throw new Error('png.unsupported_color_type');

  const inflated = inflateSync(Buffer.concat(idat));
  const rowBytes = width * channelBytes;
  const rows = [];
  let readOffset = 0;
  let prior = null;
  for (let y = 0; y < height; y += 1) {
    const filter = inflated[readOffset];
    readOffset += 1;
    const raw = inflated.subarray(readOffset, readOffset + rowBytes);
    readOffset += rowBytes;
    if (filter > 4 || raw.length !== rowBytes) throw new Error('png.invalid_scanline');
    const row = unfilterScanline(filter, raw, prior, pixelBytes);
    rows.push(row);
    prior = row;
  }
  return { width, height, colorType, rows };
}

export function pixelFor(decoded, x, y) {
  const row = decoded.rows[y];
  if (!row) return null;
  const colorType = decoded.colorType;
  if (colorType === 0) {
    const gray = row[x];
    return { r: gray, g: gray, b: gray, a: 255 };
  }
  if (colorType === 2) {
    const index = x * 3;
    return { r: row[index], g: row[index + 1], b: row[index + 2], a: 255 };
  }
  if (colorType === 4) {
    const index = x * 2;
    const gray = row[index];
    return { r: gray, g: gray, b: gray, a: row[index + 1] };
  }
  if (colorType === 6) {
    const index = x * 4;
    return { r: row[index], g: row[index + 1], b: row[index + 2], a: row[index + 3] };
  }
  return null;
}


export function encodePng(decoded, rect = {}) {
  if (!decoded || !decoded.width || !decoded.height || !Array.isArray(decoded.rows)) {
    throw new Error('png.invalid_decoded_image');
  }
  const channelBytes = channelBytesForColor(decoded.colorType);
  if (!channelBytes) throw new Error('png.unsupported_color_type');
  const crop = normalizedCropRect(decoded, rect);
  const scanlines = [];
  for (let y = crop.y; y < crop.y + crop.height; y += 1) {
    const row = decoded.rows[y];
    if (!row) throw new Error('png.invalid_crop_row');
    const start = crop.x * channelBytes;
    const end = start + crop.width * channelBytes;
    const raw = row.subarray(start, end);
    if (raw.length !== crop.width * channelBytes) throw new Error('png.invalid_crop_width');
    scanlines.push(Buffer.from([0]), raw);
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(crop.width, 0);
  ihdr.writeUInt32BE(crop.height, 4);
  ihdr[8] = 8;
  ihdr[9] = decoded.colorType;
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  return Buffer.concat([
    PNG_SIGNATURE,
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(Buffer.concat(scanlines))),
    pngChunk('IEND'),
  ]);
}

export function encodeRightEdgeCropPng(decoded, cropWidth = 24) {
  const width = Math.max(1, Math.min(Math.round(Number(cropWidth || 24)), Number(decoded?.width || 1)));
  const height = Math.max(1, Number(decoded?.height || 1));
  return encodePng(decoded, {
    x: Math.max(0, Number(decoded.width || 0) - width),
    y: 0,
    width,
    height,
  });
}
