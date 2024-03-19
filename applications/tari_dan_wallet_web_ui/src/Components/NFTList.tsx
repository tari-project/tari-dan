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

import React from "react";
import FetchStatusCheck from "./FetchStatusCheck";
import { Table, TableBody, TableCell, TableContainer, TableHead, TableRow } from "@mui/material";
import type { ListAccountNftResponse } from "@tariproject/typescript-bindings/wallet-daemon-client";
import type { apiError } from "../api/helpers/types";
import { DataTableCell } from "./StyledComponents";
import { renderJson } from "../utils/helpers";
import { IoCheckmarkOutline, IoCloseOutline } from "react-icons/io5";
import type { NonFungibleToken } from "@tariproject/typescript-bindings";

function NftsList({ nft }: { nft: NonFungibleToken }) {
  return (
    <TableRow>
      <DataTableCell>{JSON.stringify(nft.nft_id)}</DataTableCell>
      <DataTableCell>{nft.vault_id}</DataTableCell>
      <DataTableCell>{JSON.stringify(nft.data)}</DataTableCell>
      <DataTableCell>{JSON.stringify(nft.mutable_data)}</DataTableCell>
      <DataTableCell>
        {nft.is_burned ? (
          <IoCheckmarkOutline style={{ height: 22, width: 22 }} color="#DB7E7E" />
        ) : (
          <IoCloseOutline style={{ height: 22, width: 22 }} color="#5F9C91" />
        )}
      </DataTableCell>
    </TableRow>
  );
}

export default function NFTList({
  nftsListIsError,
  nftsListIsFetching,
  nftsListError,
  nftsListData,
}: {
  nftsListIsError: boolean;
  nftsListIsFetching: boolean;
  nftsListError: apiError | null;
  nftsListData?: ListAccountNftResponse;
}) {
  if (nftsListIsError || nftsListIsFetching) {
    <FetchStatusCheck
      isError={nftsListIsError}
      errorMessage={nftsListError?.message || "Error fetching data"}
      isLoading={nftsListIsFetching}
    />;
  }
  return (
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell>ID</TableCell>
            <TableCell>Vault</TableCell>
            <TableCell>Data</TableCell>
            <TableCell style={{ whiteSpace: "nowrap" }}>Mutable Data</TableCell>
            <TableCell style={{ whiteSpace: "nowrap" }}>Is Burned</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {nftsListData?.nfts.map((nft: NonFungibleToken, index) => (
            <NftsList key={index} nft={nft} />
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
