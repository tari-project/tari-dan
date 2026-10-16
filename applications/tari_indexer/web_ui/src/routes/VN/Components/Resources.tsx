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

import { Box, Grid, Stack, Typography } from "@mui/material";
import { shortenString } from "@tari-project/ootle-ts-bindings";
import { useMemo } from "react";
import { useParams } from "react-router-dom";
import { useGetSubstate } from "../../../api/hooks/useSubstates";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import { formatCurrency } from "../../../utils/helpers";

function Resources() {
  const { resourceAddress } = useParams();

  const {
    data: substateData,
    isLoading,
    isError,
    error: errorMessage,
  } = useGetSubstate({
    address: resourceAddress,
    version: null,
    local_search_only: true,
    enabled: !!resourceAddress,
  });

  const resource = useMemo(() => {
    const substate = substateData && "up" in substateData.state ? substateData.state.up.substate : null;
    if (!substate || !("Resource" in substate)) {
      return null;
    }
    return substate.Resource;
  }, [substateData]);

  return (
    <FetchStatusCheck
      isLoading={isLoading}
      isError={isError}
      errorMessage={errorMessage ? (errorMessage as Error).message : "Error fetching resource data."}
    >
      <Stack direction="column">
        <Box sx={{ p: 2, backgroundColor: "background.paper", borderRadius: 1 }}>
          <Typography variant="h6" gutterBottom>
            Resource Information
          </Typography>
          <Grid container spacing={0}>
            <Grid size={{ xs: 12 }}>
              <Stack direction="row" alignItems="baseline" spacing={1}>
                <Typography variant="subtitle2">Address:</Typography>
                <Typography variant="body2" color="text.secondary">
                  {resourceAddress ? shortenString(resourceAddress) : ""}
                  <CopyToClipboard copy={resourceAddress || ""} />
                </Typography>
              </Stack>
            </Grid>
            <Grid size={{ xs: 12 }}>
              <Stack direction="row" alignItems="baseline" spacing={1}>
                <Typography variant="subtitle2">Token Symbol:</Typography>
                <Typography variant="body2" color="text.secondary">
                  {resource?.metadata?.SYMBOL || "<none>"}
                </Typography>
              </Stack>
            </Grid>
            <Grid size={{ xs: 12 }}>
              <Stack direction="row" alignItems="baseline" spacing={1}>
                <Typography variant="subtitle2">Resource Type:</Typography>
                <Typography variant="body2" color="text.secondary">
                  {resource?.resource_type}
                </Typography>
              </Stack>
            </Grid>
            <Grid size={{ xs: 12 }}>
              <Stack direction="row" alignItems="baseline" spacing={1}>
                <Typography variant="subtitle2">Total Supply:</Typography>
                <Typography variant="body2" color="text.secondary">
                  {resource?.total_supply
                    ? formatCurrency(resource.total_supply, resource.divisibility, resource.metadata["SYMBOL"])
                    : "--"}
                </Typography>
              </Stack>
            </Grid>
          </Grid>
        </Box>
      </Stack>
    </FetchStatusCheck>
  );
}

export default Resources;
