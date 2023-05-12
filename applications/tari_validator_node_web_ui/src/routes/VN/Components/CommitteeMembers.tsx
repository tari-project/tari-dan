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

import { useState, useEffect, useContext, useCallback } from 'react';
import { useParams, useLocation } from 'react-router-dom';
import { getCommittee, getShardKey } from '../../../utils/json_rpc';
import { Grid, Typography } from '@mui/material';
import { StyledPaper } from '../../../Components/StyledComponents';
import PageHeading from '../../../Components/PageHeading';
import Committee from './CommitteeSingle';
import { VNContext } from '../../../App';

async function getMembers(
  currentEpoch: number,
  shardKey: string,
  publicKey: string
) {
  const committee = await getCommittee(currentEpoch, shardKey);
  const committeeMembers = committee?.committee?.members;
  if (!committeeMembers || committeeMembers.length === 0) {
    throw new Error('Committee members not found');
  }
  return committeeMembers;
}

export default function CommitteeMembers() {
  const [members, setMembers] = useState([]);
  const { epoch, identity, shardKey } = useContext(VNContext);
  const { address } = useParams();
  const addresses = address && address.split(',');

  useEffect(() => {
    const fetchMembers = async () => {
      try {
        if (identity?.public_key && shardKey && epoch) {
          const committeeMembers = await getMembers(
            epoch.current_epoch,
            shardKey,
            identity.public_key
          );
          setMembers(committeeMembers);
        }
      } catch (error) {
        console.log('Error fetching members:', error);
      }
    };
    fetchMembers();
  }, [epoch, identity?.public_key, shardKey]);

  if (epoch === undefined || identity === undefined) return <div>Loading</div>;

  return (
    <>
      <Grid container spacing={5}>
        <Grid item xs={12} md={12} lg={12}>
          <PageHeading>Committee Members</PageHeading>
        </Grid>
        <Grid item xs={12} md={12} lg={12}>
          <StyledPaper>
            {addresses && (
              <>
                <Committee
                  key={addresses[0]}
                  begin={addresses[0]}
                  end={addresses[1]}
                  members={members}
                  publicKey={identity.public_key}
                />
              </>
            )}
          </StyledPaper>
        </Grid>
      </Grid>
    </>
  );
}
