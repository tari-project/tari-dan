//  Copyright 2026 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import type { Event, ExecuteResult, SubstateValue, TransactionReceipt } from "@tari-project/ootle-ts-bindings";
import { useSyncExternalStore } from "react";
import initOotleWasm, {
  decodeEvent as decodeEventJson,
  decodeExecuteResult as decodeExecuteResultJson,
  decodeSubstateValue as decodeSubstateValueJson,
  decodeTransactionReceipt as decodeTransactionReceiptJson,
} from "../../ootle-wasm-bor/ootle_wasm_bor.js";

// The indexer serves engine values in their CBOR encoding, base64 in JSON, and ootle-wasm-bor decodes them. A value that
// cannot be decoded, because the module has not loaded (or failed to) or the bytes are not a value, decodes to
// `undefined`, and callers show it encoded.

export interface Decoder {
  decodeSubstateValue(encoded: string): SubstateValue | undefined;
  decodeExecuteResult(encoded: string): ExecuteResult | undefined;
  decodeTransactionReceipt(encoded: string): TransactionReceipt | undefined;
  decodeEvent(encoded: string): Event | undefined;
}

const LOADED: Decoder = {
  decodeSubstateValue: (encoded) => decode(encoded, decodeSubstateValueJson),
  decodeExecuteResult: (encoded) => decode(encoded, decodeExecuteResultJson),
  decodeTransactionReceipt: (encoded) => decode(encoded, decodeTransactionReceiptJson),
  decodeEvent: (encoded) => decode(encoded, decodeEventJson),
};

const UNLOADED: Decoder = {
  decodeSubstateValue: () => undefined,
  decodeExecuteResult: () => undefined,
  decodeTransactionReceipt: () => undefined,
  decodeEvent: () => undefined,
};

let loaded = false;
const listeners = new Set<() => void>();

/** Loads the decoder. Resolves whether or not it loads. */
export async function loadDecoder(): Promise<void> {
  try {
    await initOotleWasm();
    loaded = true;
    listeners.forEach((listener) => listener());
  } catch (error) {
    console.error("Could not load ootle-wasm-bor; values are shown in their CBOR encoding", error);
  }
}

/** The decoder. The caller re-renders, with a new `Decoder`, once the module loads. */
export function useDecoder(): Decoder {
  return useSyncExternalStore(subscribe, () => (loaded ? LOADED : UNLOADED));
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function decode<T>(encoded: string, decoder: (encoded: string) => string): T | undefined {
  try {
    return JSON.parse(decoder(encoded)) as T;
  } catch (error) {
    console.warn("Could not decode a value the indexer served", error);
    return undefined;
  }
}

/** The number of bytes `encoded` (base64) holds. */
export function encodedByteLength(encoded: string): number {
  const padding = encoded.endsWith("==") ? 2 : encoded.endsWith("=") ? 1 : 0;
  return Math.floor((encoded.length * 3) / 4) - padding;
}
