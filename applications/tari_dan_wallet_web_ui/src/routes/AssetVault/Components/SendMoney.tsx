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

import { useEffect, useState } from "react";
import { Form } from "react-router-dom";
import Button from "@mui/material/Button";
import CheckBox from "@mui/material/Checkbox";
import TextField from "@mui/material/TextField";
import Dialog from "@mui/material/Dialog";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import FormControlLabel from "@mui/material/FormControlLabel";
import Box from "@mui/material/Box";
import { useAccountsGet, useAccountsGetBalances, useAccountsTransfer } from "../../../api/hooks/useAccounts";
import { useTheme } from "@mui/material/styles";
import useAccountStore from "../../../store/accountStore";
import Select from "@mui/material/Select";
import { SelectChangeEvent } from "@mui/material/Select/Select";
import MenuItem from "@mui/material/MenuItem";
import {
  ResourceAddress,
  ResourceType,
  ConfidentialTransferInputSelection,
  TransactionResult,
} from "@tari-project/typescript-bindings";
import InputLabel from "@mui/material/InputLabel";

const XTR2 = "resource_01010101010101010101010101010101010101010101010101010101";

export default function SendMoney() {
  const [open, setOpen] = useState(false);

  return (
    <>
      <Button variant="outlined" onClick={() => setOpen(true)}>
        Send Tari
      </Button>
      <SendMoneyDialog
        open={open}
        handleClose={() => setOpen(false)}
        onSendComplete={() => setOpen(false)}
        resource_type="Confidential"
        resource_address={XTR2}
      />
    </>
  );
}

export interface SendMoneyDialogProps {
  open: boolean;
  resource_address?: ResourceAddress;
  resource_type?: ResourceType;
  onSendComplete?: () => void;
  handleClose: () => void;
}

export function SendMoneyDialog(props: SendMoneyDialogProps) {
  const INITIAL_VALUES = {
    publicKey: "",
    outputToConfidential: false,
    inputSelection: "PreferRevealed",
    amount: "",
    fee: "",
    badge: null,
  };
  const isConfidential = props.resource_type === "Confidential";
  const [useBadge, setUseBadge] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [transferFormState, setTransferFormState] = useState(INITIAL_VALUES);
  const [validity, setValidity] = useState<object>({
    publicKey: false,
    amount: false,
  });
  const [allValid, setAllValid] = useState(false);

  const { accountName, setPopup } = useAccountStore();

  const theme = useTheme();

  const { data } = useAccountsGetBalances(accountName);
  const badges = data?.balances
    ?.filter((b) => b.resource_type === "NonFungible" && b.balance > 0)
    .map((b) => b.resource_address) as string[];

  // TODO: we should have separate calls for confidential and non-confidential transfers
  const { mutateAsync: sendIt } = useAccountsTransfer(
    accountName,
    parseInt(transferFormState.amount),
    // HACK: default to XTR2 because the resource is only set when open==true, and we cannot conditionally call hooks i.e. when props.resource_address is set
    props.resource_address || XTR2,
    transferFormState.publicKey,
    parseInt(transferFormState.fee),
    // estimatedFee,
    props.resource_type === "Confidential",
    !transferFormState.outputToConfidential,
    transferFormState.inputSelection as ConfidentialTransferInputSelection,
    transferFormState.badge,
    false,
  );

  const { mutateAsync: calculateFeeEstimate } = useAccountsTransfer(
    accountName,
    parseInt(transferFormState.amount),
    props.resource_address || XTR2,
    transferFormState.publicKey,
    3000,
    props.resource_type === "Confidential",
    !transferFormState.outputToConfidential,
    transferFormState.inputSelection as ConfidentialTransferInputSelection,
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
  }

  function setSelectFormValue(e: SelectChangeEvent<unknown>) {
    setTransferFormState({
      ...transferFormState,
      [e.target.name]: e.target.value,
    });
  }

  function setCheckboxFormValue(e: React.ChangeEvent<HTMLInputElement>) {
    setTransferFormState({
      ...transferFormState,
      [e.target.name]: e.target.checked,
    });
  }

  const onTransfer = async () => {
    if (accountName) {
      setDisabled(true);
      if (!isNaN(parseInt(transferFormState.fee))) {
        sendIt?.()
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
        calculateFeeEstimate?.()
          .then((result) => {
            if (!("Accept" in result.result.result)) {
              setPopup({
                title: "Fee estimate failed",
                error: true,
                // TODO: fix this
                message: JSON.stringify(
                  unionGet(result.result.result, "Reject" as keyof TransactionResult) ||
                    unionGet(result.result.result, "AcceptFeeRejectRest" as keyof TransactionResult)?.[1],
                ),
              });
              return;
            }
            setTransferFormState({ ...transferFormState, fee: result.fee.toString() });
          })
          .catch((e) => {
            setPopup({ title: "Fee estimate failed", error: true, message: e.message });
          })
          .finally(() => {
            setDisabled(false);
          });
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

  useEffect(() => {
    setAllValid(Object.values(validity).every((v) => v));
  }, [validity]);

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
              <InputLabel id="select-badge">Badge</InputLabel>
              <Select
                id="select-badge"
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
            required
            onChange={setFormValue}
            style={{ flexGrow: 1 }}
            disabled={disabled}
          />
          {isConfidential && (
            <>
              <FormControlLabel
                control={
                  <CheckBox
                    name="outputToConfidential"
                    checked={transferFormState.outputToConfidential}
                    onChange={setCheckboxFormValue}
                    disabled={disabled}
                  />
                }
                label="Send Confidential Outputs"
              />
              <InputLabel id="select-input-selection">Input Selection</InputLabel>
              <Select
                name="inputSelection"
                disabled={disabled}
                displayEmpty
                value={transferFormState.inputSelection}
                onChange={setSelectFormValue}
              >
                <MenuItem value="PreferRevealed">Spend revealed funds first, then confidential</MenuItem>
                <MenuItem value="PreferConfidential">Spend confidential funds first, then revealed</MenuItem>
                <MenuItem value="ConfidentialOnly">Only spend confidential funds</MenuItem>
                <MenuItem value="RevealedOnly">Only spend revealed funds</MenuItem>
              </Select>
            </>
          )}
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
            value={transferFormState.fee}
            placeholder="Enter fee or press Estimate Fee to calculate"
            onChange={setFormValue}
            disabled={disabled}
            style={{ flexGrow: 1 }}
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
              {isNaN(parseInt(transferFormState.fee)) ? "Estimate fee" : "Send"}
            </Button>
          </Box>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function unionGet<T extends object>(object: T, key: keyof T): T[keyof T] | null {
  return key in object ? object[key] : null;
}
