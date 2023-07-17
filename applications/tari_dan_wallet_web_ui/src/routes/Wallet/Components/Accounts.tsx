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

import { ReactNode, useEffect, useState } from "react";
import {accountsClaimBurn, accountsCreate, accountsCreateFreeTestCoins, accountsList} from "../../../utils/json_rpc";
import Error from "./Error";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { fromHexString, toHexString } from "../../../utils/helpers";
import { BoxHeading2 } from "../../../Components/StyledComponents";
import Fade from "@mui/material/Fade";
import { Form } from "react-router-dom";
import TextField from "@mui/material/TextField/TextField";
import Select, { SelectChangeEvent } from "@mui/material/Select/Select";
import Button from "@mui/material/Button/Button";
import AddIcon from "@mui/icons-material/Add";
import { removeTagged } from "../../../utils/helpers";
import MenuItem from "@mui/material/MenuItem";
import Alert from "@mui/material/Alert";
import Link from "@mui/material/Link";

function Account(account: any) {
  return (
    <TableRow key={toHexString(account.account.address.Component)}>
      <TableCell><Link href={"accounts/" + account.account.name}>{account.account.name}</Link></TableCell>
      <TableCell>{account.account.address.Component}</TableCell>
      <TableCell>{account.account.key_index}</TableCell>
      <TableCell>{account.public_key}</TableCell>
    </TableRow>
  );
}

function Accounts() {
  const [state, setState] = useState<any>();
  const [error, setError] = useState<String>();
  const [showAccountDialog, setShowAddAccountDialog] = useState(false);
  const [showClaimDialog, setShowClaimBurnDialog] = useState(false);
  const [accountFormState, setAccountFormState] = useState({ accountName: "", signingKeyIndex: "", fee: "" });
  const [claimBurnFormState, setClaimBurnFormState] = useState({ account: "", claimProof: "", fee: "" });

  const showAddAccountDialog = (setElseToggle: boolean = !showAccountDialog) => {
    setShowAddAccountDialog(setElseToggle);
  };

  const showClaimBurnDialog = (setElseToggle: boolean = !showClaimDialog) => {
    setShowClaimBurnDialog(setElseToggle);
  };

  const loadAccounts = () => {
    accountsList(0, 10)
      .then((response) => {
        console.log(response);
        setState(response);
        setError(undefined);
      })
      .catch((err) => {
        setError(err && err.message ? err.message : `Unknown error: ${JSON.stringify(err)}`);
      });
  };

  const onSubmitAddAccount = () => {
    accountsCreate(
      accountFormState.accountName,
      accountFormState.signingKeyIndex ? +accountFormState.signingKeyIndex : undefined,
      undefined,
      accountFormState.fee ? +accountFormState.fee : undefined,
      false
    ).then((response) => {
      loadAccounts();
    });
    setAccountFormState({ accountName: "", signingKeyIndex: "", fee: "" });
    setShowAddAccountDialog(false);
  };
  const onAccountChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setAccountFormState({ ...accountFormState, [e.target.name]: e.target.value });
  };

  const onClaimFreeCoins = () => {
    accountsCreateFreeTestCoins("TestAccount", 1000, 0).then((response) => { loadAccounts(); });
  }

  const onClaimBurn = () => {
    accountsClaimBurn(claimBurnFormState.account, JSON.parse(claimBurnFormState.claimProof), +claimBurnFormState.fee)
      .then((response) => {
        console.log(response);
        loadAccounts();
      })
      .catch((err) => {
        console.log(err);
        setError(err && err.message ? err.message : `Unknown error: ${JSON.stringify(err)}`);
      });
    setClaimBurnFormState({ account: "", claimProof: "", fee: "" });
    setShowClaimBurnDialog(false);
  };

  const onClaimBurnChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setClaimBurnFormState({ ...claimBurnFormState, [e.target.name]: e.target.value });
  };

  const onClaimBurnAccountChange = (e: SelectChangeEvent<string>) => {
    setClaimBurnFormState({ ...claimBurnFormState, [e.target.name]: e.target.value });
  };

  useEffect(() => {
    loadAccounts();
  }, []);

  return (
    <>
      {error ? (
        <Alert severity="error">{error}</Alert>
      ) : null}
      <BoxHeading2>
        <div className="flex-container">
          <Button variant="outlined" startIcon={<AddIcon />} onClick={() => onClaimFreeCoins()}>
            Claim free testnet coins
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
              <Select name="account"
                label="Account"
                value={claimBurnFormState.account}
                onChange={onClaimBurnAccountChange}
                style={{ flexGrow: 1 }}  >
                {state?.accounts.map((account: any) => (
                  <MenuItem key={toHexString(account.account.address.Component)} value={"component_" + toHexString(account.account.address.Component)}>{account.account.name} </MenuItem>

                ))}
              </Select>
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
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>Name</TableCell>
              <TableCell>Address</TableCell>
              <TableCell>Key index</TableCell>
              <TableCell>Public key</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>{state && state.accounts.map((account: any) => Account(account))}</TableBody>
        </Table>
      </TableContainer>
    </>
  );
}

export default Accounts;
