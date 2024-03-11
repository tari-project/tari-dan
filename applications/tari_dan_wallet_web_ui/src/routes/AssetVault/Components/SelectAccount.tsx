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
import { IoAdd } from "react-icons/io5";
import Divider from "@mui/material/Divider";
import Box from "@mui/material/Box";
import { useTheme } from "@mui/material/styles";
import InputLabel from "@mui/material/InputLabel";
import MenuItem from "@mui/material/MenuItem";
import FormControl from "@mui/material/FormControl";
import Select, { SelectChangeEvent } from "@mui/material/Select";
import Dialog from "./AddAccount";
import useAccountStore from "../../../store/accountStore";
import { useAccountsList } from "../../../api/hooks/useAccounts";
import type { AccountInfo } from "@tariproject/typescript-bindings/wallet-daemon-client";

function SelectAccount() {
  const { accountName, setAccountName } = useAccountStore();
  const { data: dataAccountsList } = useAccountsList(0, 10);
  const [dialogOpen, setDialogOpen] = useState(false);
  const theme = useTheme();

  const handleChange = (event: SelectChangeEvent) => {
    const selectedValue = event.target.value as string;
    if (selectedValue !== "addAccount") {
      setAccountName(event.target.value as string);
    }
  };

  const handleAddAccount = () => {
    setDialogOpen(true);
  };
  return (
    <Box sx={{ minWidth: 250 }}>
      <Dialog open={dialogOpen} setOpen={setDialogOpen} />
      <FormControl fullWidth>
        <InputLabel id="account-select-label">Account</InputLabel>
        <Select
          labelId="account-select-label"
          id="account-select"
          value={
            dataAccountsList?.accounts.some((account: AccountInfo) => account.account.name == accountName)
              ? accountName
              : "addAccount"
          }
          label="Account"
          onChange={handleChange}
        >
          {dataAccountsList?.accounts.map((account: AccountInfo) => {
            if (account.account.name === null) {
              return null;
            }
            return (
              <MenuItem key={account.public_key} value={account.account.name}>
                {account.account.name}
              </MenuItem>
            );
          })}
          <Divider />
          <MenuItem value={"addAccount"} onClick={handleAddAccount}>
            <IoAdd style={{ marginRight: theme.spacing(1) }} />
            Add Account
          </MenuItem>
        </Select>
      </FormControl>
    </Box>
  );
}

export default SelectAccount;
