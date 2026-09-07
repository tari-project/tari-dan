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

/**
 * Groups lines into whole log entries.
 *
 * The view shows the newest entry first, and reversing bare lines to get there would turn every multi-line
 * entry - a record whose message contains newlines, a panic and its backtrace - upside down. Reversing entries
 * instead keeps each one readable top to bottom.
 *
 * A timestamped file starts a new entry at each timestamp. Raw stdout and stderr carry no timestamps, so each
 * line stands alone there, except inside a panic block: that runs from the `panicked at` header to the next
 * blank line, which is the one multi-line shape those files reliably contain.
 */
export function toEntries(lines: Line[]): Line[][] {
  const timestamped = lines.some((line) => TIME_RE.test(line.text));
  const entries: Line[][] = [];
  let inPanic = false;

  for (const line of lines) {
    const startsRecord = TIME_RE.test(line.text);
    if (startsRecord) {
      inPanic = false;
    } else if (!timestamped) {
      if (inPanic && line.text.trim() === "") {
        inPanic = false;
      } else if (!inPanic && line.text.includes("panicked at")) {
        inPanic = true;
        entries.push([line]);
        continue;
      }
    }

    const continues = !startsRecord && (timestamped ? entries.length > 0 : inPanic);
    if (continues) {
      entries[entries.length - 1].push(line);
    } else {
      entries.push([line]);
    }
  }

  return entries;
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
