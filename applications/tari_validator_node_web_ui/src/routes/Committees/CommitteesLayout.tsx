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

import { useEffect, useState, useContext } from "react";
import PageHeading from "../../Components/PageHeading";
import Typography from "@mui/material/Typography";
import Grid from "@mui/material/Grid";
import { StyledPaper } from "../../Components/StyledComponents";
import { VNContext } from "../../App";
import Committees from "./Committees";
import CommitteesRadial from "./CommitteesRadial";
import CommitteesPieChart from "./CommitteesPieChart";
import { getNetworkCommittees } from "../../utils/json_rpc";
import type {
  CommitteeShardInfo,
  GetNetworkCommitteeResponse,
} from "@tariproject/typescript-bindings/validator-node-client";

function CommitteesLayout() {
  const [committees, setCommittees] = useState<GetNetworkCommitteeResponse | null>(null);

  const { epoch, identity, error } = useContext(VNContext);

  useEffect(() => {
    getNetworkCommittees().then(setCommittees);
  }, []);

  if (error !== "") {
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
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          Current epoch: {epoch.current_epoch}
          <br />
          Total number of validators:{" "}
          {committees.committees.reduce((acc: number, info: CommitteeShardInfo) => acc + info.validators.length, 0)}
          <br />
          Total buckets: {committees.committees.length}
          <br />
          Min committee size:{" "}
          {committees.committees
            .map((vn: CommitteeShardInfo) => vn.validators.length)
            .reduce((acc, curr) => Math.min(acc, curr), 100000)}
          <br />
          Max committee size:{" "}
          {committees.committees
            .map((vn: CommitteeShardInfo) => vn.validators.length)
            .reduce((acc, curr) => Math.max(acc, curr), 0)}
        </StyledPaper>
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
            <Committees committees={committees.committees} peerId={identity.peer_id} />
          </Typography>
        </StyledPaper>
      </Grid>
    </>
  );
}

export default CommitteesLayout;
