//  Copyright 2024. The Tari Project
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
import { StyledPaper } from "../../Components/StyledComponents";
import { Box, Button, IconButton, Stack, Table, TableBody, TableCell, TableHead, TableRow, TextField, Typography, Select, MenuItem, InputLabel } from "@mui/material";
import React, { useEffect, useState } from "react";
import { toHexString } from "../VN/Components/helpers";
import { truncateText } from "../../utils/helpers";
import ContentCopyIcon from '@mui/icons-material/ContentCopy';
import KeyboardArrowLeftIcon from '@mui/icons-material/KeyboardArrowLeft';
import KeyboardArrowRightIcon from '@mui/icons-material/KeyboardArrowRight';
import saveAs from "file-saver";
import JsonDialog from "../../Components/JsonDialog";
import type {
  ListSubstatesRequest,
  ListSubstatesResponse,
  SubstateType
} from "@tari-project/typescript-bindings";
import { listSubstates, getSubstate } from "../../utils/json_rpc";

const PAGE_SIZE = 10;
const SUBSTATE_TYPES = ["Component", "Resource", "Vault", "UnclaimedConfidentialOutput", "NonFungible", "TransactionReceipt", "FeeClaim"] as const;

function SubstatesLayout() {
  const [substates, setSubstates] = useState<any []>([]);
  const [page, setPage] = useState(0);
  const [jsonDialogOpen, setJsonDialogOpen] = React.useState(false);
  const [selectedContent, setSelectedContent] = useState({});
  const [filter, setFilter] = useState({
    filter_by_template: null,
    filter_by_type: null
  });

  useEffect(() => {
    get_substates(page, PAGE_SIZE, filter);
  }, []);

  async function get_substates(offset: number, limit: number, filter: any) {
    let params = {
      limit,
      offset,
      filter_by_template: null,
      filter_by_type: null,
    };
    if (filter.filter_by_template) {
      params.filter_by_template = filter.filter_by_template;
    }
    if (filter.filter_by_type) {
      params.filter_by_type = filter.filter_by_type;
    }

    // Ignoring eslint about BintInt to number conversion, as BigInts break serialization
    // @ts-ignore
    let resp = await listSubstates(params);

    console.log({resp});

    let substates = resp.substates.map((s) => {
      return {
        ...s,
        address: Object.values(s.substate_id)[0],
        timestamp: (new Date(Number(s.timestamp) * 1000)).toDateString(),
      };
    });

    console.log({substates});
    setSubstates(substates);
  }

  async function handleCopyClick(text: string) {
    if (text) {
      navigator.clipboard.writeText(text);
    }
  };

  async function handleChangePage(newPage: number) { 
    const offset = newPage * PAGE_SIZE;
    await get_substates(offset, PAGE_SIZE, filter);
    setPage(newPage);
  };

  const handleContentDownload = async (substate: any) => {
    const data = await getSubstate({
      address: substate.address,
      version: null,
      local_search_only: false
    });

    const json = JSON.stringify(data, null, 2);
    const blob = new Blob([json], { type: "application/json" });
    const filename = `substates-${substate.address}-${substate.version}.json`;
    saveAs(blob, filename);
  };

  const handleContentView = async (substate: any) => {
    const data = await getSubstate({
      address: substate.address,
      version: null,
      local_search_only: false
    });
    setSelectedContent(data);
    setJsonDialogOpen(true);
  };  

  const handleJsonDialogClose = () => {
    setJsonDialogOpen(false);
  };

  const onFilterChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const newFilter = {
      ...filter,
      [e.target.name]: e.target.value,
    };

    setFilter(newFilter);

    const offset = 0;
    await get_substates(offset, PAGE_SIZE, newFilter);
    setPage(0);
  };

  return (
    <>
      <Grid item sm={12} md={12} xs={12}>
        <PageHeading>Substates</PageHeading>
      </Grid>
      <Grid item>
        <Box className="flex-container" sx={{ marginBottom: 4 }}>
          <TextField
            name="filter_by_template"
            label="Template"
            value={filter.filter_by_template}
            onChange={async (e: any) => onFilterChange(e)}
            style={{ flexGrow: 1 }} />
          <Select
            name="filter_by_type"
            label="Type"
            value={filter.filter_by_type}
            displayEmpty
            onChange={async (e: any) => onFilterChange(e)}
            size="medium"
            renderValue={(value) => {
              if (!value) {
                return <>All Types</>;
              }
  
              return value;
            }}     
            style={{ flexGrow: 1, minWidth: "200px" }}>
              <MenuItem key={"All Types"} value={undefined}>
                {"All types"}
              </MenuItem>
              {SUBSTATE_TYPES.map((type) => (
                <MenuItem key={type} value={type}>
                  {type}
                </MenuItem>
              ))}
          </Select>
        </Box>
      </Grid>
      <Grid item sm={12} md={12} xs={12}>
        <StyledPaper>
          <Table sx={{ minWidth: 650 }} aria-label="simple table">
            <TableHead>
              <TableRow>
                <TableCell><Typography variant="h3">Address</Typography></TableCell>
                <TableCell><Typography variant="h3">Version</Typography></TableCell>
                <TableCell><Typography variant="h3">Template</Typography></TableCell>
                <TableCell><Typography variant="h3">Timestamp</Typography></TableCell>
                <TableCell><Typography variant="h3">Content</Typography></TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {substates.map((row: any) => (
                <TableRow
                  sx={{ '&:last-child td, &:last-child th': { border: 0 } }}
                >
                  <TableCell>
                    {truncateText(row.address, 20)}
                    <IconButton aria-label="copy" onClick={() => handleCopyClick(row.address)}>
                      <ContentCopyIcon />
                    </IconButton>
                  </TableCell>

                  <TableCell>{row.version}</TableCell>

                  <TableCell>
                    { row.template_address != null &&
                      <>
                      {truncateText(row.template_address, 20)}
                      <IconButton aria-label="copy" onClick={() => handleCopyClick(row.template_address)}>
                        <ContentCopyIcon />
                      </IconButton>
                      </>
                    }
                  </TableCell>

                  <TableCell>{row.timestamp}</TableCell>

                  <TableCell>
                    <Stack direction="row" spacing={2} alignItems="left">
                    <Button variant="outlined" onClick={async () => handleContentView(row)}>
                        View
                    </Button>
                    <Button variant="outlined" onClick={async () => handleContentDownload(row)}>
                        Download
                    </Button>
                    </Stack>
                    
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <Stack direction="row" justifyContent="right" spacing={2} alignItems="center">
            <IconButton aria-label="copy" onClick={() => handleChangePage(Math.max(page - 1, 0))}>
              <KeyboardArrowLeftIcon />
            </IconButton>
            <Typography sx={{}}>{page}</Typography>
            <IconButton aria-label="copy" onClick={() => handleChangePage(page + 1)}>
              <KeyboardArrowRightIcon />
            </IconButton>
          </Stack>
        </StyledPaper>
      </Grid>
      <JsonDialog
        open={jsonDialogOpen}
        onClose={handleJsonDialogClose}
        data={selectedContent}/>
    </>
  );
}

export default SubstatesLayout;
