
import { readFile } from 'node:fs/promises';

import { DEFAULT_BOTTOM_GAP_EPSILON_PX, clampNumber } from './util.mjs';
import { decodePng, pixelFor } from './png.mjs';

function brightness(pixel) {
  if (!pixel) return 255;
  return (pixel.r + pixel.g + pixel.b) / 3;
}

function scrollbarPixel(pixel, options = {}) {
  if (!pixel || pixel.a < 16) return false;
  const maxBrightness = clampNumber(options.maxBrightness, 205);
  const minColorSpread = clampNumber(options.maxColorSpread, 36);
  const spread = Math.max(pixel.r, pixel.g, pixel.b) - Math.min(pixel.r, pixel.g, pixel.b);
  return brightness(pixel) <= maxBrightness && spread <= minColorSpread;
}

function segmentsFromRows(rows) {
  const segments = [];
  let start = null;
  for (let y = 0; y < rows.length; y += 1) {
    if (rows[y] && start === null) start = y;
    if ((!rows[y] || y === rows.length - 1) && start !== null) {
      const end = rows[y] ? y : y - 1;
      segments.push({ start, end, height: end - start + 1 });
      start = null;
    }
  }
  return segments;
}

function rowHasScrollbarPixels(decoded, y, options = {}) {
  const minPixels = clampNumber(options.minPixelsPerRow, 2);
  let count = 0;
  for (let x = 0; x < decoded.width; x += 1) {
    if (scrollbarPixel(pixelFor(decoded, x, y), options)) count += 1;
  }
  return count >= minPixels;
}

function columnSegmentsForRows(decoded, startY, endY, options = {}) {
  const height = Math.max(0, endY - startY + 1);
  if (height <= 0) return [];
  const minRowsPerColumn = Math.max(12, Math.round(height * 0.35));
  const columns = Array.from({ length: decoded.width }, (_, x) => {
    let count = 0;
    for (let y = startY; y <= endY; y += 1) {
      if (scrollbarPixel(pixelFor(decoded, x, y), options)) count += 1;
    }
    return count >= minRowsPerColumn;
  });
  return segmentsFromRows(columns);
}

function clippedContentArtifact(decoded, thumb, options = {}) {
  const minArtifactWidth = clampNumber(options.minArtifactCropWidth, 16);
  if (!decoded || decoded.width < minArtifactWidth || !thumb) return null;
  const columnSegments = columnSegmentsForRows(decoded, thumb.start, thumb.end, options);
  if (columnSegments.length === 0) return null;
  const rightHalfStart = Math.floor(decoded.width * 0.5);
  const expectedRightEdgeStart = Math.floor(decoded.width * 0.42);
  const largest = [...columnSegments].sort((left, right) => right.height - left.height)[0];

  if (largest.end < rightHalfStart) {
    return {
      reason: 'dominant_dark_band_is_not_on_right_edge',
      columnSegments: columnSegments.slice(0, 8),
    };
  }

  const leftContamination = columnSegments.find(segment => segment.start <= 1 && segment.end < expectedRightEdgeStart);
  const rightBand = columnSegments.find(segment => segment.start >= expectedRightEdgeStart || segment.end >= rightHalfStart);
  if (leftContamination && rightBand) {
    return {
      reason: 'left_edge_dark_band_present_with_right_edge_band',
      columnSegments: columnSegments.slice(0, 8),
    };
  }
  return null;
}

export function analyzeRightEdgeScrollbarPixels(decoded, options = {}) {
  const bottomGapEpsilonPx = clampNumber(options.bottomGapEpsilonPx, DEFAULT_BOTTOM_GAP_EPSILON_PX);
  if (!decoded || !decoded.width || !decoded.height || !Array.isArray(decoded.rows)) {
    return {
      status: 'unavailable',
      reason: 'right_edge_crop_unavailable',
      method: 'right_edge_crop_pixel_scan',
      alignment: { status: 'unavailable' },
    };
  }
  const rows = Array.from({ length: decoded.height }, (_, y) => rowHasScrollbarPixels(decoded, y, options));
  const allSegments = segmentsFromRows(rows);
  const minThumbHeight = Math.max(24, Math.round(decoded.height * 0.05));
  const thumbSegments = allSegments.filter(segment => segment.height >= minThumbHeight);
  const thumb = thumbSegments.sort((left, right) => right.height - left.height)[0];
  if (!thumb) {
    return {
      status: 'unavailable',
      reason: 'scrollbar_thumb_not_found_in_right_edge_crop',
      method: 'right_edge_crop_pixel_scan',
      crop: { width: decoded.width, height: decoded.height },
      segments: allSegments.slice(0, 8),
      alignment: { status: 'unavailable' },
    };
  }

  const artifact = clippedContentArtifact(decoded, thumb, options);
  if (artifact) {
    return {
      status: 'unavailable',
      reason: 'right_edge_crop_contains_clipped_content',
      method: 'right_edge_crop_pixel_scan',
      crop: { width: decoded.width, height: decoded.height },
      artifact,
      segments: allSegments.slice(0, 8),
      alignment: { status: 'unavailable' },
    };
  }

  const bottomCap = allSegments
    .filter(segment => segment.start > thumb.end && segment.height <= 20)
    .sort((left, right) => right.start - left.start)[0];
  const trackBottomPx = bottomCap ? bottomCap.start - 1 : decoded.height - 1;
  const thumbBottomGapPx = Math.max(0, trackBottomPx - thumb.end);
  const alignmentStatus = thumbBottomGapPx <= bottomGapEpsilonPx
    ? 'bottom_aligned'
    : 'bottom_gap_exceeds_epsilon';
  return {
    status: alignmentStatus === 'bottom_aligned'
      ? 'right_edge_scrollbar_at_bottom'
      : 'right_edge_scrollbar_not_at_bottom',
    reason: alignmentStatus === 'bottom_aligned' ? undefined : 'right_edge_scrollbar_thumb_bottom_gap',
    method: 'right_edge_crop_pixel_scan',
    crop: { width: decoded.width, height: decoded.height },
    track: {
      topPx: 0,
      bottomPx: trackBottomPx,
      bottomCapStartPx: bottomCap?.start,
      heightPx: trackBottomPx + 1,
    },
    thumb: {
      topPx: thumb.start,
      bottomPx: thumb.end,
      heightPx: thumb.height,
    },
    alignment: {
      status: alignmentStatus,
      thumbBottomGapPx,
      allowedBottomGapPx: bottomGapEpsilonPx,
    },
    segments: allSegments.slice(0, 8),
  };
}

export async function analyzeRightEdgeScrollbarCrop(path, options = {}) {
  try {
    const decoded = decodePng(await readFile(path));
    return analyzeRightEdgeScrollbarPixels(decoded, options);
  } catch (error) {
    return {
      status: 'unavailable',
      reason: 'right_edge_crop_pixel_decode_failed',
      message: error instanceof Error ? error.message : String(error),
      method: 'right_edge_crop_pixel_scan',
      alignment: { status: 'unavailable' },
    };
  }
}
