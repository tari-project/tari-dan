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

import { useState, useEffect } from 'react';
import { Link } from 'react-router-dom';
import { getAllTransactionByStatus } from '../../utils/json_rpc';
import {
  emptyRows,
  handleChangePage,
  handleChangeRowsPerPage,
} from '../../utils/helpers';
import {
  TableContainer,
  TablePagination,
  Table,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Fade,
  Alert,
} from '@mui/material';
import { DataTableCell } from '../../Components/StyledComponents';
import Loading from '../../Components/Loading';
import StatusChip from '../../Components/StatusChip';

export default function Transactions() {
  const [transactions, setTransactions] = useState<any>([]);
  const [error, setError] = useState<String>();
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);

  const loadTransactions = () => {
    setLoading(true);
    getAllTransactionByStatus(null)
      .then((response) => {
        setTransactions(
          response.transactions.map((t: any) => {
            return {
              sender_public_key: t[0].sender_public_key,
              total_fees_charged:
                t[1]?.cost_breakdown === null
                  ? 0
                  : t[1]?.cost_breakdown.total_fees_charged,
              status: t[2],
              transaction_hash: t[0].id,
            };
          })
        );
        setError(undefined);
      })
      .catch((err) => {
        setError(
          err && err.message
            ? err.message
            : `Unknown error: ${JSON.stringify(err)}`
        );
      })
      .finally(() => {
        setLoading(false);
      });
  };

  useEffect(() => {
    loadTransactions();
  }, []);

  return (
    <>
      {loading && <Loading />}
      <Fade in={!loading}>
        <TableContainer>
          <Table>
            <TableHead>
              <TableRow>
                <TableCell>Transaction Hash</TableCell>
                <TableCell>Status</TableCell>
                <TableCell>Total Fees</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {transactions &&
                transactions
                  .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
                  .map(
                    ({ transaction_hash, status, total_fees_charged }: any) => {
                      return (
                        <TableRow key={transaction_hash}>
                          <DataTableCell>
                            <Link
                              to={`/transactions/${transaction_hash}`}
                              style={{ textDecoration: 'none' }}
                            >
                              {transaction_hash}
                            </Link>
                          </DataTableCell>
                          <DataTableCell>
                            <StatusChip status={status} showTitle />
                          </DataTableCell>
                          <DataTableCell>{total_fees_charged}</DataTableCell>
                        </TableRow>
                      );
                    }
                  )}
              {emptyRows(page, rowsPerPage, transactions) > 0 && (
                <TableRow
                  style={{
                    height: 57 * emptyRows(page, rowsPerPage, transactions),
                  }}
                >
                  <TableCell colSpan={3} />
                </TableRow>
              )}
              {error ? (
                <TableRow>
                  <TableCell colSpan={3}>
                    <Alert severity="error">{error}</Alert>
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
          <TablePagination
            rowsPerPageOptions={[10, 25, 50]}
            component="div"
            count={transactions.length}
            rowsPerPage={rowsPerPage}
            page={page}
            onPageChange={(event, newPage) =>
              handleChangePage(event, newPage, setPage)
            }
            onRowsPerPageChange={(event) =>
              handleChangeRowsPerPage(event, setRowsPerPage, setPage)
            }
          />
        </TableContainer>
      </Fade>
    </>
  );
}
