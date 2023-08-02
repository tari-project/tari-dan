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
import { getRecentTransactions, getTransaction, getUpSubstates } from "../../../utils/json_rpc";
import { toHexString } from "./helpers";
import { Link } from "react-router-dom";
import { renderJson } from "../../../utils/helpers";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, CodeBlock, AccordionIconButton, BoxHeading2 } from "../../../Components/StyledComponents";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import Collapse from "@mui/material/Collapse";
import TablePagination from "@mui/material/TablePagination";
import Typography from "@mui/material/Typography";
import HeadingMenu from "../../../Components/HeadingMenu";
import SearchFilter from "../../../Components/SearchFilter";
import Fade from "@mui/material/Fade";
import StatusChip from "../../../Components/StatusChip";

// TODO: fill this, and change instructions in IRecentTransaction
// interface IInstruction {
// }

interface ISignature {
  public_nonce: string;
  signature: string;
}
interface ITransactionSignature {
  public_key: string;
  signature: ISignature;
}

interface IRecentTransaction {
  id: string;
  fee_instructions: any[];
  instructions: any[];
  signature: ITransactionSignature;
  inputs: string[];
  input_refs: string[];
  outputs: string[];
  filled_inputs: string[];
  filled_outputs: string[];
}

export interface ITableRecentTransaction {
  transaction_hash: string;
  status: any;
  total_fees_charged: number;
  show?: boolean;
}

type ColumnKey = keyof ITableRecentTransaction;

// function RowData({
//   id,
//   payload_id,
//   timestamp,
//   instructions,
//   meta,
// }: ITableRecentTransaction) {
//   const [open1, setOpen1] = useState(false);
//   const [open2, setOpen2] = useState(false);

//   return (
//     <>
//       <TableRow key={id} sx={{ borderBottom: 'none' }}>
//         <DataTableCell
//           sx={{
//             borderBottom: 'none',
//           }}
//         >
//           <Link
//             style={{ textDecoration: 'none' }}
//             to={`/transactions/${payload_id}`}
//           >
//             {payload_id}
//           </Link>
//         </DataTableCell>
//         <DataTableCell
//           sx={{
//             borderBottom: 'none',
//           }}
//         >
//           {timestamp.replace('T', ' ')}
//         </DataTableCell>
//         <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
//           <AccordionIconButton
//             open={open1}
//             aria-label="expand row"
//             size="small"
//             onClick={() => {
//               setOpen1(!open1);
//               setOpen2(false);
//             }}
//           >
//             {open1 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
//           </AccordionIconButton>
//         </DataTableCell>
//         <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
//           <AccordionIconButton
//             open={open2}
//             aria-label="expand row"
//             size="small"
//             onClick={() => {
//               setOpen2(!open2);
//               setOpen1(false);
//             }}
//           >
//             {open2 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
//           </AccordionIconButton>
//         </DataTableCell>
//       </TableRow>
//       <TableRow key={`${id}-2`}>
//         <DataTableCell
//           style={{
//             paddingBottom: 0,
//             paddingTop: 0,
//             borderBottom: 'none',
//           }}
//           colSpan={4}
//         >
//           <Collapse in={open1} timeout="auto" unmountOnExit>
//             <CodeBlock style={{ marginBottom: '10px' }}>
//               {renderJson(JSON.parse(meta))}
//             </CodeBlock>
//           </Collapse>
//         </DataTableCell>
//       </TableRow>
//       <TableRow key={`${id}-3`}>
//         <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={4}>
//           <Collapse in={open2} timeout="auto" unmountOnExit>
//             <CodeBlock style={{ marginBottom: '10px' }}>
//               {renderJson(JSON.parse(instructions))}
//             </CodeBlock>
//           </Collapse>
//         </DataTableCell>
//       </TableRow>
//     </>
//   );
// }

function RecentTransactions() {
  const [recentTransactions, setRecentTransactions] = useState<ITableRecentTransaction[]>([]);
  const [lastSort, setLastSort] = useState({ column: "", order: -1 });

  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);

  // Avoid a layout jump when reaching the last page with empty rows.
  const emptyRows = page > 0 ? Math.max(0, (1 + page) * rowsPerPage - recentTransactions.length) : 0;

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (event: React.ChangeEvent<HTMLInputElement>) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  useEffect(() => {
    getRecentTransactions().then((resp) => {
      console.log("resp", resp);
      setRecentTransactions(
        // Display from newest to oldest by reversing
        resp.transactions
          .slice()
          .reverse()
          .map(
            ({
              id,
              fee_instructions,
              instructions,
              signature,
              inputs,
              input_refs,
              outputs,
              filled_inputs,
              filled_outputs,
            }: IRecentTransaction) => ({
              transaction_hash: id,
              total_fees_charged: null,
              status: "Loading",
              show: true,
            })
          )
      );
      for (let tx in resp.transactions) {
        Promise.all([getTransaction(resp.transactions[tx].id), getUpSubstates(String(resp.transactions[tx].id))]).then(([transaction, substates]) => {
          setRecentTransactions((prevState: any) =>
            prevState.map((item: any, index: any) => {
              if (tx == index) {
                return {
                  ...item,
                  status: transaction["transaction"]["final_decision"],
                  total_fees_charged: substates["substates"].reduce((acc:number,cur:any) => acc+Number(cur?.substate_value?.TransactionReceipt?.fee_receipt?.fee_resource?.Confidential?.revealed_amount || 0), 0),
                };
              }
              return item;
            })
          );
        });
      }
    });
  }, []);
  const sort = (column: ColumnKey, order: number) => {
    // let order = 1;
    // if (lastSort.column === column) {
    //   order = -lastSort.order;
    // }
    if (column) {
      setRecentTransactions(
        [...recentTransactions].sort((r0: any, r1: any) =>
          r0[column] > r1[column] ? order : r0[column] < r1[column] ? -order : 0
        )
      );
      setLastSort({ column, order });
    }
  };
  return (
    <>
      <BoxHeading2>
        <SearchFilter
          stateObject={recentTransactions}
          setStateObject={setRecentTransactions}
          setPage={setPage}
          filterItems={[
            {
              title: "Transaction hash",
              value: "transaction_hash",
              filterFn: (value: string, row: ITableRecentTransaction) => {
                return row.transaction_hash.toLowerCase().includes(value.toLowerCase())
              },
            },
            {
              title: "Status",
              value: "status",
              filterFn: (value: string, row: ITableRecentTransaction) => row.status.includes(value),
            },
            {
              title: "Total fees",
              value: "total_fees_charged",
              filterFn: (value: string, row: ITableRecentTransaction) => String(row.total_fees_charged) == value,
            },
          ]}
          placeholder="Search for Transactions"
          defaultSearch="transaction_hash"
        />
      </BoxHeading2>
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>
                <HeadingMenu
                  menuTitle="Transaction Hash"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("transaction_hash", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("transaction_hash", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="transaction_hash"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Status"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("status", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("status", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="status"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
              <HeadingMenu
                  menuTitle="Total fees"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("total_fees_charged", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("total_fees_charged", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="total_fees_charged"
                  sortFunction={sort}
                />
              </TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {recentTransactions
              .filter(({ show }) => show === true)
              .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
              .map(({ transaction_hash, status, total_fees_charged }) => {
                return (
                  <TableRow key={transaction_hash}>
                    <DataTableCell>
                      <Link to={`/transactions/${transaction_hash}`} style={{ textDecoration: "none" }}>
                        {transaction_hash}
                      </Link>
                    </DataTableCell>
                    <DataTableCell>
                      <StatusChip status={status} showTitle />
                    </DataTableCell>
                    <DataTableCell>{total_fees_charged}</DataTableCell>
                  </TableRow>
                );
              })}
            {recentTransactions.filter(({ show }) => show === true).length === 0 && (
              <TableRow>
                <TableCell colSpan={4} style={{ textAlign: "center" }}>
                  <Fade in={recentTransactions.filter(({ show }) => show === true).length === 0} timeout={500}>
                    <Typography variant="h5">No results found</Typography>
                  </Fade>
                </TableCell>
              </TableRow>
            )}
            {emptyRows > 0 && (
              <TableRow
                style={{
                  height: 67 * emptyRows,
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
          count={recentTransactions.filter((transaction) => transaction.show === true).length}
          rowsPerPage={rowsPerPage}
          page={page}
          onPageChange={handleChangePage}
          onRowsPerPageChange={handleChangeRowsPerPage}
        />
      </TableContainer>
    </>
  );
}

export default RecentTransactions;
