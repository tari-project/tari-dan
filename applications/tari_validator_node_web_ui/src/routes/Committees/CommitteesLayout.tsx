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

import { useEffect, useState, useContext } from 'react';
import PageHeading from '../../Components/PageHeading';
import Typography from '@mui/material/Typography';
import Grid from '@mui/material/Grid';
import { StyledPaper } from '../../Components/StyledComponents';
import { VNContext } from '../../App';
import Committees from './Committees';
import CommitteesRadial from './CommitteesRadial';
import CommitteesPieChart from './CommitteesPieChart';
import { get_all_committees } from './helpers';

function CommitteesLayout() {
  const [committees, setCommittees] = useState<
    Array<[string, string, Array<string>]>
  >([]);

  const { epoch, shardKey, identity, error } = useContext(VNContext);

  useEffect(() => {
    if (identity?.public_key && shardKey && epoch) {
      get_all_committees(
        epoch.current_epoch,
        shardKey,
        identity.public_key
      ).then((response) => {
        if (response) setCommittees(response);
      });
    }
  }, [epoch?.current_epoch, shardKey, identity?.public_key, epoch]);

  if (error !== '') {
    return <div className="error">{error}</div>;
  }
  if (epoch === undefined || identity === undefined) return <div>Loading</div>;

  if (!committees) {
    return <Typography>Committees are loading</Typography>;
  }

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Committees</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={8}>
        <StyledPaper>
          <CommitteesPieChart chartData={committees} />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={4}>
        <StyledPaper>
          <CommitteesRadial committees={committees} />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Typography>
            {shardKey ? (
              <Committees
                currentEpoch={epoch.current_epoch}
                shardKey={shardKey}
                publicKey={identity.public_key}
              />
            ) : null}
          </Typography>
        </StyledPaper>
      </Grid>
    </>
  );
}

export default CommitteesLayout;
