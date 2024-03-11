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
import { Form, Link, useLocation } from "react-router-dom";
import AddIcon from "@mui/icons-material/Add";
import Alert from "@mui/material/Alert";
import Button from "@mui/material/Button/Button";
import Fade from "@mui/material/Fade";
import MenuItem from "@mui/material/MenuItem";
import Select, { SelectChangeEvent } from "@mui/material/Select/Select";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import TextField from "@mui/material/TextField/TextField";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import FormControl from "@mui/material/FormControl";
import InputLabel from "@mui/material/InputLabel";
import { ChevronRight } from "@mui/icons-material";
import IconButton from "@mui/material/IconButton";
import { BoxHeading2, DataTableCell } from "../../../Components/StyledComponents";
import { shortenString, toHexString } from "../../../utils/helpers";
import {
  useAccountsCreate,
  useAccountsCreateFreeTestCoins,
  useAccountsClaimBurn,
  useAccountsList,
} from "../../../api/hooks/useAccounts";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import queryClient from "../../../api/queryClient";
import type { AccountInfo } from "@tariproject/typescript-bindings/wallet-daemon-client";

function Account(account: AccountInfo, index: number) {
  const { pathname } = useLocation();
  if (!("Component" in account.account.address)) {
    return null;
  }
  return (
    <TableRow key={index}>
      <DataTableCell>
        <Link
          to={`/accounts/${account.account.name}`}
          style={{
            textDecoration: "none",
            color: "inherit",
          }}
        >
          {account.account.name}
        </Link>
      </DataTableCell>
      <DataTableCell>
        {shortenString(account.account.address.Component)}
        <CopyToClipboard copy={account.account.address.Component} />
      </DataTableCell>
      <DataTableCell>{account.account.key_index}</DataTableCell>
      <DataTableCell>
        {shortenString(account.public_key)}
        <CopyToClipboard copy={account.public_key} />
      </DataTableCell>
      <DataTableCell>
        <IconButton component={Link} to={`/accounts/${account.account.name}`}>
          <ChevronRight />
        </IconButton>
      </DataTableCell>
    </TableRow>
  );
}

function Accounts() {
  const [error, setError] = useState<String>();
  const [showAccountDialog, setShowAddAccountDialog] = useState(false);
  const [showClaimDialog, setShowClaimBurnDialog] = useState(false);
  const [accountFormState, setAccountFormState] = useState({
    accountName: "",
    signingKeyIndex: "",
    fee: "",
  });
  const [claimBurnFormState, setClaimBurnFormState] = useState({
    account: "",
    claimProof: "",
    fee: "",
  });
  const {
    data: dataAccountsList,
    isLoading: isLoadingAccountsList,
    isError: isErrorAccountsList,
    error: errorAccountsList,
  } = useAccountsList(0, 10);
  const { mutateAsync: mutateCreateFeeTestCoins } = useAccountsCreateFreeTestCoins();

  const { mutateAsync: mutateAddAccount } = useAccountsCreate(accountFormState.accountName, null, null, false);

  const { mutateAsync: mutateClaimBurn } = useAccountsClaimBurn(
    claimBurnFormState.account,
    claimBurnFormState.claimProof ? JSON.parse(claimBurnFormState.claimProof) : null,
    +claimBurnFormState.fee,
  );

  const showAddAccountDialog = (setElseToggle: boolean = !showAccountDialog) => {
    setShowAddAccountDialog(setElseToggle);
    setAccountFormState({
      accountName: "",
      signingKeyIndex: "",
      fee: "",
    });
  };

  const showClaimBurnDialog = (setElseToggle: boolean = !showClaimDialog) => {
    setShowClaimBurnDialog(setElseToggle);
    setClaimBurnFormState({
      account: "",
      claimProof: "",
      fee: "",
    });
  };

  const onSubmitAddAccount = () => {
    mutateAddAccount(undefined, {
      onSettled: () => {
        setAccountFormState({
          accountName: "",
          signingKeyIndex: "",
          fee: "",
        });
        setShowAddAccountDialog(false);
        queryClient.invalidateQueries(["accounts"]);
      },
    });
  };

  const onAccountChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    e.preventDefault();
    setAccountFormState({
      ...accountFormState,
      [e.target.name]: e.target.value,
    });
  };

  const onClaimFreeCoins = async () => {
    await mutateCreateFeeTestCoins({
      accountName: "TestAccount",
      amount: 100000,
      fee: 1000,
    });
  };

  const onClaimBurn = () => {
    mutateClaimBurn(undefined, {
      onSettled: () => {
        setClaimBurnFormState({ account: "", claimProof: "", fee: "" });
        setShowClaimBurnDialog(false);
      },
    });
  };

  const onClaimBurnChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setClaimBurnFormState({
      ...claimBurnFormState,
      [e.target.name]: e.target.value,
    });
  };

  const onClaimBurnAccountChange = (e: SelectChangeEvent<string>) => {
    setClaimBurnFormState({
      ...claimBurnFormState,
      [e.target.name]: e.target.value,
    });
  };

  return (
    <>
      {error ? <Alert severity="error">{error}</Alert> : null}
      <BoxHeading2
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
        }}
      >
        <div className="flex-container">
          <Button variant="outlined" startIcon={<AddIcon />} onClick={() => onClaimFreeCoins()}>
            Claim Free Testnet Coins
          </Button>
        </div>
        {showAccountDialog && (
          <Fade in={showAccountDialog}>
            <Form onSubmit={onSubmitAddAccount} className="flex-container">
              <TextField
                name="accountName"
                label="Account Name"
                value={accountFormState.accountName}
                onChange={onAccountChange}
                style={{ flexGrow: 1 }}
              />
              <Button variant="contained" type="submit">
                Add Account
              </Button>
              <Button variant="outlined" onClick={() => showAddAccountDialog(false)}>
                Cancel
              </Button>
            </Form>
          </Fade>
        )}
        {!showAccountDialog && (
          <Fade in={!showAccountDialog}>
            <div className="flex-container">
              <Button variant="outlined" startIcon={<AddIcon />} onClick={() => showAddAccountDialog()}>
                Add Account
              </Button>
            </div>
          </Fade>
        )}
        {showClaimDialog && (
          <Fade in={showClaimDialog}>
            <Form onSubmit={onClaimBurn} className="flex-container">
              <FormControl>
                <InputLabel id="account">Account</InputLabel>
                <Select
                  labelId="account"
                  name="account"
                  label="Account"
                  value={claimBurnFormState.account}
                  onChange={onClaimBurnAccountChange}
                  style={{ flexGrow: 1, minWidth: "200px" }}
                >
                  {dataAccountsList?.accounts.map((account: AccountInfo, index: number) => {
                    if (!("Component" in account.account.address)) {
                      return null;
                    }
                    return (
                      <MenuItem
                        key={toHexString(account.account.address.Component)}
                        value={"component_" + toHexString(account.account.address.Component)}
                      >
                        {account.account.name}
                      </MenuItem>
                    );
                  })}
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
              <Button variant="contained" type="submit">
                Claim Burn
              </Button>
              <Button variant="outlined" onClick={() => showClaimBurnDialog(false)}>
                Cancel
              </Button>
            </Form>
          </Fade>
        )}
        {!showClaimDialog && (
          <Fade in={!showClaimDialog}>
            <div className="flex-container">
              <Button variant="outlined" startIcon={<AddIcon />} onClick={() => showClaimBurnDialog()}>
                Claim Burn
              </Button>
            </div>
          </Fade>
        )}
      </BoxHeading2>
      <FetchStatusCheck
        isLoading={isLoadingAccountsList}
        isError={isErrorAccountsList}
        errorMessage={errorAccountsList?.message || "Error fetching data"}
      />
      <Fade in={!isLoadingAccountsList && !isErrorAccountsList}>
        <TableContainer>
          <Table>
            <TableHead>
              <TableRow>
                <TableCell>Name</TableCell>
                <TableCell>Address</TableCell>
                <TableCell>Key index</TableCell>
                <TableCell>Public key</TableCell>
                <TableCell>Details</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {dataAccountsList &&
                dataAccountsList.accounts.map((account: AccountInfo, index: number) => Account(account, index))}
            </TableBody>
          </Table>
        </TableContainer>
      </Fade>
    </>
  );
}

export default Accounts;
