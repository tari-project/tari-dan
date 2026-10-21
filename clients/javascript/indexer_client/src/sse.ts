/*
 * //  Copyright 2026 The Tari Project
 * //  SPDX-License-Identifier: BSD-3-Clause
 */

import { HttpError } from "./errors";

/**
 * A parsed Server-Sent Event.
 */
export interface SseEvent {
  event: string;
  data: string;
  id?: string;
  retry?: number;
}

/**
 * Callback-based SSE stream handle. Call `close()` to stop the stream.
 */
export interface SseStream {
  close(): void;
}

export interface SseStreamOptions {
  /** Called for each received SSE event */
  onEvent: (event: SseEvent) => void;
  /** Called when the stream ends or encounters an error */
  onError?: (error: Error) => void;
  /** Called when the stream closes cleanly */
  onClose?: () => void;
  /** AbortSignal to cancel the stream externally */
  signal?: AbortSignal;
}

/**
 * Longest SSE line accepted, in UTF-16 code units, excluding its terminator. Indexer events are a few KiB of
 * JSON, so this bounds the read buffer.
 */
const MAX_LINE_LENGTH = 1024 * 1024;

/** Most `data` accepted for one event, in UTF-16 code units, across all of its `data:` lines. */
const MAX_EVENT_DATA_LENGTH = 1024 * 1024;

/**
 * Connect to an SSE endpoint using fetch and parse the streaming response.
 * Returns an SseStream handle that can be closed.
 */
export function connectSse(url: string, options: SseStreamOptions): SseStream {
  const controller = new AbortController();

  // Link external signal if provided
  if (options.signal) {
    if (options.signal.aborted) {
      controller.abort();
    } else {
      options.signal.addEventListener("abort", () => controller.abort(), { once: true });
    }
  }

  const run = async () => {
    const response = await fetch(url, {
      method: "GET",
      headers: { Accept: "text/event-stream" },
      signal: controller.signal,
    });

    if (!response.ok) {
      const text = await response.text();
      throw new HttpError(response.status, response.statusText, text);
    }

    if (!response.body) {
      throw new Error("Response body is not readable");
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    // Splits on \r\n, \r, or \n per the SSE spec
    const lineEnding = /\r\n|\r|\n/g;
    // Pieces of the current line received so far, held apart so that each chunk is searched only once
    let pending: string[] = [];
    let pendingLength = 0;
    // The last chunk ended in "\r", so a "\n" opening the next chunk completes that terminator
    let afterCr = false;

    // Current event being built
    let eventType = "";
    let dataLines: string[] = [];
    let dataLength = 0;
    let eventId: string | undefined;
    let retry: number | undefined;

    const dispatch = () => {
      if (dataLines.length === 0) {
        // Reset but don't dispatch
        eventType = "";
        eventId = undefined;
        retry = undefined;
        return;
      }

      const event: SseEvent = {
        event: eventType || "message",
        data: dataLines.join("\n"),
        id: eventId,
        retry,
      };

      options.onEvent(event);

      // Reset for next event
      eventType = "";
      dataLines = [];
      dataLength = 0;
      eventId = undefined;
      retry = undefined;
    };

    const processLine = (line: string) => {
      if (line.length > MAX_LINE_LENGTH) {
        throw new Error(`SSE line exceeds the maximum length of ${MAX_LINE_LENGTH}`);
      }
      if (line === "") {
        // Empty line = end of event
        dispatch();
        return;
      }

      if (line.startsWith(":")) {
        // Comment, ignore
        return;
      }

      const colonIndex = line.indexOf(":");
      let field: string;
      let value: string;

      if (colonIndex === -1) {
        field = line;
        value = "";
      } else {
        field = line.slice(0, colonIndex);
        value = line.slice(colonIndex + 1);
        if (value.startsWith(" ")) {
          value = value.slice(1);
        }
      }

      switch (field) {
        case "event":
          eventType = value;
          break;
        case "data":
          // Each line after the first adds a "\n" separator
          dataLength += value.length + (dataLines.length > 0 ? 1 : 0);
          if (dataLength > MAX_EVENT_DATA_LENGTH) {
            throw new Error(`SSE event data exceeds the maximum length of ${MAX_EVENT_DATA_LENGTH}`);
          }
          dataLines.push(value);
          break;
        case "id":
          eventId = value;
          break;
        case "retry": {
          const parsed = parseInt(value, 10);
          if (!isNaN(parsed)) {
            retry = parsed;
          }
          break;
        }
        // Unknown fields are ignored per spec
      }
    };

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        let text = decoder.decode(value, { stream: true });
        if (afterCr && text.length > 0) {
          if (text.startsWith("\n")) {
            text = text.slice(1);
          }
          afterCr = false;
        }

        let lineStart = 0;
        lineEnding.lastIndex = 0;
        let match: RegExpExecArray | null;
        while ((match = lineEnding.exec(text)) !== null) {
          pending.push(text.slice(lineStart, match.index));
          const line = pending.join("");
          pending = [];
          pendingLength = 0;
          processLine(line);
          lineStart = lineEnding.lastIndex;
          afterCr = match[0] === "\r" && lineStart === text.length;
        }
        if (lineStart < text.length) {
          pending.push(text.slice(lineStart));
          pendingLength += text.length - lineStart;
          if (pendingLength > MAX_LINE_LENGTH) {
            throw new Error(`SSE line exceeds the maximum length of ${MAX_LINE_LENGTH}`);
          }
        }
      }

      // Process any remaining data
      pending.push(decoder.decode());
      const rest = pending.join("");
      if (rest.length > 0) {
        processLine(rest);
        dispatch();
      }
    } catch (err) {
      // Closes the connection; the caller learns of the failure from the rethrown error
      await reader.cancel().catch(() => {});
      throw err;
    } finally {
      reader.releaseLock();
    }
  };

  run()
    .then(() => options.onClose?.())
    .catch((err) => {
      if (controller.signal.aborted) {
        options.onClose?.();
      } else {
        options.onError?.(err instanceof Error ? err : new Error(String(err)));
      }
    });

  return {
    close() {
      controller.abort();
    },
  };
}
