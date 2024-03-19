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
import { useAccountsGetBalances, useAccountsTransfer } from "../../../api/hooks/useAccounts";
import { useTheme } from "@mui/material/styles";
import useAccountStore from "../../../store/accountStore";
import Select from "@mui/material/Select";
import { SelectChangeEvent } from "@mui/material/Select/Select";
import MenuItem from "@mui/material/MenuItem";

const XTR2 = "resource_01010101010101010101010101010101010101010101010101010101";

export default function SendMoney() {
  const [open, setOpen] = useState(false);


  return (
    <div>
      <Button variant="outlined" onClick={() => setOpen(true)}>
        Send Tari
      </Button>
      <SendMoneyDialog
        open={open}
        handleClose={() => setOpen(false)}
        onSendComplete={() => setOpen(false)}
        resource_address={XTR2}
      />
    </div>
  );
}

export interface SendMoneyDialogProps {
  open: boolean;
  resource_address: string;
  onSendComplete?: () => void;
  handleClose: () => void;
}

export function SendMoneyDialog(props: SendMoneyDialogProps) {
  const INITIAL_VALUES = {
    publicKey: "",
    confidential: false,
    amount: "",
    badge: null,
  };
  const [useBadge, setUseBadge] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [estimatedFee, setEstimatedFee] = useState(0);
  const [transferFormState, setTransferFormState] = useState(INITIAL_VALUES);
  const [validity, setValidity] = useState<object>({
    publicKey: false,
    amount: false,
  });

  const { accountName, setPopup } = useAccountStore();

  const theme = useTheme();

  const { data } = useAccountsGetBalances(accountName);
  const badges = data?.balances
    ?.filter((b) => b.resource_type === "NonFungible" && b.balance > 0)
    .map((b) => b.resource_address) as string[];

  const { mutateAsync: sendIt } = useAccountsTransfer(
    accountName,
    parseInt(transferFormState.amount),
    props.resource_address,
    transferFormState.publicKey,
    estimatedFee,
    transferFormState.confidential,
    transferFormState.badge,
    false,
  );

  const { mutateAsync: calculateFeeEstimate } = useAccountsTransfer(
    accountName,
    parseInt(transferFormState.amount),
    props.resource_address,
    transferFormState.publicKey,
    1000,
    transferFormState.confidential,
    transferFormState.badge,
    true,
  );

  function setFormValue(e: React.ChangeEvent<HTMLInputElement>) {
    setTransferFormState({
      ...transferFormState,
      [e.target.name]: e.target.value,
    });
    if (validity[e.target.name as keyof object] !== undefined) {
      setValidity({
        ...validity,
        [e.target.name]: e.target.validity.valid,
      });
    }
    setEstimatedFee(0);
  }

  function setSelectFormValue(e: SelectChangeEvent<unknown>) {
    setTransferFormState({
      ...transferFormState,
      [e.target.name]: e.target.value,
    });
    setEstimatedFee(0);
  }

  function setCheckboxFormValue(e: React.ChangeEvent<HTMLInputElement>) {
    setTransferFormState({
      ...transferFormState,
      [e.target.name]: e.target.checked,
    });
    setEstimatedFee(0);
  }

  const onTransfer = async () => {
    if (accountName) {
      setDisabled(true);
      if (estimatedFee) {
        sendIt()
          .then(() => {
            setTransferFormState(INITIAL_VALUES);
            props.onSendComplete?.();
            setPopup({ title: "Send successful", error: false });
          })
          .catch((e) => {
            setPopup({ title: "Send failed", error: true, message: e.message });
          })
          .finally(() => {
            setDisabled(false);
          });
      } else {
        let result = await calculateFeeEstimate();
        setEstimatedFee(result.fee);
        setDisabled(false);
      }
    }
  };

  const handleClose = () => {
    props.handleClose?.();
  };

  const handleUseBadgeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setUseBadge(e.target.checked);
    if (!e.target.checked) {
      setTransferFormState({
        ...transferFormState,
        badge: null,
      });
    }
  };

  const allValid = Object.values(validity).every((v) => v);

  return (
    <Dialog open={props.open} onClose={handleClose}>
      <DialogTitle>Send {props.resource_address}</DialogTitle>
      <DialogContent className="dialog-content">
        <Form onSubmit={onTransfer} className="flex-container-vertical" style={{ paddingTop: theme.spacing(1) }}>
          {badges && (
            <>
              <FormControlLabel
                control={<CheckBox name="useBadge" checked={useBadge} onChange={handleUseBadgeChange} />}
                label="Use Badge"
              />
              <Select
                name="badge"
                disabled={!useBadge || disabled}
                displayEmpty
                value={transferFormState.badge || ""}
                onChange={setSelectFormValue}
              >
                {badges.map((b, i) => (
                  <MenuItem key={i} value={b}>
                    {b}
                  </MenuItem>
                ))}
              </Select>
            </>
          )}
          <TextField
            name="publicKey"
            label="Public Key"
            value={transferFormState.publicKey}
            inputProps={{ pattern: "^[0-9a-fA-F]*$" }}
            onChange={setFormValue}
            style={{ flexGrow: 1 }}
            disabled={disabled}
          />
          <FormControlLabel
            control={
              <CheckBox
                name="confidential"
                checked={transferFormState.confidential}
                onChange={setCheckboxFormValue}
                disabled={disabled}
              />
            }
            label="Confidential"
          />
          <TextField
            name="amount"
            label="Amount"
            value={transferFormState.amount}
            type="number"
            onChange={setFormValue}
            style={{ flexGrow: 1 }}
            disabled={disabled}
          />
          <TextField
            name="fee"
            label="Fee"
            value={estimatedFee || "Press fee estimate to calculate"}
            style={{ flexGrow: 1 }}
            disabled={disabled}
            InputProps={{ readOnly: true }}
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
            <Button variant="contained" type="submit" disabled={disabled || !allValid}>
              {estimatedFee ? "Send" : "Estimate fee"}
            </Button>
          </Box>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
