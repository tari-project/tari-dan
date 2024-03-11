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

import { useContext, useEffect, useState } from "react";
import AllVNs from "./Components/AllVNs";
import Committees from "../Committees/Committees";
import Connections from "./Components/Connections";
import Fees from "./Components/Fees";
import Info from "./Components/Info";
import Mempool from "./Components/Mempool";
import Blocks from "./Components/Blocks";
import Templates from "./Components/Templates";
import "./ValidatorNode.css";
import { StyledPaper } from "../../Components/StyledComponents";
import Grid from "@mui/material/Grid";
import SecondaryHeading from "../../Components/SecondaryHeading";
import { VNContext } from "../../App";
import { getNetworkCommittees } from "../../utils/json_rpc";
import type { GetNetworkCommitteeResponse } from "@tariproject/typescript-bindings/validator-node-client";

function ValidatorNode() {
  const [committees, setCommittees] = useState<GetNetworkCommitteeResponse | null>(null);

  const { epoch, identity, shardKey, error } = useContext(VNContext);

  useEffect(() => {
    getNetworkCommittees().then(setCommittees);
  }, []);

  if (error !== "") {
    return <div className="error">{error}</div>;
  }
  if (epoch === undefined || identity === undefined || shardKey === undefined) return <div>Loading</div>;

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Info</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Info epoch={epoch} identity={identity} shardKey={shardKey} />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Committees</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          {committees ? (
            <>
              <Committees peerId={identity.peer_id} committees={committees.committees} />
            </>
          ) : null}
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Connections</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Connections />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Fees</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Fees />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Mempool</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Mempool />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Blocks</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Blocks />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Templates</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Templates />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>VNs</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <AllVNs epoch={epoch.current_epoch} />
        </StyledPaper>
      </Grid>
    </>
  );
}

export default ValidatorNode;
