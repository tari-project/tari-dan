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
import CheckBox from "@mui/material/Checkbox";
import TextField from "@mui/material/TextField";
import Dialog from "@mui/material/Dialog";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import FormControlLabel from "@mui/material/FormControlLabel";
import Box from "@mui/material/Box";
import { useAccountsList, useAccountsTransfer } from "../../../api/hooks/useAccounts";
import { useTheme } from "@mui/material/styles";
import useAccountStore from "../../../store/accountStore";
import { FormControl, InputLabel, MenuItem, Select, SelectChangeEvent } from "@mui/material";
import { useKeysList } from "../../../api/hooks/useKeys";
import { validatorsClaimFees } from "../../../utils/json_rpc";
import type { AccountInfo } from "@tariproject/typescript-bindings/wallet-daemon-client";

export default function ClaimFees() {
  const [open, setOpen] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [estimatedFee, setEstimatedFee] = useState(0);
  const [claimFeesFormState, setClaimFeesFormState] = useState({
    account: "",
    fee: "",
    validatorNodePublicKey: "",
    epoch: "",
    key_index: -1,
  });

  const { data: dataAccountsList } = useAccountsList(0, 10);
  const { data: dataKeysList } = useKeysList();
  const { setPopup } = useAccountStore();

  const theme = useTheme();

  const isFormFilled = () => {
    if (claimFeesFormState.validatorNodePublicKey.length !== 64) {
      return false;
    }
    return (
      claimFeesFormState.account !== "" &&
      claimFeesFormState.validatorNodePublicKey !== "" &&
      claimFeesFormState.epoch !== ""
    );
  };

  const is_filled = isFormFilled();

  const onClaimFeesKeyChange = (e: SelectChangeEvent<number>) => {
    if (!dataKeysList) {
      return;
    }
    let key_index = +e.target.value;
    let account = claimFeesFormState.account;
    let selected_account = dataAccountsList?.accounts.find(
      (account: AccountInfo) => account.account.key_index === key_index,
    );
    let new_account_name;
    account = selected_account?.account.name || "";
    new_account_name = false;
    setClaimFeesFormState({
      ...claimFeesFormState,
      key_index: key_index,
      account: account,
    });
  };

  const onEpochChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (/^[0-9]*$/.test(e.target.value)) {
      setEstimatedFee(0);
      setClaimFeesFormState({
        ...claimFeesFormState,
        [e.target.name]: e.target.value,
      });
    }
  };

  const onClaimBurnAccountNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setEstimatedFee(0);
    setClaimFeesFormState({
      ...claimFeesFormState,
      [e.target.name]: e.target.value,
    });
  };

  const onPublicKeyChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (/^[0-9a-fA-F]*$/.test(e.target.value)) {
      setClaimFeesFormState({
        ...claimFeesFormState,
        [e.target.name]: e.target.value,
      });
    }
    setEstimatedFee(0);
  };

  const onClaim = async () => {
    if (claimFeesFormState.account) {
      setDisabled(true);
      validatorsClaimFees({
        account: { Name: claimFeesFormState.account },
        max_fee: 3000,
        validator_public_key: claimFeesFormState.validatorNodePublicKey,
        epoch: parseInt(claimFeesFormState.epoch),
        dry_run: estimatedFee == 0,
      })
        .then((resp) => {
          if (estimatedFee == 0) {
            setEstimatedFee(resp.fee);
          } else {
            setEstimatedFee(0);
            setOpen(false);
            setPopup({ title: "Claim successful", error: false });
            setClaimFeesFormState({
              account: "",
              fee: "",
              validatorNodePublicKey: "",
              epoch: "",
              key_index: -1,
            });
          }
        })
        .catch((e) => {
          setPopup({ title: "Claim failed", error: true, message: e.message });
        })
        .finally(() => {
          setDisabled(false);
        });
    }
  };

  const handleClickOpen = () => {
    setOpen(true);
  };

  const handleClose = () => {
    setOpen(false);
  };

  const formattedKey = (key: [number, string, boolean]) => {
    let account = dataAccountsList?.accounts.find((account: AccountInfo) => account.account.key_index === +key[0]);
    if (account === undefined) {
      return null;
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
        Claim Fees
      </Button>
      <Dialog open={open} onClose={handleClose}>
        <DialogTitle>Claim Fees</DialogTitle>
        <DialogContent className="dialog-content">
          <Form onSubmit={onClaim} className="flex-container-vertical" style={{ paddingTop: theme.spacing(1) }}>
            <FormControl>
              <InputLabel id="key">Key</InputLabel>
              <Select
                labelId="key"
                name="key"
                label="Key"
                value={claimFeesFormState.key_index >= 0 ? claimFeesFormState.key_index : ""}
                onChange={onClaimFeesKeyChange}
                style={{ flexGrow: 1, minWidth: "200px" }}
                disabled={disabled}
              >
                {dataKeysList?.keys
                  ?.filter(
                    (key: [number, string, boolean]) =>
                      dataAccountsList?.accounts.find(
                        (account: AccountInfo) => account.account.key_index === +key[0],
                      ) !== undefined,
                  )
                  .map((key: [number, string, boolean]) => (
                    <MenuItem key={key[0]} value={key[0]}>
                      {formattedKey(key)}
                    </MenuItem>
                  ))}
              </Select>
            </FormControl>
            <TextField
              name="account"
              label="Account Name"
              value={claimFeesFormState.account}
              onChange={onClaimBurnAccountNameChange}
              style={{ flexGrow: 1 }}
              disabled={true}
            ></TextField>
            <TextField
              name="fee"
              label="Fee"
              value={estimatedFee || "Press fee estimate to calculate"}
              style={{ flexGrow: 1 }}
              InputProps={{ readOnly: true }}
              disabled={disabled}
            />
            <TextField
              name="validatorNodePublicKey"
              label="Validator Node Public Key"
              value={claimFeesFormState.validatorNodePublicKey}
              onChange={onPublicKeyChange}
              style={{ flexGrow: 1 }}
              disabled={disabled}
            />
            <TextField
              name="epoch"
              label="Epoch"
              value={claimFeesFormState.epoch}
              style={{ flexGrow: 1 }}
              onChange={onEpochChange}
              disabled={disabled}
            />
            <Box
              className="flex-container"
              style={{
                justifyContent: "flex-end",
              }}
            >
              <Button variant="outlined" onClick={handleClose} disabled={disabled}>
                Cancel
              </Button>
              <Button variant="contained" type="submit" disabled={disabled || !is_filled}>
                {estimatedFee ? "Claim" : "Estimate fee"}
              </Button>
            </Box>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
