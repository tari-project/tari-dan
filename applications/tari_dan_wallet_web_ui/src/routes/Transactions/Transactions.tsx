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

import {ChevronRight} from "@mui/icons-material";
import {
    Fade,
    IconButton,
    Table,
    TableBody,
    TableCell,
    TableContainer,
    TableHead,
    TablePagination,
    TableRow,
} from "@mui/material";
import {useTheme} from "@mui/material/styles";
import {useEffect, useState} from "react";
import {Link} from "react-router-dom";
import FetchStatusCheck from "../../Components/FetchStatusCheck";
import StatusChip from "../../Components/StatusChip";
import {DataTableCell} from "../../Components/StyledComponents";
import {useGetAllTransactions} from "../../api/hooks/useTransactions";
import {emptyRows, handleChangePage, handleChangeRowsPerPage} from "../../utils/helpers";
import {useAccountsGet} from "../../api/hooks/useAccounts";
import type {FinalizeResult, Transaction, TransactionStatus} from "@tariproject/typescript-bindings";

export default function Transactions({accountName}: { accountName: string }) {
    const {data: accountsData} = useAccountsGet(accountName);
    const [page, setPage] = useState(0);
    const [rowsPerPage, setRowsPerPage] = useState(10);
    const {data, isLoading, error, isError, refetch} = useGetAllTransactions(
        null,
        (accountsData && "Component" in accountsData?.account.address && accountsData?.account.address.Component) || null,
    );
    useEffect(() => {
        refetch();
    }, [accountsData?.account.address]);
    const theme = useTheme();

    return (
        <>
            <FetchStatusCheck
                isLoading={isLoading}
                isError={isError}
                errorMessage={error?.message || "Error fetching data"}
            />
            <Fade in={!isLoading && !isError}>
                <TableContainer>
                    <Table>
                        <TableHead>
                            <TableRow>
                                <TableCell>Transaction Hash</TableCell>
                                <TableCell>Status</TableCell>
                                <TableCell>Total Fees</TableCell>
                                <TableCell>Details</TableCell>
                            </TableRow>
                        </TableHead>
                        <TableBody>
                            {data?.transactions
                                ?.slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
                                .map((t: [Transaction, FinalizeResult | null, TransactionStatus, string]) => {
                                    if (t?.[0]?.id !== undefined) {
                                        const hash = t[0].id;
                                        return (
                                            <TableRow key={hash}>
                                                <DataTableCell>
                                                    <Link
                                                        to={`/transactions/${hash}`}
                                                        style={{
                                                            textDecoration: "none",
                                                            color: theme.palette.text.secondary,
                                                        }}
                                                    >
                                                        {hash}
                                                    </Link>
                                                </DataTableCell>
                                                <DataTableCell>
                                                    <StatusChip status={t[2]} showTitle/>
                                                </DataTableCell>
                                                <DataTableCell>{t?.[1]?.fee_receipt.total_fees_paid || 0}</DataTableCell>
                                                <DataTableCell>
                                                    <IconButton
                                                        component={Link}
                                                        to={`/transactions/${hash}`}
                                                        style={{
                                                            color: theme.palette.text.secondary,
                                                        }}
                                                    >
                                                        <ChevronRight/>
                                                    </IconButton>
                                                </DataTableCell>
                                            </TableRow>
                                        );
                                    }
                                })}
                            {emptyRows(page, rowsPerPage, data?.transactions) > 0 && (
                                <TableRow
                                    style={{
                                        height: 57 * emptyRows(page, rowsPerPage, data?.transactions),
                                    }}
                                >
                                    <TableCell colSpan={3}/>
                                </TableRow>
                            )}
                        </TableBody>
                    </Table>
                    {data?.transactions && (
                        <TablePagination
                            rowsPerPageOptions={[10, 25, 50]}
                            component="div"
                            count={data.transactions.length}
                            rowsPerPage={rowsPerPage}
                            page={page}
                            onPageChange={(event, newPage) => handleChangePage(event, newPage, setPage)}
                            onRowsPerPageChange={(event) => handleChangeRowsPerPage(event, setRowsPerPage, setPage)}
                        />
                    )}
                </TableContainer>
            </Fade>
        </>
    );
}
