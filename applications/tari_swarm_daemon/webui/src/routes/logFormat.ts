//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

export const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;
export type Level = (typeof LEVELS)[number];

export interface Line {
  /** Byte offset of the line in the file: unique across chunks and stable when follow mode refetches the tail. */
  n: number;
  level: Level | null;
  text: string;
}

/** log4rs writes the level bare; tracing writes it after an RFC 3339 timestamp. Both are matched here. */
const LEVEL_RE = /\b(ERROR|WARN|INFO|DEBUG|TRACE)\b/;
export const TIME_RE = /^(\d{4}-\d{2}-\d{2}[T ]\d+:\d{2}:\d{2}[.\d]*Z?)/;
export const TARGET_RE = /\[([\w:]+::[\w:]+)\]/;
/**
 * A tracing line is `<timestamp> <LEVEL> [<spans>: ]<target>: <message>`, with no brackets around the target.
 * Rewriting it into log4rs' shape lets one renderer highlight both.
 */
const TRACING_RE = /^(\d{4}-\d{2}-\d{2}T[\d:.]+Z)\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+([\s\S]*)$/;
const TRACING_TARGET_RE = /^([\w:]+::[\w:]+):\s([\s\S]*)$/;
const TARGET_SEARCH_RE = /[\w:]+::[\w:]+:\s/;
const ANSI_RE = /\u001b\[[0-9;]*m/g;

const ENCODER = new TextEncoder();

/**
 * Normalises one raw line to `<timestamp> [<target>] <LEVEL> <message>`.
 *
 * The indexer logs through `tracing`, which orders the fields differently from log4rs and, in files written before
 * ANSI output was turned off, wraps every one of them in colour escapes.
 */
export function normalise(raw: string): string {
  const text = raw.replace(ANSI_RE, "");
  const tracing = TRACING_RE.exec(text);
  if (!tracing) {
    return text;
  }

  const [, time, level, rest] = tracing;
  // Anything ahead of the target is span context, e.g. `request{method=POST uri=/x}:`.
  const at = rest.search(TARGET_SEARCH_RE);
  const spans = at > 0 ? rest.slice(0, at).replace(/:\s*$/, "").trim() : "";
  const targeted = TRACING_TARGET_RE.exec(at > 0 ? rest.slice(at) : rest);
  if (!targeted) {
    return `${time} ${level.padEnd(5)} ${rest}`;
  }

  const [, target, message] = targeted;
  return `${time} [${target}] ${level.padEnd(5)} ${spans ? `${spans} ` : ""}${message}`;
}

/** Splits a chunk of file bytes into lines, keyed by their byte offset in the file. */
export function parse(body: string, start: number): Line[] {
  const raw = body.split("\n");
  // A chunk ends on a newline, so the split leaves a trailing empty element that is not a line of the file.
  if (raw.length && raw[raw.length - 1] === "") {
    raw.pop();
  }

  const lines: Line[] = [];
  let offset = start;
  for (const text of raw) {
    const normalised = normalise(text);
    const match = LEVEL_RE.exec(normalised);
    lines.push({ n: offset, level: (match?.[1] as Level) ?? null, text: normalised });
    offset += ENCODER.encode(text).length + 1;
  }
  return lines;
}
