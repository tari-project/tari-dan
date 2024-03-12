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

import PageHeading from "../../Components/PageHeading";
import Grid from "@mui/material/Grid";
import {StyledPaper} from "../../Components/StyledComponents";
import TableContainer from "@mui/material/TableContainer";
import Table from "@mui/material/Table";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import TableCell from "@mui/material/TableCell";
import TableBody from "@mui/material/TableBody";
import {useParams} from "react-router-dom";
import {useAccountsGetBalances, useAccountsGet, useAccountNFTsList} from "../../api/hooks/useAccounts";
import {shortenString} from "../../utils/helpers";
import {DataTableCell} from "../../Components/StyledComponents";
import CopyToClipboard from "../../Components/CopyToClipboard";
import FetchStatusCheck from "../../Components/FetchStatusCheck";
import {NonFungibleToken, substateIdToString} from "@tariproject/typescript-bindings";
import type {BalanceEntry} from "@tariproject/typescript-bindings/wallet-daemon-client";

function BalanceRow(props: BalanceEntry) {
    return (
        <TableRow key={props.resource_address}>
            <DataTableCell>
                {shortenString(props.token_symbol || props.resource_address)}
                <CopyToClipboard copy={props.token_symbol || props.resource_address}/>
            </DataTableCell>
            <DataTableCell>{props.resource_type}</DataTableCell>
            <DataTableCell>{props.balance}</DataTableCell>
            <DataTableCell>{props.confidential_balance}</DataTableCell>
        </TableRow>
    );
}

function NftDetails({nft}: { nft: NonFungibleToken }) {
    return (
        <TableRow>
            <DataTableCell>{JSON.stringify(nft.nft_id)}</DataTableCell>
            <DataTableCell>{nft.vault_id}</DataTableCell>
            <DataTableCell>{JSON.stringify(nft.data)}</DataTableCell>
            <DataTableCell>{JSON.stringify(nft.mutable_data)}</DataTableCell>
            <DataTableCell>{nft.is_burned ? "Yes" : "No"}</DataTableCell>
        </TableRow>
    );
}

function AccountDetailsLayout() {
    const {name} = useParams();
    const {
        data: balancesData,
        isLoading: balancesIsLoading,
        isError: balancesIsError,
        error: balancesError,
    } = useAccountsGetBalances(name || "");

    const {
        data: nftsListData,
        isLoading: nftsListIsLoading,
        isError: nftsListIsError,
        error: nftsListError,
    } = useAccountNFTsList(0, 10);

    const {
        data: accountsData,
        isLoading: accountsIsLoading,
        isError: accountsIsError,
        error: accountsError,
    } = useAccountsGet(name || "");

    return (
        <>
            <Grid item xs={12} md={12} lg={12}>
                <PageHeading>Account Details</PageHeading>
            </Grid>
            <Grid item xs={12} md={12} lg={12}>
                <StyledPaper>
                    <FetchStatusCheck
                        isError={accountsIsError}
                        errorMessage={accountsError?.message || "Error fetching data"}
                        isLoading={accountsIsLoading}
                    />
                    <TableContainer>
                        <Table>
                            <TableHead>
                                <TableRow>
                                    <TableCell>Name</TableCell>
                                    <TableCell>Address</TableCell>
                                    <TableCell>Public key</TableCell>
                                </TableRow>
                            </TableHead>
                            <TableBody>
                                {accountsData && (
                                    <TableRow>
                                        <DataTableCell>{accountsData.account.name}</DataTableCell>
                                        <DataTableCell>
                                            {shortenString(substateIdToString(accountsData.account.address))}
                                            <CopyToClipboard copy={substateIdToString(accountsData.account.address)}/>
                                        </DataTableCell>
                                        <DataTableCell>
                                            {shortenString(accountsData.public_key)}
                                            <CopyToClipboard copy={accountsData.public_key}/>
                                        </DataTableCell>
                                    </TableRow>
                                )}
                            </TableBody>
                        </Table>
                    </TableContainer>
                </StyledPaper>
            </Grid>
            <Grid item xs={12} md={12} lg={12}>
                <StyledPaper>
                    Balances
                    <FetchStatusCheck
                        isError={balancesIsError}
                        errorMessage={balancesError?.message || "Error fetching data"}
                        isLoading={balancesIsLoading}
                    />
                    <TableContainer>
                        <Table>
                            <TableHead>
                                <TableRow>
                                    <TableCell>Resource</TableCell>
                                    <TableCell>Resource Type</TableCell>
                                    <TableCell>Revealed Balance</TableCell>
                                    <TableCell>Confidential Balance</TableCell>
                                </TableRow>
                            </TableHead>
                            <TableBody>{balancesData?.balances.map((balance: BalanceEntry) => BalanceRow(balance))}</TableBody>
                        </Table>
                    </TableContainer>
                </StyledPaper>
            </Grid>
            <Grid item xs={12} md={12} lg={12}>
                <StyledPaper>
                    Account NFTs
                    <FetchStatusCheck
                        isError={nftsListIsError}
                        errorMessage={nftsListError?.message || "Error fetching data"}
                        isLoading={nftsListIsLoading}
                    />
                    <TableContainer>
                        <Table>
                            <TableHead>
                                <TableRow>
                                    <TableCell>ID</TableCell>
                                    <TableCell>Vault</TableCell>
                                    <TableCell>Data</TableCell>
                                    <TableCell>Mutable Data</TableCell>
                                    <TableCell>Is Burned</TableCell>
                                </TableRow>
                            </TableHead>
                            <TableBody>
                                {nftsListData?.nfts.map((nft: NonFungibleToken, index: number) =>
                                    <NftDetails key={index} nft={nft}/>
                                )}
                            </TableBody>
                        </Table>
                    </TableContainer>
                </StyledPaper>
            </Grid>
        </>
    );
}

export default AccountDetailsLayout;
