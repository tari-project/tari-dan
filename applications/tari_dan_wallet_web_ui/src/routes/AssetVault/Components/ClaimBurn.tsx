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

import { useState } from "react";
import { Form } from "react-router-dom";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import Dialog from "@mui/material/Dialog";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import FormControl from "@mui/material/FormControl";
import InputLabel from "@mui/material/InputLabel";
import Select, { SelectChangeEvent } from "@mui/material/Select/Select";
import MenuItem from "@mui/material/MenuItem";
import Box from "@mui/material/Box";
import { useAccountsList, useAccountsClaimBurn } from "../../../api/hooks/useAccounts";
import { useTheme } from "@mui/material/styles";
import { accountsClaimBurn } from "../../../utils/json_rpc";
import useAccountStore from "../../../store/accountStore";
import { useKeysList } from "../../../api/hooks/useKeys";
import type { AccountInfo } from "@tariproject/typescript-bindings/wallet-daemon-client";

export default function ClaimBurn() {
  const [open, setOpen] = useState(false);
  const [claimBurnFormState, setClaimBurnFormState] = useState({
    account: "",
    key_index: -1,
    claimProof: "",
    fee: "",
    is_valid_json: false,
    newAccount: false,
    filled: false,
    disabled: false,
  });

  const { data: dataAccountsList } = useAccountsList(0, 10);
  const { data: dataKeysList } = useKeysList();
  const { setPopup } = useAccountStore();

  const onClaimBurnKeyChange = (e: SelectChangeEvent<number>) => {
    if (dataKeysList === undefined) {
      return;
    }
    let key_index = +e.target.value;
    let account = claimBurnFormState.account;
    if (dataAccountsList === undefined) {
      return;
    }
    let selected_account = dataAccountsList.accounts.find(
      (account: AccountInfo) => account.account.key_index === key_index,
    );
    let new_account_name;
    if (selected_account !== undefined) {
      account = selected_account.account.name || "";
      new_account_name = false;
    } else {
      if (claimBurnFormState.newAccount === false) {
        account = "";
      }
      new_account_name = true;
    }
    setClaimBurnFormState({
      ...claimBurnFormState,
      key_index: key_index,
      account: account,
      newAccount: new_account_name,
      filled: claimBurnFormState.is_valid_json && claimBurnFormState.fee !== "" && e.target.value !== "",
    });
  };

  const theme = useTheme();

  const onClaimBurnClaimProofChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    // We have to check if the claim proof is valid JSON
    try {
      JSON.parse(e.target.value);
      setClaimBurnFormState({
        ...claimBurnFormState,
        [e.target.name]: e.target.value,
        is_valid_json: true,
        filled: claimBurnFormState.key_index >= 0 && claimBurnFormState.fee !== "" && e.target.value !== "",
      });
    } catch {
      setClaimBurnFormState({
        ...claimBurnFormState,
        [e.target.name]: e.target.value,
        is_valid_json: false,
        filled: false,
      });
    }
  };

  const onClaimBurnAccountNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setClaimBurnFormState({
      ...claimBurnFormState,
      [e.target.name]: e.target.value,
      filled: claimBurnFormState.key_index >= 0 && claimBurnFormState.is_valid_json && e.target.value !== "",
    });
  };

  const onClaimBurnFeeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setClaimBurnFormState({
      ...claimBurnFormState,
      [e.target.name]: e.target.value,
      filled: claimBurnFormState.key_index >= 0 && claimBurnFormState.is_valid_json && e.target.value !== "",
    });
  };

  const onClaimBurn = async () => {
    try {
      setClaimBurnFormState({ ...claimBurnFormState, disabled: true });
      await accountsClaimBurn({
        account: { Name: claimBurnFormState.account },
        claim_proof: JSON.parse(claimBurnFormState.claimProof),
        max_fee: +claimBurnFormState.fee,
        key_id: +claimBurnFormState.key_index,
      });
      setOpen(false);
      setPopup({ title: "Claimed", error: false });
      setClaimBurnFormState({
        key_index: -1,
        account: "",
        claimProof: "",
        fee: "",
        is_valid_json: false,
        filled: false,
        disabled: false,
        newAccount: false,
      });
    } catch (e: any) {
      setClaimBurnFormState({ ...claimBurnFormState, disabled: false });
      setPopup({ title: "Claim burn failed: " + e.message, error: true });
    }
  };

  const handleClickOpen = () => {
    setClaimBurnFormState({ ...claimBurnFormState, disabled: false });
    setOpen(true);
  };

  const handleClose = () => {
    setOpen(false);
  };

  const formattedKey = (key: [number, string, boolean]) => {
    let account = dataAccountsList?.accounts.find((account: AccountInfo) => account.account.key_index === +key[0]);
    if (account === undefined) {
      return (
        <div>
          <b>{key[0]}</b> {key[1]}
        </div>
      );
    }
    return (
      <div>
        <b>{key[0]}</b> {key[1]}
        <br></br>Account <i>{account.account.name}</i>
      </div>
    );
  };

  return (
    <div>
      <Button variant="outlined" onClick={handleClickOpen}>
        Claim Burn
      </Button>
      <Dialog open={open} onClose={handleClose}>
        <DialogTitle>Claim Burn</DialogTitle>
        <DialogContent className="dialog-content">
          <Form onSubmit={onClaimBurn} className="flex-container-vertical" style={{ paddingTop: theme.spacing(1) }}>
            <FormControl>
              <InputLabel id="key">Key</InputLabel>
              <Select
                labelId="key"
                name="key"
                label="Key"
                value={claimBurnFormState.key_index >= 0 ? claimBurnFormState.key_index : ""}
                onChange={onClaimBurnKeyChange}
                style={{ flexGrow: 1, minWidth: "200px" }}
                disabled={claimBurnFormState.disabled}
              >
                {dataKeysList?.keys?.map((key: [number, string, boolean]) => (
                  <MenuItem key={key[0]} value={key[0]}>
                    {formattedKey(key)}
                  </MenuItem>
                ))}
              </Select>
            </FormControl>
            <TextField
              name="account"
              label="Account Name"
              value={claimBurnFormState.account}
              onChange={onClaimBurnAccountNameChange}
              style={{ flexGrow: 1 }}
              disabled={claimBurnFormState.disabled || !claimBurnFormState.newAccount}
            ></TextField>
            <TextField
              name="claimProof"
              label="Claim Proof"
              value={claimBurnFormState.claimProof}
              onChange={onClaimBurnClaimProofChange}
              style={{ flexGrow: 1 }}
              disabled={claimBurnFormState.disabled}
            />
            <TextField
              name="fee"
              label="Fee"
              value={claimBurnFormState.fee}
              onChange={onClaimBurnFeeChange}
              style={{ flexGrow: 1 }}
              disabled={claimBurnFormState.disabled}
            />
            <Box
              className="flex-container"
              style={{
                justifyContent: "flex-end",
              }}
            >
              <Button variant="outlined" onClick={handleClose} disabled={claimBurnFormState.disabled}>
                Cancel
              </Button>
              <Button
                variant="contained"
                type="submit"
                disabled={!claimBurnFormState.filled || claimBurnFormState.disabled}
              >
                Claim Burn
              </Button>
            </Box>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
