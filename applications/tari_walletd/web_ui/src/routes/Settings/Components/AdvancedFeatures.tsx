// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Checkbox from "@mui/material/Checkbox";
import FormControlLabel from "@mui/material/FormControlLabel";
import useSettingsStore from "@store/settingsStore";
import { AdvancedUiFeatures } from "@tari-project/ootle-ts-bindings";
import { settingsGet, settingsSet } from "@utils/json_rpc";
import { useEffect } from "react";

function AdvancedFeatures() {
  const { advancedUiFeatures, setAdvancedUiFeatures } = useSettingsStore();

  useEffect(() => {
    settingsGet().then((res) => {
      setAdvancedUiFeatures(res.advanced_ui_features);
    });
  }, [setAdvancedUiFeatures]);

  const setFeature = (feature: keyof AdvancedUiFeatures) => (e: React.ChangeEvent<HTMLInputElement>) => {
    const updated = { ...advancedUiFeatures, [feature]: e.target.checked };
    setAdvancedUiFeatures(updated);
    settingsSet({ indexer_url: null, indexer_urls: null, advanced_ui_features: updated, claimed_accounts: null });
  };

  return (
    <Box>
      <Alert severity="warning" style={{ marginBottom: 16 }}>
        These features are experimental. No support or documentation is provided. Use at your own risk.
      </Alert>
      <FormControlLabel
        control={<Checkbox checked={advancedUiFeatures.enable_manifest} onChange={setFeature("enable_manifest")} />}
        label="Enable manifest transaction builder"
      />
      <FormControlLabel
        control={
          <Checkbox checked={advancedUiFeatures.enable_validator_set} onChange={setFeature("enable_validator_set")} />
        }
        label="Show the validator set page"
      />
    </Box>
  );
}

export default AdvancedFeatures;
