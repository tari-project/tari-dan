//  Copyright 2025. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

import type { KeyId } from "@tari-project/ootle-ts-bindings";
import { create } from "zustand";
import { persist } from "zustand/middleware";

const DEFAULT_CODE = `
// use template_xxx as TemplateName;

fn main() {
   // TemplateName::call_something();
   // let account = var!["account"];
   // let bucket = account.withdraw(1000);
}`;

export interface ManifestTab {
  id: string;
  name: string;
  code: string;
  variables: Record<string, string>;
  signingKeys: KeyId[];
  /**
   * Blob payloads referenced from the manifest via `blob!(name)`. Keys are blob names; values
   * are base64-encoded byte payloads (matching the Blob TS binding).
   */
  blobs: Record<string, string>;
}

interface Store {
  tabs: ManifestTab[];
  activeTabId: string;

  // Active tab convenience accessors
  code: string;
  variables: Record<string, string>;
  signingKeys: KeyId[];
  blobs: Record<string, string>;

  // Tab management
  addTab: () => void;
  removeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  renameTab: (id: string, name: string) => void;

  // Active tab code/variable operations
  setCode: (code: string) => void;
  addVariable: (key: string, value: string) => void;
  removeVariable: (key: string) => void;
  renameVariable: (oldKey: string, newKey: string) => void;
  addSigningKey: (key: KeyId) => void;
  removeSigningKey: (index: number) => void;
  /** Add or update a blob payload under `name`. `base64` is base64-encoded bytes. */
  setBlob: (name: string, base64: string) => void;
  removeBlob: (name: string) => void;
  loadTabs: (tabs: ManifestTab[]) => void;
}

let nextId = 1;
function generateId(): string {
  return `tab_${Date.now()}_${nextId++}`;
}

function createTab(name: string): ManifestTab {
  return { id: generateId(), name, code: DEFAULT_CODE, variables: {}, signingKeys: [], blobs: {} };
}

function nextTabName(tabs: ManifestTab[]): string {
  let i = 1;
  const names = new Set(tabs.map((t) => t.name));
  while (names.has(`Manifest ${i}`)) i++;
  return `Manifest ${i}`;
}

function updateActiveTab(state: Store, patch: Partial<ManifestTab>): Partial<Store> {
  const tabs = state.tabs.map((t) => (t.id === state.activeTabId ? { ...t, ...patch } : t));
  const active = tabs.find((t) => t.id === state.activeTabId)!;
  return {
    tabs,
    code: active.code,
    variables: active.variables,
    signingKeys: active.signingKeys,
    blobs: active.blobs,
  };
}

const defaultTab = createTab("Manifest 1");

const useManifestCodeStore = create<Store>()(
  persist<Store>(
    (set) => ({
      tabs: [defaultTab],
      activeTabId: defaultTab.id,
      code: defaultTab.code,
      variables: defaultTab.variables,
      signingKeys: defaultTab.signingKeys,
      blobs: defaultTab.blobs,

      addTab: () =>
        set((state) => {
          const tab = createTab(nextTabName(state.tabs));
          return {
            tabs: [...state.tabs, tab],
            activeTabId: tab.id,
            code: tab.code,
            variables: tab.variables,
            signingKeys: tab.signingKeys,
            blobs: tab.blobs,
          };
        }),

      removeTab: (id: string) =>
        set((state) => {
          if (state.tabs.length <= 1) return state;
          const idx = state.tabs.findIndex((t) => t.id === id);
          const tabs = state.tabs.filter((t) => t.id !== id);
          let activeTabId = state.activeTabId;
          if (activeTabId === id) {
            const newIdx = Math.min(idx, tabs.length - 1);
            activeTabId = tabs[newIdx].id;
          }
          const active = tabs.find((t) => t.id === activeTabId)!;
          return {
            tabs,
            activeTabId,
            code: active.code,
            variables: active.variables,
            signingKeys: active.signingKeys,
            blobs: active.blobs,
          };
        }),

      setActiveTab: (id: string) =>
        set((state) => {
          const tab = state.tabs.find((t) => t.id === id);
          if (!tab) return state;
          return {
            activeTabId: id,
            code: tab.code,
            variables: tab.variables,
            signingKeys: tab.signingKeys,
            blobs: tab.blobs,
          };
        }),

      renameTab: (id: string, name: string) =>
        set((state) => ({
          tabs: state.tabs.map((t) => (t.id === id ? { ...t, name } : t)),
        })),

      setCode: (code: string) => set((state) => updateActiveTab(state, { code })),

      addVariable: (key: string, value: string) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          return updateActiveTab(state, { variables: { ...active.variables, [key]: value } });
        }),

      removeVariable: (key: string) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          const { [key]: _, ...rest } = active.variables;
          return updateActiveTab(state, { variables: rest });
        }),

      renameVariable: (oldKey: string, newKey: string) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          if (oldKey === newKey || !(oldKey in active.variables)) return state;
          const entries = Object.entries(active.variables).map(([k, v]) => (k === oldKey ? [newKey, v] : [k, v]));
          return updateActiveTab(state, { variables: Object.fromEntries(entries) });
        }),

      addSigningKey: (key: KeyId) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          // Avoid duplicates
          const isDuplicate = active.signingKeys.some((k) => JSON.stringify(k) === JSON.stringify(key));
          if (isDuplicate) return state;
          return updateActiveTab(state, { signingKeys: [...active.signingKeys, key] });
        }),

      removeSigningKey: (index: number) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          return updateActiveTab(state, { signingKeys: active.signingKeys.filter((_, i) => i !== index) });
        }),

      setBlob: (name: string, base64: string) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          return updateActiveTab(state, { blobs: { ...active.blobs, [name]: base64 } });
        }),

      removeBlob: (name: string) =>
        set((state) => {
          const active = state.tabs.find((t) => t.id === state.activeTabId)!;
          const { [name]: _removed, ...rest } = active.blobs;
          return updateActiveTab(state, { blobs: rest });
        }),

      loadTabs: (loaded: ManifestTab[]) =>
        set(() => {
          // Assign fresh IDs to avoid collisions with existing tabs
          const tabs = loaded.map((t) => ({
            ...t,
            id: generateId(),
            signingKeys: t.signingKeys || [],
            blobs: t.blobs || {},
          }));
          const first = tabs[0];
          return {
            tabs,
            activeTabId: first.id,
            code: first.code,
            variables: first.variables,
            signingKeys: first.signingKeys,
            blobs: first.blobs,
          };
        }),
    }),
    {
      name: "manifest-code",
      version: 3,
    },
  ),
);

export default useManifestCodeStore;
