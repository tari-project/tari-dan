//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import { Line } from "./logFormat";

/** Bytes fetched per request. The daemon trims each window to whole lines. */
export const CHUNK_BYTES = 256 * 1024;
/** Chunks held at once, which bounds the rendered DOM once the level filters are letting lines through. */
export const MAX_CHUNKS = 6;
/**
 * Chunks the buffer may grow to while nothing in it passes the filters.
 *
 * A node log can run for megabytes without a single line above DEBUG. Every one of those chunks renders nothing,
 * so `MAX_CHUNKS` cannot evict against them - it would throw away the one chunk the reader can see - and the
 * search for a matching line has to stop somewhere instead of walking the whole file.
 */
export const MAX_SCAN_CHUNKS = 32;

/** Newest chunk first, matching the rendered order. */
export interface Chunk {
  start: number;
  end: number;
  lines: Line[];
  /** Grouped once on arrival: the trim policy counts rendered entries on every load. */
  entries: Line[][];
}

/**
 * Bounds the buffer without emptying the view.
 *
 * Chunks are dropped newest-first as the reader pages backwards, but never past the point where nothing would be
 * left to render: a run of chunks that the filters hide entirely contributes no rendered lines, so evicting
 * against it would blank the screen and strand the reader on a file they were reading a moment ago.
 */
export function trimBuffer(chunks: Chunk[], countVisible: (chunks: Chunk[]) => number): Chunk[] {
  let out = chunks;
  while (out.length > MAX_CHUNKS && countVisible(out.slice(1)) > 0) {
    out = out.slice(1);
  }
  return out;
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
