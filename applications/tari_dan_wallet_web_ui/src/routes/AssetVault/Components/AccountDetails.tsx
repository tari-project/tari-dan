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

import Box from "@mui/material/Box";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import { GridHeadCell, GridDataCell } from "../../../Components/StyledComponents";
import { useAccountsGet } from "../../../api/hooks/useAccounts";
import { shortenString } from "../../../utils/helpers";
import { styled } from "@mui/material/styles";
import { substateIdToString } from "@tariproject/typescript-bindings";

const GridContainer = styled(Box)(({ theme }) => ({
  display: "grid",
  gridTemplateColumns: "1fr 2fr",
  gridTemplateAreas: `'head1 content1'
    'head2 content2'
    'head3 content3'`,

  [theme.breakpoints.up("md")]: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr 1fr",
    gridTemplateAreas: `'head1 head2 head3'
      'content1 content2 content3'`,
  },
}));

function AccountDetails({ accountName }: { accountName: string }) {
  const {
    data: accountsData,
    isError: accountsIsError,
    error: accountsError,
    isFetching: accountsIsFetching,
  } = useAccountsGet(accountName);

  return (
    <>
      {accountsIsError || accountsIsFetching ? (
        <FetchStatusCheck
          isError={accountsIsError}
          errorMessage={accountsError?.message || "Error fetching data"}
          isLoading={accountsIsFetching}
        />
      ) : (
        <>
          {accountsData && (
            <GridContainer>
              <GridHeadCell className="head1">Name</GridHeadCell>
              <GridHeadCell className="head2">Address</GridHeadCell>
              <GridHeadCell className="head3">Public Key</GridHeadCell>
              <GridDataCell className="content1">{accountsData.account.name}</GridDataCell>
              <GridDataCell className="content2">
                {shortenString(substateIdToString(accountsData.account.address))}
                <CopyToClipboard copy={substateIdToString(accountsData.account.address)} />
              </GridDataCell>
              <GridDataCell className="content3">
                {shortenString(accountsData.public_key)}
                <CopyToClipboard copy={accountsData.public_key} />
              </GridDataCell>
            </GridContainer>
          )}
        </>
      )}
    </>
  );
}

export default AccountDetails;
