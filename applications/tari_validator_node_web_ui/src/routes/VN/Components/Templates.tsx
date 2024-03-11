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
import { getTemplates } from "../../../utils/json_rpc";
import { shortenString } from "./helpers";
import "./Templates.css";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, BoxHeading2 } from "../../../Components/StyledComponents";
import { Link } from "react-router-dom";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import IconButton from "@mui/material/IconButton";
import KeyboardArrowRightIcon from "@mui/icons-material/KeyboardArrowRight";
import HeadingMenu from "../../../Components/HeadingMenu";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import TablePagination from "@mui/material/TablePagination";
import SearchFilter from "../../../Components/SearchFilter";
import Typography from "@mui/material/Typography";
import Fade from "@mui/material/Fade";
import FileDownloadOutlinedIcon from "@mui/icons-material/FileDownloadOutlined";
import { emptyRows } from "../../../utils/helpers";
import type { TemplateMetadata } from "@tariproject/typescript-bindings/validator-node-client";

export interface ITemplate {
  id: string;
  name: string;
  address: Uint8Array;
  url: string;
  binary_sha: Array<number>;
  height: number;
  show: boolean;
}

type ColumnKey = keyof ITemplate;

function Templates() {
  const [templates, setTemplates] = useState<ITemplate[]>([]);
  const [lastSort, setLastSort] = useState({ column: "", order: -1 });

  useEffect(() => {
    getTemplates({ limit: 10 }).then((response) => {
      setTemplates(
        response.templates
          .slice()
          .sort((a: TemplateMetadata, b: TemplateMetadata) => b.height - a.height)
          .map((template: TemplateMetadata) => ({
            id: toHex(template.address),
            name: template.name,
            address: template.address,
            url: template.url,
            binary_sha: template.binary_sha,
            height: template.height,
            template: template,
            show: true,
          })),
      );
    });
  }, []);

  const toHex = (str: Uint8Array) => {
    return "0x" + Array.prototype.map.call(str, (x: number) => ("00" + x.toString(16)).slice(-2)).join("");
  };

  const sort = (column: ColumnKey, order: number) => {
    // let order = 1;
    // if (lastSort.column === column) {
    //   order = -lastSort.order;
    // }
    if (column) {
      setTemplates(
        [...templates].sort((r0: ITemplate, r1: ITemplate) =>
          r0[column] > r1[column] ? order : r0[column] < r1[column] ? -order : 0,
        ),
      );
      setLastSort({ column, order });
    }
  };

  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);

  // Avoid a layout jump when reaching the last page with empty rows.
  const emptyRowsCnt = emptyRows(page, rowsPerPage, templates);

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (event: React.ChangeEvent<HTMLInputElement>) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  return (
    <>
      <BoxHeading2>
        <SearchFilter
          stateObject={templates}
          setStateObject={setTemplates}
          setPage={setPage}
          filterItems={[
            {
              title: "Template Address",
              value: "id",
              filterFn: (value: string, row: ITemplate) => row.id.toLowerCase().includes(value.toLowerCase()),
            },
            {
              title: "Mined Height",
              value: "height",
              filterFn: (value: string, row: ITemplate) => row.height.toString().includes(value),
            },
          ]}
          placeholder="Search for Templates"
        />
      </BoxHeading2>
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell style={{ minWidth: "300px" }}>
                <HeadingMenu
                  menuTitle="Address"
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
              <TableCell>Name</TableCell>
              <TableCell>Download URL</TableCell>
              <TableCell style={{ textAlign: "center", width: "210px" }}>
                <HeadingMenu
                  menuTitle="Mined Height"
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
              <TableCell style={{ textAlign: "center" }}>Status</TableCell>
              <TableCell style={{ textAlign: "center" }}>Functions</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {templates
              .filter(({ show }) => show)
              .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
              .map(({ address, binary_sha, height, url, id, name }) => (
                <TableRow key={id}>
                  <DataTableCell>
                    <Link to={`/templates/${toHex(address)}`} state={[address]} style={{ textDecoration: "none" }}>
                      {shortenString(toHex(address))}
                    </Link>
                    <CopyToClipboard copy={toHex(address)} />
                  </DataTableCell>
                  <DataTableCell>{name}</DataTableCell>
                  <DataTableCell>
                    {url && (
                      <>
                        <a
                          href={url}
                          target="_blank"
                          rel="noreferrer"
                          style={{
                            textDecoration: "none",
                            display: "inline-flex",
                            gap: "10px",
                            alignItems: "center",
                          }}
                        >
                          Download
                          <IconButton size="small">
                            <FileDownloadOutlinedIcon
                              color="primary"
                              style={{
                                width: "18px",
                                height: "18px",
                              }}
                            />
                          </IconButton>
                        </a>
                      </>
                    )}
                  </DataTableCell>
                  <DataTableCell style={{ textAlign: "center" }}>{height}</DataTableCell>
                  <DataTableCell style={{ textAlign: "center" }}>Active</DataTableCell>
                  <DataTableCell style={{ textAlign: "center" }}>
                    <Link to={`/templates/${toHex(address)}`} state={[address]}>
                      <IconButton>
                        <KeyboardArrowRightIcon color="primary" />
                      </IconButton>
                    </Link>
                  </DataTableCell>
                </TableRow>
              ))}
            {templates.filter(({ show }) => show).length === 0 && (
              <TableRow>
                <TableCell colSpan={4} style={{ textAlign: "center" }}>
                  <Fade in={templates.filter(({ show }) => show).length === 0} timeout={500}>
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
          count={templates.filter((template) => template.show).length}
          rowsPerPage={rowsPerPage}
          page={page}
          onPageChange={handleChangePage}
          onRowsPerPageChange={handleChangeRowsPerPage}
        />
      </TableContainer>
    </>
  );
}

export default Templates;
