//  Copyright 2026 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import type { ExecuteResult, SubstateValue } from "@tari-project/ootle-ts-bindings";
import initOotleWasm, {
  decodeExecuteResult as decodeExecuteResultJson,
  decodeSubstateValue as decodeSubstateValueJson,
} from "../../ootle-wasm/ootle_wasm.js";

// The indexer serves engine values in their CBOR encoding, base64 in JSON, and ootle-wasm decodes them. A value that
// cannot be decoded, because the module failed to load or the bytes are not a value, decodes to `undefined`, and
// callers show it encoded.

let loaded = false;

/** Loads the decoder. Resolves whether or not it loads. */
export async function loadDecoder(): Promise<void> {
  try {
    await initOotleWasm();
    loaded = true;
  } catch (error) {
    console.error("Could not load ootle-wasm; values are shown in their CBOR encoding", error);
  }
}

export function decodeSubstateValue(encoded: string): SubstateValue | undefined {
  return decode(encoded, decodeSubstateValueJson);
}

export function decodeExecuteResult(encoded: string): ExecuteResult | undefined {
  return decode(encoded, decodeExecuteResultJson);
}

function decode<T>(encoded: string, decoder: (encoded: string) => string): T | undefined {
  if (!loaded) {
    return undefined;
  }
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
