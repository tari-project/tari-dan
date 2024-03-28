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

import { useEffect, useState } from "react";
import Connections from "./Components/Connections";
import Info from "./Components/Info";
import { getIdentity } from "../../utils/json_rpc";
import RecentTransactions from "./Components/RecentTransactions";
import "./ValidatorNode.css";
import { StyledPaper } from "../../Components/StyledComponents";
import Grid from "@mui/material/Grid";
import SecondaryHeading from "../../Components/SecondaryHeading";
import MonitoredSubstates from "./Components/MonitoredSubstates";
import MonitoredNftCollections from "./Components/MonitoredNftCollections";
import type { GetIdentityResponse } from "@tariproject/typescript-bindings/tari-indexer-client";

function ValidatorNode() {
  const [identity, setIdentity] = useState<GetIdentityResponse>();
  const [error, setError] = useState("");
  // Initial fetch
  useEffect(() => {
    getIdentity()
      .then((response) => {
        setIdentity(response);
      })
      .catch((reason) => {
        console.log(reason);
        setError("Json RPC error, please check console");
      });
  }, []);
  useEffect(() => {
    // getRecentTransactions();
  }, []);
  if (error !== "") {
    return <div className="error">{error}</div>;
  }
  if (identity === undefined) return <div>Loading</div>;
  return (
    <>
      <Grid item sm={12} md={12} xs={12}>
        <SecondaryHeading>Info</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Info identity={identity} />
        </StyledPaper>
      </Grid>
      <Grid item sm={12} md={12} xs={12}>
        <SecondaryHeading>Monitored Substates</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <MonitoredSubstates />
        </StyledPaper>
      </Grid>
      <Grid item sm={12} md={12} xs={12}>
        <SecondaryHeading>Monitored NFT collections</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <MonitoredNftCollections />
        </StyledPaper>
      </Grid>
      <Grid item sm={12} md={12} xs={12}>
        <SecondaryHeading>Connections</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <Connections />
        </StyledPaper>
      </Grid>
      <Grid item sm={12} md={12} xs={12}>
        <SecondaryHeading>Recent Transactions</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <RecentTransactions />
        </StyledPaper>
      </Grid>
    </>
  );
}

export default ValidatorNode;
