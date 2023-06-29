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
  TableContainer,
  TablePagination,
  Table,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Fade,
} from '@mui/material';
import { DataTableCell } from '../../Components/StyledComponents';
import Loading from '../../Components/Loading';
import StatusChip from '../../Components/StatusChip';

// Possible states:
// New,
// DryRun,
// Pending,
// Accepted,
// Rejected,
// InvalidTransaction,

export default function Transactions() {
  const [transactions, setTransactions] = useState<any>([]);
  const [error, setError] = useState<String>();
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);

  const loadTransactions = () => {
    setLoading(true); // Set loading to true before making the API call

    getAllTransactionByStatus(null)
      .then((response) => {
        console.log('response', response.transactions);
        setTransactions(
          response.transactions.map((t: any) => {
            return {
              id: t[0].sender_public_key,
              sender_public_key: t[0].sender_public_key,
              total_fees_charged: t[1].cost_breakdown.total_fees_charged,
              status: t[2],
              transaction_hash: t[1].transaction_hash,
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

  const emptyRows =
    page > 0 ? Math.max(0, (1 + page) * rowsPerPage - transactions.length) : 0;

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (
    event: React.ChangeEvent<HTMLInputElement>
  ) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  console.log('state', transactions);

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
                  .map((s: any) => {
                    return (
                      <TableRow key={s.transaction_hash}>
                        <DataTableCell>
                          <Link
                            to={`/transactions/${s.transaction_hash}`}
                            style={{ textDecoration: 'none' }}
                          >
                            {s.transaction_hash}
                          </Link>
                        </DataTableCell>
                        <DataTableCell>
                          <StatusChip status={s.status} showTitle={false} />
                        </DataTableCell>
                        <DataTableCell>{s.total_fees_charged}</DataTableCell>
                      </TableRow>
                    );
                  })}
              {emptyRows > 0 && (
                <TableRow
                  style={{
                    height: 57 * emptyRows,
                  }}
                >
                  <TableCell colSpan={4} />
                </TableRow>
              )}
            </TableBody>
          </Table>
          <TablePagination
            rowsPerPageOptions={[10, 25, 50]}
            component="div"
            count={transactions.length}
            rowsPerPage={rowsPerPage}
            page={page}
            onPageChange={handleChangePage}
            onRowsPerPageChange={handleChangeRowsPerPage}
          />
        </TableContainer>
      </Fade>
    </>
  );
}
