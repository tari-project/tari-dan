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

import { useEffect, useState } from 'react';
import AllVNs from './Components/AllVNs';
import Committees from './Components/Committees';
import Connections from './Components/Connections';
import Info from './Components/Info';
import { IEpoch, IIdentity } from '../../utils/interfaces';
import {
  getEpochManagerStats,
  getIdentity,
  getRecentTransactions,
  getShardKey,
} from '../../utils/json_rpc';
import Mempool from './Components/Mempool';
import RecentTransactions from './Components/RecentTransactions';
import Templates from './Components/Templates';
import './ValidatorNode.css';
import { StyledPaper } from '../../Components/StyledComponents';
import Grid from '@mui/material/Grid';
import SecondaryHeading from '../../Components/SecondaryHeading';

function ValidatorNode() {
  const [epoch, setEpoch] = useState<IEpoch | undefined>(undefined);
  const [identity, setIdentity] = useState<IIdentity | undefined>(undefined);
  const [shardKey, setShardKey] = useState<string | null>(null);
  const [error, setError] = useState('');
  // Refresh every 2 minutes
  const refreshEpoch = (epoch: IEpoch | undefined) => {
    getEpochManagerStats()
      .then((response) => {
        if (response.current_epoch !== epoch?.current_epoch) {
          setEpoch(response);
        }
      })
      .catch((reason) => {
        console.log(reason);
        setError('Json RPC error, please check console');
      });
  };
  useEffect(() => {
    const id = window.setInterval(() => {
      refreshEpoch(epoch);
    }, 2 * 60 * 1000);
    return () => {
      window.clearInterval(id);
    };
  }, [epoch]);
  // Initial fetch
  useEffect(() => {
    refreshEpoch(undefined);
    getIdentity()
      .then((response) => {
        setIdentity(response);
      })
      .catch((reason) => {
        console.log(reason);
        setError('Json RPC error, please check console');
      });
  }, []);
  // Get shard key.
  useEffect(() => {
    if (epoch !== undefined && identity !== undefined) {
      // The *10 is from the hardcoded constant in VN.
      getShardKey(epoch.current_epoch * 10, identity.public_key).then(
        (response) => {
          setShardKey(response.shard_key);
        }
      );
    }
  }, [epoch, identity]);
  useEffect(() => {
    getRecentTransactions();
  }, []);
  if (error !== '') {
    return <div className="error">{error}</div>;
  }
  if (epoch === undefined || identity === undefined) return <div>Loading</div>;
  return (
    <Grid container spacing={5}>
      <SecondaryHeading>Info</SecondaryHeading>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Info epoch={epoch} identity={identity} shardKey={shardKey} />
          {shardKey ? (
            <Committees
              currentEpoch={epoch.current_epoch}
              shardKey={shardKey}
              publicKey={identity.public_key}
            />
          ) : null}
        </StyledPaper>
      </Grid>
      <SecondaryHeading>Connections</SecondaryHeading>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Connections />
        </StyledPaper>
      </Grid>
      <SecondaryHeading>Mempool</SecondaryHeading>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Mempool />
        </StyledPaper>
      </Grid>
      <SecondaryHeading>Recent Transactions</SecondaryHeading>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <RecentTransactions />
        </StyledPaper>
      </Grid>
      <SecondaryHeading>Templates</SecondaryHeading>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Templates />
        </StyledPaper>
      </Grid>
      <SecondaryHeading>Epoch</SecondaryHeading>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <AllVNs epoch={epoch.current_epoch} />
        </StyledPaper>
      </Grid>
    </Grid>
  );
}

export default ValidatorNode;
