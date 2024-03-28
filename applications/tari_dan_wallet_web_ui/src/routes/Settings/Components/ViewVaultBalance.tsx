//  Copyright 2022. The Tari Project
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

import Typography from "@mui/material/Typography";
import TextField from "@mui/material/TextField";
import { useState } from "react";
import Button from "@mui/material/Button";
import Box from "@mui/material/Box";
import { useTheme } from "@mui/material/styles";
import { Divider } from "@mui/material";
import { confidentialViewVaultBalance } from "../../../utils/json_rpc";
import { ConfidentialViewVaultBalanceRequest } from "@tariproject/typescript-bindings/wallet-daemon-client";

function ViewVaultBalanceForm() {
  const [formState, setFormState] = useState({
    vaultId: null,
    keyId: 0,
  });
  const [vaultBalance, setVaultBalance] = useState<any>(null);

  const onViewBalanceClicked = async () => {
    const resp = await confidentialViewVaultBalance({
      vault_id: formState.vaultId!,
      minimum_expected_value: null,
      maximum_expected_value: null,
      view_key_id: formState.keyId,
    } as ConfidentialViewVaultBalanceRequest);

    setVaultBalance(resp);
  };

  const balances =
    vaultBalance &&
    Object.keys(vaultBalance?.balances).map((key) => {
      return (
        <Box key={key}>
          <Typography>
            {key}: {vaultBalance.balances[key] || "Failed not decrypt value"}
          </Typography>
        </Box>
      );
    });

  const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFormState({
      ...formState,
      [e.target.name]: e.target.value,
    });
  };

  return (
    <>
      <Box className="flex-container" sx={{ marginBottom: 4 }}>
        <TextField name="keyId" label="Key ID" value={formState.keyId} onChange={onChange} style={{ flexGrow: 1 }} />
        <TextField
          name="vaultId"
          label="Vault Id"
          value={formState.vaultId}
          onChange={onChange}
          style={{ flexGrow: 1 }}
        />
        <Button variant="contained" onClick={onViewBalanceClicked} disabled={!formState.vaultId}>
          Fetch Balance
        </Button>
      </Box>
      {balances && (
        <>
          <Typography variant="h3">Balances</Typography>
          {balances}
        </>
      )}
    </>
  );
}

function ViewVaultBalance() {
  const theme = useTheme();
  return (
    <Box
      style={{
        display: "flex",
        flexDirection: "column",
        gap: theme.spacing(3),
        paddingTop: theme.spacing(3),
      }}
    >
      <p>Brute force a vault balance using a secret view key</p>
      <Box>
        <ViewVaultBalanceForm />
      </Box>
      <Divider />
    </Box>
  );
}

export default ViewVaultBalance;
