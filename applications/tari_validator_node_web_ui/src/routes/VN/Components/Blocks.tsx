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
import {
  listBlocks,
  getBlocksCount,
  getIdentity,
  getRecentTransactions,
  getTransaction,
  getUpSubstates,
} from "../../../utils/json_rpc";
import { toHexString } from "./helpers";
import { Link } from "react-router-dom";
import { primitiveDateTimeToDate, primitiveDateTimeToSecs, renderJson } from "../../../utils/helpers";
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

// TODO: fill these ()
type IBlockId = string;

export interface IQuorumCertificate {
  block_id: IBlockId,
  decision: string;
}

type INodeHeight = number;

type IEpoch = number;

type IPublicKey = string;

type IFixedHash = string;

export interface ICommand {}

export interface IBlock {
  id: IBlockId;
  parent: IBlockId;
  justify: IQuorumCertificate;
  height: INodeHeight;
  epoch: IEpoch;
  proposed_by: IPublicKey;
  total_leader_fee: number;
  merkle_root: IFixedHash;
  stored_at: number[],
  commands: ICommand[];
}

export interface ITableBlock {
  id: string;
  epoch: number;
  height: number;
  decision: string;
  total_leader_fee: number;
  proposed_by_me: boolean;
  proposed_by:string;
  transactions_cnt: number;
  block_time: number;
  stored_at: Date;
  show?: boolean;
}

interface IGetBlockReponse {
  blocks: IBlock[];
}

type ColumnKey = keyof ITableBlock;

function Blocks() {
  const [blocks, setBlocks] = useState<ITableBlock[]>([]);
  const [lastSort, setLastSort] = useState({ column: "", order: -1 });

  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);
  const [blockCount, setBlockCount] = useState(0);

  // Avoid a layout jump when reaching the last page with empty rows.
  const emptyRows = page > 0 ? Math.max(0, (1 + page) * rowsPerPage - blocks.length) : 0;

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (event: React.ChangeEvent<HTMLInputElement>) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  useEffect(() => {
    Promise.all([getIdentity(), getBlocksCount()]).then(([identity, resp]) => {
      // TODO: remove this once the pagination is done
      // resp.count = 100;
      setBlockCount(resp.count);
      listBlocks(null, resp.count).then((resp: IGetBlockReponse) => {
        let times = Object.fromEntries(resp.blocks.map((block:IBlock) => [block.id, primitiveDateTimeToSecs(block.stored_at)]));
        setBlocks(
          resp.blocks.map((block: IBlock) => {
            return {
              id: block.id,
              epoch: block.epoch,
              height: block.height,
              decision: block.justify.decision,
              total_leader_fee: block.total_leader_fee,
              proposed_by_me: block.proposed_by == identity.public_key,
              transactions_cnt: block.commands.length,
              block_time: times[block.id] - times[block.justify.block_id],
              stored_at: primitiveDateTimeToDate(block.stored_at),
              proposed_by: block.proposed_by,
              show: true,
            };
          })
        );
      });
    });
  }, []);
  const sort = (column: ColumnKey, order: number) => {
    if (column) {
      setBlocks(
        [...blocks].sort((r0: any, r1: any) => (r0[column] > r1[column] ? order : r0[column] < r1[column] ? -order : 0))
      );
      setLastSort({ column, order });
    }
  };
  return (
    <>
      <BoxHeading2>
        <SearchFilter
          stateObject={blocks}
          setStateObject={setBlocks}
          setPage={setPage}
          filterItems={[
            {
              title: "Block id",
              value: "block_id",
              filterFn: (value: string, row: ITableBlock) => row.id.toLowerCase().includes(value.toLowerCase()),
            },
            {
              title: "Epoch",
              value: "epoch",
              filterFn: (value: string, row: ITableBlock) => String(row.epoch).includes(value),
            },
            {
              title: "Height",
              value: "height",
              filterFn: (value: string, row: ITableBlock) => String(row.height).includes(value),
            },
            {
              title: "Decision",
              value: "decision",
              filterFn: (value: string, row: ITableBlock) => row.decision.includes(value),
            },
            {
              title: "# of Transactions",
              value: "transactions_cnt",
              filterFn: (value: string, row: ITableBlock) => String(row.transactions_cnt).includes(value),
            },
            {
              title: "Total fees",
              value: "total_leader_fee",
              filterFn: (value: string, row: ITableBlock) => String(row.total_leader_fee).includes(value),
            },
          ]}
          placeholder="Search for Transactions"
          defaultSearch="block_id"
        />
      </BoxHeading2>
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>
                <HeadingMenu
                  menuTitle="Block id"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("id", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("id", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="id"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Epoch"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("epoch", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("epoch", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="epoch"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Height"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("height", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("height", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="height"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Status"
                  menuItems={[
                    {
                      title: "Decision",
                      fn: () => sort("decision", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("decision", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="decision"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="# of transactions"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("transactions_cnt", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("transactions_cnt", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="transactions_cnt"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Total fees"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("total_leader_fee", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("total_leader_fee", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="total_leader_fee"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Block time"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("block_time", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("block_time", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="block_time"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Stored at"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("stored_at", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("stored_at", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="stored_at"
                  sortFunction={sort}
                />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="Proposed by"
                  menuItems={[
                    {
                      title: "Sort Ascending",
                      fn: () => sort("proposed_by", 1),
                      icon: <KeyboardArrowUpIcon />,
                    },
                    {
                      title: "Sort Descending",
                      fn: () => sort("proposed_by", -1),
                      icon: <KeyboardArrowDownIcon />,
                    },
                  ]}
                  showArrow
                  lastSort={lastSort}
                  columnName="proposed_by"
                  sortFunction={sort}
                />
              </TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {blocks
              .filter(({ show }) => show === true)
              .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
              .map(({ id, epoch, height, decision, total_leader_fee, transactions_cnt, proposed_by_me,stored_at, block_time,proposed_by }) => {
                return (
                  <TableRow key={id}>
                    <DataTableCell>
                      <Link to={`/blocks/${id}`} style={{ textDecoration: "none" }}>
                        {id.slice(0,8)}
                      </Link>
                    </DataTableCell>
                    <DataTableCell>{epoch}</DataTableCell>
                    <DataTableCell>{height}</DataTableCell>
                    <DataTableCell>
                      <StatusChip status={decision == "Accept" ? "Commit" : "Abort"} showTitle />
                    </DataTableCell>
                    <DataTableCell>{transactions_cnt}</DataTableCell>
                    <DataTableCell>
                      <div className={proposed_by_me ? "my_money" : ""}>{total_leader_fee}</div>
                    </DataTableCell>
                    <DataTableCell>{block_time} secs</DataTableCell>
                    <DataTableCell>{stored_at.toLocaleString()}</DataTableCell>
                    <DataTableCell><div className={proposed_by_me ? "my_money" : ""}>{proposed_by.slice(0,8)}</div></DataTableCell>
                  </TableRow>
                );
              })}
            {blocks.filter(({ show }) => show === true).length === 0 && (
              <TableRow>
                <TableCell colSpan={4} style={{ textAlign: "center" }}>
                  <Fade in={blocks.filter(({ show }) => show === true).length === 0} timeout={500}>
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
          count={blocks.filter((transaction) => transaction.show === true).length}
          rowsPerPage={rowsPerPage}
          page={page}
          onPageChange={handleChangePage}
          onRowsPerPageChange={handleChangeRowsPerPage}
        />
      </TableContainer>
    </>
  );
}

export default Blocks;
