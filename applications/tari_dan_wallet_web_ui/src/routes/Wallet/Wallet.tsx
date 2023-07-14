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
import Accounts from './Components/Accounts';
import Keys from './Components/Keys';
import './Wallet.css';
import { StyledPaper } from '../../Components/StyledComponents';
import Grid from '@mui/material/Grid';
import SecondaryHeading from '../../Components/SecondaryHeading';
import Transactions from '../Transactions/Transactions';
import JWTGrid from './Components/JWTGrid';

function Wallet() {
  const [error, setError] = useState('');
  if (error !== '') {
    return <div className="error">{error}</div>;
  }
  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Accounts</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Accounts />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Keys</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Keys />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Transactions</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Transactions />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>JWTs</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <JWTGrid />
        </StyledPaper>
      </Grid>
    </>
  );
}

export default Wallet;
