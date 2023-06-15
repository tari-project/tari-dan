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

import PageHeading from '../../Components/PageHeading';
import Grid from '@mui/material/Grid';
import { StyledPaper } from '../../Components/StyledComponents';
import Accounts from '../Wallet/Components/Accounts';
import TableContainer from '@mui/material/TableContainer';
import Table from '@mui/material/Table';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import TableCell from '@mui/material/TableCell';
import TableBody from '@mui/material/TableBody';
import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import { accountsGetBalances, accountsGet } from '../../utils/json_rpc';
import Alert from '@mui/material/Alert';
import { removeTagged, toHexString } from '../../utils/helpers';

function BalanceRow(props: any) {
  return (
    <TableRow>
      <TableCell>{props.resource_address}</TableCell>
      <TableCell>{props.resource_type}</TableCell>
      <TableCell>{removeTagged(props.balance)}</TableCell>
      <TableCell>{removeTagged(props.confidential_balance)}</TableCell>
    </TableRow>
  );
}

function AccountDetailsLayout() {
  const { name } = useParams<{ name: string }>();
  let [state, setState] = useState(null);
  let [balances, setBalances] = useState(null);
  let [error, setError] = useState(null);

  const loadAccount = () => {
    accountsGet(name)
      .then((response: any) => {
        setState(response);
      })
      .catch((error: any) => {
        console.error(error);
        setError(error.message);
      });
  };

  const loadBalances = () => {
    accountsGetBalances(name)
      .then((response: any) => {
        setBalances(response);
      })
      .catch((error: any) => {
        console.error(error);
        setError(error.message);
      });
  };

  useEffect(() => loadAccount(), []);
  useEffect(() => loadBalances(), []);

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Account Details</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        {error ? <Alert severity="error">{error}</Alert> : null}
        <StyledPaper>
          <TableContainer>
            <Table>
              <TableHead>
                <TableRow>
                  <TableCell>Name</TableCell>
                  <TableCell>Address</TableCell>
                  <TableCell>Public key</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                <TableRow>
                  <TableCell>{state?.account.name}</TableCell>
                  <TableCell>
                    {state?.account.address.Component}
                  </TableCell>
                  <TableCell>{state?.public_key}</TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </TableContainer>
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          Balances
          <TableContainer>
            <Table>
              <TableHead>
                <TableRow>
                  <TableCell>Resource</TableCell>
                  <TableCell>Resource Type</TableCell>
                  <TableCell>Revealed Balance</TableCell>
                  <TableCell>Confidential Balance</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {balances?.balances.map((balance: any) => BalanceRow(balance))}
              </TableBody>
            </Table>
          </TableContainer>
        </StyledPaper>
      </Grid>
    </>
  );
}

export default AccountDetailsLayout;
