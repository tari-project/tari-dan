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
import type { AccountNftInfo, ListAccountNftResponse } from "@tariproject/typescript-bindings/wallet-daemon-client";
import type { apiError } from "../api/helpers/types";
import { DataTableCell } from "./StyledComponents";
import { renderJson } from "../utils/helpers";
import { IoCheckmarkOutline, IoCloseOutline } from "react-icons/io5";

function NftsList({ metadata, is_burned }: AccountNftInfo) {
  return (
    <TableRow>
      <DataTableCell>{metadata.name || <i>No name</i>}</DataTableCell>
      <DataTableCell>
        {metadata.image_url ? (
          <a href={metadata.image_url} target="_blank" rel="noopener noreferrer">
            <img src={metadata.image_url} style={{ maxWidth: "100px", maxHeight: "100px", objectFit: "contain" }} />
          </a>
        ) : (
          <i>No image</i>
        )}
      </DataTableCell>
      <DataTableCell>{renderJson(metadata)}</DataTableCell>
      <DataTableCell>
        {is_burned ? (
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
            <TableCell>Name</TableCell>
            <TableCell>Image</TableCell>
            <TableCell>Metadata</TableCell>
            <TableCell style={{ whiteSpace: "nowrap" }}>Is Burned</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {nftsListData?.nfts.map(({ metadata, is_burned }: AccountNftInfo, index) => (
            <NftsList key={index} metadata={metadata} is_burned={is_burned} />
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
