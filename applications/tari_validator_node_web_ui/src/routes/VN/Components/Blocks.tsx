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
import { listBlocks, getBlocksCount, getIdentity, getBlocks, getFilteredBlocksCount } from "../../../utils/json_rpc";
import { Link } from "react-router-dom";
import { emptyRows, primitiveDateTimeToDate, primitiveDateTimeToSecs } from "../../../utils/helpers";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, BoxHeading2 } from "../../../Components/StyledComponents";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import TablePagination from "@mui/material/TablePagination";
import Typography from "@mui/material/Typography";
import HeadingMenu from "../../../Components/HeadingMenu";
import Filter from "../../../Components/Filter";
import Fade from "@mui/material/Fade";
import StatusChip from "../../../Components/StatusChip";
import { Ordering, type Block } from "@tari-project/typescript-bindings";
import type { VNGetIdentityResponse } from "@tari-project/typescript-bindings";

function Blocks() {
  const [blocks, setBlocks] = useState<Block[]>([]);
  const [blocksCount, setBlocksCount] = useState(0);
  const [lastSort, setLastSort] = useState({ column: "height", order: -1 });
  const [identity, setIdentity] = useState<VNGetIdentityResponse>();

  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);
  const [ordering, setOrdering] = useState<Ordering>("Descending");
  const [orderingIndex, setOrderingIndex] = useState(2);
  const [filter, setFilter] = useState<string | null>(null);
  const [filterIndex, setFilterIndex] = useState(0);

  // Avoid a layout jump when reaching the last page with empty rows.
  const emptyRowsCnt = rowsPerPage - blocks.length;

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (event: React.ChangeEvent<HTMLInputElement>) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  useEffect(() => {
    getIdentity().then((resp) => setIdentity(resp));
  }, []);

  useEffect(() => {
    getFilteredBlocksCount({ filter: filter, filter_index: filterIndex }).then((resp) => {
      setBlocksCount(resp.count);
      if (rowsPerPage * page > resp.count) {
        setPage(Math.floor(resp.count / rowsPerPage));
      }
      getBlocks({
        limit: rowsPerPage,
        offset: page * rowsPerPage,
        ordering_index: orderingIndex,
        ordering: ordering,
        filter_index: filterIndex,
        filter: filter,
      }).then((resp) => {
        setBlocks(resp.blocks);
      });
    });
  }, [page, rowsPerPage, ordering, orderingIndex, filter, filterIndex]);

  const columnNameToId = (column: string) => {
    switch (column) {
      case "id":
        return 0;
      case "epoch":
        return 1;
      case "height":
        return 2;
      case "transactions_cnt":
        return 4;
      case "total_leader_fee":
        return 5;
      case "block_time":
        return 6;
      case "stored_at":
        return 7;
      case "proposed_by":
        return 8;
    }
    return 0;
  };

  const sort = (column: string, order: number) => {
    setOrderingIndex(columnNameToId(column));
    setOrdering(order == 1 ? "Ascending" : "Descending");
    setLastSort({ column, order });
  };

  return (
    <>
      <BoxHeading2>
        <Filter
          filterItems={[
            {
              title: "Block id",
              value: "id",
            },
            {
              title: "Epoch",
              value: "epoch",
            },
            {
              title: "Height",
              value: "height",
            },
            {
              title: "Min # of Commands",
              value: "transactions_cnt",
            },
            {
              title: "Min total fees",
              value: "total_leader_fee",
            },
            {
              title: "Proposed by",
              value: "proposed_by",
            },
          ]}
          placeholder="Search for Blocks"
          defaultSearch="id"
          setSearchValue={(value) => {
            setFilter(value);
          }}
          setSearchColumn={(name) => {
            setFilterIndex(columnNameToId(name));
          }}
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
                <HeadingMenu menuTitle="Status" showArrow={false} columnName="decision" />
              </TableCell>
              <TableCell>
                <HeadingMenu
                  menuTitle="# of commands"
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
            {blocks.map((block) => {
              return (
                <TableRow key={block.id}>
                  <DataTableCell>
                    <Link to={`/blocks/${block.id}`} style={{ textDecoration: "none" }}>
                      {block.id.slice(0, 8)}
                    </Link>
                  </DataTableCell>
                  <DataTableCell>{block.epoch}</DataTableCell>
                  <DataTableCell>{block.height}</DataTableCell>
                  <DataTableCell>
                    <StatusChip
                      status={block.is_dummy ? "Dummy" : block.is_committed ? "Commit" : "Pending"}
                      showTitle
                    />
                  </DataTableCell>
                  <DataTableCell>{block.commands.length}</DataTableCell>
                  <DataTableCell>
                    <div className={block.proposed_by == identity?.public_key ? "my_money" : ""}>
                      {block.total_leader_fee}
                    </div>
                  </DataTableCell>
                  <DataTableCell>{block.block_time} secs</DataTableCell>
                  <DataTableCell>{primitiveDateTimeToDate(block.stored_at || []).toLocaleString()}</DataTableCell>
                  <DataTableCell>
                    <div className={block.proposed_by == identity?.public_key ? "my_money" : ""}>
                      {block.proposed_by.slice(0, 8)}
                    </div>
                  </DataTableCell>
                </TableRow>
              );
            })}
            {blocks.length == 0 && (
              <TableRow>
                <TableCell colSpan={4} style={{ textAlign: "center" }}>
                  <Fade in={blocks.length == 0} timeout={500}>
                    <Typography variant="h5">No results found</Typography>
                  </Fade>
                </TableCell>
              </TableRow>
            )}
            {emptyRowsCnt > 0 && (
              <TableRow
                style={{
                  height: 67 * emptyRowsCnt,
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
          count={blocksCount}
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
