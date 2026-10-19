// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import { AdvancedUiFeatures } from "@tari-project/ootle-ts-bindings";
import { create } from "zustand";

interface Store {
  advancedUiFeatures: AdvancedUiFeatures;
  setAdvancedUiFeatures: (features: AdvancedUiFeatures) => void;
}

const useSettingsStore = create<Store>()((set) => ({
  advancedUiFeatures: { enable_manifest: false, enable_validator_set: false },
  setAdvancedUiFeatures: (features) => set({ advancedUiFeatures: features }),
}));

export default useSettingsStore;
