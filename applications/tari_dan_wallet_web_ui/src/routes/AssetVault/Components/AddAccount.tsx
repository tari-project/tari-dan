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

import { useState } from 'react';
import { Form } from 'react-router-dom';
import Button from '@mui/material/Button';
import TextField from '@mui/material/TextField';
import Dialog from '@mui/material/Dialog';
import DialogContent from '@mui/material/DialogContent';
import DialogTitle from '@mui/material/DialogTitle';
import Box from '@mui/material/Box';
import { useAccountsCreate } from '../../../api/hooks/useAccounts';
import { useTheme } from '@mui/material/styles';

function AddAccount({
  open,
  setOpen,
}: {
  open: boolean;
  setOpen: React.Dispatch<React.SetStateAction<boolean>>;
}) {
  const [accountFormState, setAccountFormState] = useState({
    accountName: '',
    signingKeyIndex: '',
    fee: '',
  });
  const { mutate: mutateAddAccount } = useAccountsCreate(
    accountFormState.accountName,
    accountFormState.signingKeyIndex
      ? +accountFormState.signingKeyIndex
      : undefined,
    undefined,
    accountFormState.fee ? +accountFormState.fee : undefined,
    false
  );
  const theme = useTheme();

  const handleClose = () => {
    setOpen(false);
  };

  const onSubmitAddAccount = () => {
    mutateAddAccount();
    setAccountFormState({ accountName: '', signingKeyIndex: '', fee: '' });
    setOpen(false);
  };

  const onAccountChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setAccountFormState({
      ...accountFormState,
      [e.target.name]: e.target.value,
    });
  };

  return (
    <Dialog open={open} onClose={handleClose}>
      <DialogTitle>Add Account</DialogTitle>
      <DialogContent className="dialog-content">
        <Form
          onSubmit={onSubmitAddAccount}
          className="flex-container-vertical"
          style={{ paddingTop: theme.spacing(1) }}
        >
          <TextField
            name="accountName"
            label="Account Name"
            value={accountFormState.accountName}
            onChange={onAccountChange}
            style={{ flexGrow: 1 }}
          />
          <TextField
            name="signingKeyIndex"
            label="Signing Key Index"
            value={accountFormState.signingKeyIndex}
            onChange={onAccountChange}
            style={{ flexGrow: 1 }}
          />
          <TextField
            name="fee"
            label="Fee"
            value={accountFormState.fee}
            onChange={onAccountChange}
            style={{ flexGrow: 1 }}
          />
          <Box
            className="flex-container"
            style={{
              justifyContent: 'flex-end',
            }}
          >
            <Button variant="outlined" onClick={handleClose}>
              Cancel
            </Button>
            <Button variant="contained" type="submit">
              Add Account
            </Button>
          </Box>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

export default AddAccount;
