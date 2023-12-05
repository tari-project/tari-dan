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
import { useAccountsTransfer } from "../../../api/hooks/useAccounts";
import { useTheme } from "@mui/material/styles";
import useAccountStore from "../../../store/accountStore";

export default function SendMoney() {
  const [open, setOpen] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [transferFormState, setTransferFormState] = useState({
    publicKey: "",
    confidential: false,
    amount: "",
    fee: "",
  });

  const { accountName, setPopup } = useAccountStore();

  const theme = useTheme();

  const { mutateAsync: sendIt } = useAccountsTransfer(
    accountName,
    parseInt(transferFormState.amount),
    "resource_0101010101010101010101010101010101010101010101010101010101010101",
    transferFormState.publicKey,
    parseInt(transferFormState.fee),
    transferFormState.confidential
  );

  const onPublicKeyChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (/^[0-9a-fA-F]*$/.test(e.target.value)) {
      setTransferFormState({
        ...transferFormState,
        [e.target.name]: e.target.value,
      });
    }
  };

  const onConfidentialChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setTransferFormState({
      ...transferFormState,
      [e.target.name]: e.target.checked,
    });
  };

  const onNumberChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (/^[0-9]*$/.test(e.target.value)) {
      setTransferFormState({
        ...transferFormState,
        [e.target.name]: e.target.value,
      });
    }
  };

  const onTransfer = async () => {
    if (accountName) {
      setDisabled(true);
      sendIt().then(() => {
        setTransferFormState({ publicKey: "", confidential: false, amount: "", fee: "" });
        setOpen(false);
        setPopup({ title: "Send successful", error: false });
      }).catch((e) => {
        setPopup({ title: "Send failed", error: true, message: e.message });
      }).finaly(() => {
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

  return (
    <div>
      <Button variant="outlined" onClick={handleClickOpen}>
        Send Tari
      </Button>
      <Dialog open={open} onClose={handleClose}>
        <DialogTitle>Send Tari</DialogTitle>
        <DialogContent className="dialog-content">
          <Form onSubmit={onTransfer} className="flex-container-vertical" style={{ paddingTop: theme.spacing(1) }}>
            <TextField
              name="publicKey"
              label="Public Key"
              value={transferFormState.publicKey}
              onChange={onPublicKeyChange}
              style={{ flexGrow: 1 }}
              disabled={disabled}
            />
            <FormControlLabel
              control={
                <CheckBox
                  name="confidential"
                  checked={transferFormState.confidential}
                  onChange={onConfidentialChange}
                  disabled={disabled}
                />
              }
              label="Confidential"
            />
            <TextField
              name="amount"
              label="Amount"
              value={transferFormState.amount}
              onChange={onNumberChange}
              style={{ flexGrow: 1 }}
              disabled={disabled}
            />
            <TextField
              name="fee"
              label="Fee"
              value={transferFormState.fee}
              onChange={onNumberChange}
              style={{ flexGrow: 1 }}
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
              <Button variant="contained" type="submit" disabled={disabled}>
                Send Tari
              </Button>
            </Box>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
