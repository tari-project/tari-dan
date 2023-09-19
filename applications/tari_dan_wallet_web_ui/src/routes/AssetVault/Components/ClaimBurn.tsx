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
import FormControl from '@mui/material/FormControl';
import InputLabel from '@mui/material/InputLabel';
import Select, { SelectChangeEvent } from '@mui/material/Select/Select';
import MenuItem from '@mui/material/MenuItem';
import Box from '@mui/material/Box';
import {
  useAccountsList,
  useAccountsClaimBurn,
} from '../../../api/hooks/useAccounts';
import { toHexString } from '../../../utils/helpers';
import { useTheme } from '@mui/material/styles';

export default function ClaimBurn() {
  const [open, setOpen] = useState(false);
  const [claimBurnFormState, setClaimBurnFormState] = useState({
    account: '',
    claimProof: '',
    fee: '',
  });
  const { mutate: mutateClaimBurn } = useAccountsClaimBurn(
    claimBurnFormState.account,
    claimBurnFormState.claimProof
      ? JSON.parse(claimBurnFormState.claimProof)
      : null,
    +claimBurnFormState.fee
  );

  const { data: dataAccountsList } = useAccountsList(0, 10);

  const onClaimBurnAccountChange = (e: SelectChangeEvent<string>) => {
    setClaimBurnFormState({
      ...claimBurnFormState,
      [e.target.name]: e.target.value,
    });
  };

  const theme = useTheme();

  const onClaimBurnChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setClaimBurnFormState({
      ...claimBurnFormState,
      [e.target.name]: e.target.value,
    });
  };

  const onClaimBurn = () => {
    mutateClaimBurn();
    setClaimBurnFormState({ account: '', claimProof: '', fee: '' });
    setOpen(false);
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
        Claim Burn
      </Button>
      <Dialog open={open} onClose={handleClose}>
        <DialogTitle>Claim Burn</DialogTitle>
        <DialogContent className="dialog-content">
          <Form
            onSubmit={onClaimBurn}
            className="flex-container-vertical"
            style={{ paddingTop: theme.spacing(1) }}
          >
            <FormControl>
              <InputLabel id="account">Account</InputLabel>
              <Select
                labelId="account"
                name="account"
                label="Account"
                value={claimBurnFormState.account}
                onChange={onClaimBurnAccountChange}
                style={{ flexGrow: 1, minWidth: '200px' }}
              >
                {dataAccountsList?.accounts.map((account: any) => (
                  <MenuItem
                    key={toHexString(account.account.address.Component)}
                    value={
                      'component_' +
                      toHexString(account.account.address.Component)
                    }
                  >
                    {account.account.name}{' '}
                  </MenuItem>
                ))}
              </Select>
            </FormControl>
            <TextField
              name="claimProof"
              label="Claim Proof"
              value={claimBurnFormState.claimProof}
              onChange={onClaimBurnChange}
              style={{ flexGrow: 1 }}
            />
            <TextField
              name="fee"
              label="Fee"
              value={claimBurnFormState.fee}
              onChange={onClaimBurnChange}
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
                Claim Burn
              </Button>
            </Box>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
