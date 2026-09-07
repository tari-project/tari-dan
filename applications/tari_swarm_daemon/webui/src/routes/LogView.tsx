//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import {
  Fragment,
  ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useNavigate, useParams } from "react-router-dom";
import { describeError, swarmRpc } from "../api/rpc";
import { LEVELS, Level, Line, TARGET_RE, TIME_RE, parse, toEntries } from "./logFormat";

/** Bytes fetched per request. The daemon trims each window to whole lines. */
const CHUNK_BYTES = 256 * 1024;
/**
 * Chunks held at once. Paging further back drops the newest ones, so however long a session runs the buffer stays
 * bounded and the tab stays responsive.
 */
const MAX_CHUNKS = 6;
const FOLLOW_MS = 2000;
/** Distance from the older end of the view at which the next chunk starts loading. */
const LOAD_MARGIN_PX = 400;

interface Chunk {
  start: number;
  end: number;
  lines: Line[];
}

interface ChunkResponse {
  contents: string;
  start: number;
  end: number;
  file_size: number;
}

/** Highlights the timestamp, the log target and every search hit, without raw HTML. */
function renderText(text: string, needle: string): ReactNode {
  const parts: ReactNode[] = [];
  let rest = text;
  let key = 0;

  const time = TIME_RE.exec(rest);
  if (time) {
    parts.push(
      <span className="log-ts" key={key++}>
        {time[1]}
      </span>,
    );
    rest = rest.slice(time[1].length);
  }

  const target = TARGET_RE.exec(rest);
  let tail = rest;
  if (target && target.index >= 0) {
    parts.push(<Fragment key={key++}>{rest.slice(0, target.index)}</Fragment>);
    parts.push(
      <span className="log-target" key={key++}>
        [{target[1]}]
      </span>,
    );
    tail = rest.slice(target.index + target[0].length);
  }

  if (!needle) {
    parts.push(<Fragment key={key++}>{tail}</Fragment>);
    return parts;
  }

  const lower = tail.toLowerCase();
  const lowerNeedle = needle.toLowerCase();
  let from = 0;
  let at = lower.indexOf(lowerNeedle);
  while (at !== -1) {
    parts.push(<Fragment key={key++}>{tail.slice(from, at)}</Fragment>);
    parts.push(<mark key={key++}>{tail.slice(at, at + needle.length)}</mark>);
    from = at + needle.length;
    at = lower.indexOf(lowerNeedle, from);
  }
  parts.push(<Fragment key={key++}>{tail.slice(from)}</Fragment>);
  return parts;
}

async function fetchChunk(path: string, end: number | null): Promise<Chunk> {
  const res: ChunkResponse = await swarmRpc("get_file", { path, end, max_bytes: CHUNK_BYTES });
  return { start: res.start, end: res.end, lines: parse(res.contents, res.start) };
}

export default function LogView() {
  const navigate = useNavigate();
  const { name } = useParams<{ name: string; format: string }>();
  const path = useMemo(() => {
    try {
      return name ? atob(name) : null;
    } catch {
      return null;
    }
  }, [name]);

  // Newest chunk first, matching the rendered order.
  const [chunks, setChunks] = useState<Chunk[] | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);
  // A malformed link is a property of the URL, not something to store and sync.
  const error = path === null ? "That log link is not valid." : fetchError;
  const [hidden, setHidden] = useState<Set<Level>>(() => new Set(["DEBUG", "TRACE"] as Level[]));
  const [needle, setNeedle] = useState("");
  const [wrap, setWrap] = useState(true);
  const [follow, setFollow] = useState(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const view = useRef<HTMLDivElement>(null);
  const loadingRef = useRef(false);
  /**
   * Line to hold still across a buffer trim, which removes rendered lines above the viewport. `from` records the
   * oldest offset at capture time, so an anchor left behind by a load that never committed is discarded rather
   * than applied to some later, unrelated re-render.
   */
  const anchor = useRef<{ n: number; top: number; from: number } | null>(null);

  const atStartOfFile = chunks !== null && chunks[chunks.length - 1]?.start === 0;

  // Entering follow mode starts a fresh window at the tail; leaving it keeps whatever is on screen.
  useEffect(() => {
    if (!path || !follow) {
      return;
    }
    let cancelled = false;
    let timer: number | undefined;

    const load = async () => {
      try {
        const chunk = await fetchChunk(path, null);
        if (!cancelled) {
          setChunks([chunk]);
          setFetchError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setFetchError(describeError(err));
        }
      }
      if (!cancelled) {
        timer = window.setTimeout(load, FOLLOW_MS);
      }
    };

    void load();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [path, follow]);

  const loadOlder = useCallback(async () => {
    const oldest = chunks?.[chunks.length - 1];
    if (!path || !oldest || loadingRef.current || oldest.start === 0) {
      return;
    }
    loadingRef.current = true;
    setLoadingOlder(true);

    // The oldest rendered line survives any trim, so it can hold the view still if one happens.
    const rendered = view.current?.querySelectorAll<HTMLElement>(".logline");
    const oldestRendered = rendered?.length ? rendered[rendered.length - 1] : undefined;
    const anchorN = oldestRendered?.dataset.n;
    anchor.current =
      oldestRendered && anchorN !== undefined
        ? { n: Number(anchorN), top: oldestRendered.getBoundingClientRect().top, from: oldest.start }
        : null;

    try {
      const chunk = await fetchChunk(path, oldest.start);
      setChunks((current) => {
        // Follow mode may have replaced the buffer while the request was in flight.
        if (!current || current[current.length - 1]?.start !== oldest.start) {
          return current;
        }
        return [...current, chunk].slice(-MAX_CHUNKS);
      });
      setFetchError(null);
    } catch (err) {
      setFetchError(describeError(err));
    } finally {
      loadingRef.current = false;
      setLoadingOlder(false);
    }
  }, [path, chunks]);

  // Newest lines sit at the top, so scrolling down walks backwards through the file.
  const onScroll = () => {
    const el = view.current;
    if (!el) {
      return;
    }
    if (follow) {
      if (el.scrollTop > LOAD_MARGIN_PX) {
        setFollow(false);
      }
      return;
    }
    if (el.scrollHeight - el.scrollTop - el.clientHeight < LOAD_MARGIN_PX) {
      void loadOlder();
    }
  };

  // Newest entry first, but the lines inside each entry stay in the order they were written.
  const entries = useMemo(
    () => (chunks === null ? null : chunks.flatMap((chunk) => toEntries(chunk.lines).reverse())),
    [chunks],
  );

  const visible = useMemo(() => {
    if (entries === null) {
      return [];
    }
    const lowerNeedle = needle.trim().toLowerCase();
    // An entry's level is its opening line's - a continuation carries none of its own, and hiding a record has
    // to take its continuations with it.
    return entries
      .filter(
        (entry) =>
          !(entry[0].level && hidden.has(entry[0].level)) &&
          (!lowerNeedle || entry.some((line) => line.text.toLowerCase().includes(lowerNeedle))),
      )
      .flat();
  }, [entries, hidden, needle]);

  useLayoutEffect(() => {
    const el = view.current;
    if (follow) {
      el?.scrollTo({ top: 0 });
      anchor.current = null;
      return;
    }
    const held = anchor.current;
    anchor.current = null;
    if (!el || !held || chunks?.[chunks.length - 1]?.start === held.from) {
      return;
    }
    const moved = el.querySelector<HTMLElement>(`.logline[data-n="${held.n}"]`);
    if (moved) {
      el.scrollTop += moved.getBoundingClientRect().top - held.top;
    }
  }, [visible, follow, chunks]);

  // Level filters can leave too few lines to fill the view, which would strand it with nothing to scroll.
  useEffect(() => {
    const el = view.current;
    if (el && !follow && !loadingOlder && !atStartOfFile && el.scrollHeight <= el.clientHeight) {
      void loadOlder();
    }
  }, [visible, follow, loadingOlder, atStartOfFile, loadOlder]);

  const counts = useMemo(() => {
    const tally: Record<string, number> = {};
    for (const [line] of entries ?? []) {
      if (line.level) {
        tally[line.level] = (tally[line.level] ?? 0) + 1;
      }
    }
    return tally;
  }, [entries]);

  const toggle = (level: Level) =>
    setHidden((current) => {
      const next = new Set(current);
      if (next.has(level)) {
        next.delete(level);
      } else {
        next.add(level);
      }
      return next;
    });

  return (
    // Fills the page area, cancelling the page padding on all four sides.
    <div style={{ display: "flex", flexDirection: "column", height: "calc(100% + 36px)", margin: -18 }}>
      <div className="logbar">
        <button className="btn sm ghost" onClick={() => navigate(-1)}>
          ← Back
        </button>
        <span className="mono truncate grow" title={path ?? ""}>
          {path?.split("/").slice(-2).join("/") ?? "unknown file"}
        </span>

        {LEVELS.map((level) => (
          <button
            key={level}
            className={`btn sm${hidden.has(level) ? " ghost" : ""}`}
            onClick={() => toggle(level)}
            title={hidden.has(level) ? `Show ${level} lines` : `Hide ${level} lines`}
          >
            <span className={`lvl-${level}`} style={{ fontWeight: 800 }}>
              {level}
            </span>
            <span className="faint mono">{counts[level] ?? 0}</span>
          </button>
        ))}

        <input
          type="search"
          placeholder="Search"
          value={needle}
          onChange={(e) => setNeedle(e.target.value)}
          style={{ width: 200 }}
        />
        <button className={`btn sm${wrap ? " primary" : ""}`} onClick={() => setWrap(!wrap)}>
          Wrap
        </button>
        <button
          className={`btn sm${follow ? " primary" : ""}`}
          onClick={() => setFollow(!follow)}
          title={follow ? "Stop following the end of the file" : "Jump back to the newest lines"}
        >
          Follow
        </button>
      </div>

      <div className="logview" ref={view} onScroll={onScroll}>
        {error && <p className="empty">{error}</p>}
        {!error && entries === null && <p className="empty">Loading…</p>}
        {!error && entries !== null && !visible.length && (
          <p className="empty">{entries.length ? "No lines match the filters." : "This file is empty."}</p>
        )}
        {/* Paging back trims the newest chunks, so the head of the file is only reachable through Follow. */}
        {!error && !follow && visible.length > 0 && (
          <p className="logstart">Paused · Follow to jump back to the newest lines</p>
        )}
        {visible.map((line) => (
          <div className={`logline${wrap ? "" : " nowrap"}`} data-n={line.n} key={line.n}>
            <span className={`lvl lvl-${line.level ?? ""}`}>{line.level ?? ""}</span>
            <span className="txt">{renderText(line.text, needle.trim())}</span>
          </div>
        ))}
        {!error && visible.length > 0 && (
          <p className="logend">
            {loadingOlder
              ? "Loading older lines…"
              : atStartOfFile
                ? "Start of file"
                : "Scroll down for older lines"}
          </p>
        )}
      </div>
    </div>
  );
}
