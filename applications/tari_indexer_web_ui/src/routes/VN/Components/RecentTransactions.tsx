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
import { toHexString } from "./helpers";
import { Link } from "react-router-dom";
import { renderJson } from "../../../utils/helpers";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, CodeBlock, AccordionIconButton } from "../../../Components/StyledComponents";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import Collapse from "@mui/material/Collapse";
import TablePagination from "@mui/material/TablePagination";
import Typography from "@mui/material/Typography";

interface ITableRecentTransaction {
  id: string;
  payload_id: string;
  timestamp: string;
  instructions: string;
  meta: string;
}

type ColumnKey = keyof ITableRecentTransaction;

function RowData({ id, payload_id, timestamp, instructions, meta }: ITableRecentTransaction) {
  const [open1, setOpen1] = useState(false);
  const [open2, setOpen2] = useState(false);

  return (
    <>
      <TableRow key={id} sx={{ borderBottom: "none" }}>
        <DataTableCell
          sx={{
            borderBottom: "none",
          }}
        >
          <Link style={{ textDecoration: "none" }} to={`/transaction/${payload_id}`}>
            {payload_id}
          </Link>
        </DataTableCell>
        <DataTableCell
          sx={{
            borderBottom: "none",
          }}
        >
          {timestamp.replace("T", " ")}
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: "none", textAlign: "center" }}>
          <AccordionIconButton
            open={open1}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen1(!open1);
              setOpen2(false);
            }}
          >
            {open1 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: "none", textAlign: "center" }}>
          <AccordionIconButton
            open={open2}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen2(!open2);
              setOpen1(false);
            }}
          >
            {open2 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </DataTableCell>
      </TableRow>
      <TableRow key={`${id}-2`}>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
            borderBottom: "none",
          }}
          colSpan={4}
        >
          <Collapse in={open1} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: "10px" }}>{renderJson(JSON.parse(meta))}</CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
      <TableRow key={`${id}-3`}>
        <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={4}>
          <Collapse in={open2} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: "10px" }}>{renderJson(JSON.parse(instructions))}</CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

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
    // TODO: Was this ever working? We have this on VN but not in indexer
    // getRecentTransactions().then((resp) => {
    //   console.log("Response: ", resp);
    //   setRecentTransactions(
    //     // Display from newest to oldest by reversing
    //     resp.transactions
    //       .slice()
    //       .reverse()
    //       .map(({ instructions, meta, payload_id, timestamp }: IRecentTransaction) => ({
    //         id: toHexString(payload_id),
    //         payload_id: toHexString(payload_id),
    //         timestamp: timestamp,
    //         meta: meta,
    //         instructions: instructions,
    //       })),
    //   );
    // });
  }, []);
  const sort = (column: ColumnKey) => {
    let order = 1;
    if (lastSort.column === column) {
      order = -lastSort.order;
    }
    setRecentTransactions(
      [...recentTransactions].sort((r0, r1) =>
        r0[column] > r1[column] ? order : r0[column] < r1[column] ? -order : 0,
      ),
    );
    setLastSort({ column, order });
  };
  if (recentTransactions === undefined) {
    return <Typography variant="h4">Recent transactions ... loading</Typography>;
  }

  return (
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell onClick={() => sort("payload_id")}>
              <div
                style={{
                  display: "flex",
                  justifyContent: "flex-start",
                  alignItems: "center",
                  gap: "5px",
                }}
              >
                Payload id
                {lastSort.column === "payload_id" ? (
                  lastSort.order === 1 ? (
                    <KeyboardArrowUpIcon />
                  ) : (
                    <KeyboardArrowDownIcon />
                  )
                ) : (
                  ""
                )}
              </div>
            </TableCell>
            <TableCell onClick={() => sort("timestamp")}>
              <div
                style={{
                  display: "flex",
                  justifyContent: "flex-start",
                  alignItems: "center",
                  gap: "5px",
                }}
              >
                Timestamp
                {lastSort.column === "timestamp" ? (
                  lastSort.order === 1 ? (
                    <KeyboardArrowUpIcon />
                  ) : (
                    <KeyboardArrowDownIcon />
                  )
                ) : (
                  ""
                )}
              </div>
            </TableCell>
            <TableCell style={{ textAlign: "center" }}>Meta</TableCell>
            <TableCell style={{ textAlign: "center" }}>Instructions</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {recentTransactions
            .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
            .map(({ id, payload_id, timestamp, instructions, meta }) => (
              <RowData
                key={id}
                id={id}
                payload_id={payload_id}
                timestamp={timestamp}
                instructions={instructions}
                meta={meta}
              />
            ))}
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
        count={recentTransactions.length}
        rowsPerPage={rowsPerPage}
        page={page}
        onPageChange={handleChangePage}
        onRowsPerPageChange={handleChangeRowsPerPage}
      />
    </TableContainer>
  );
}

export default RecentTransactions;
