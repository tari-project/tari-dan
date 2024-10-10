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
import { Box, Button, IconButton, Stack, Table, TableBody, TableCell, TableHead, TableRow, TextField, Typography } from "@mui/material";
import React, { useEffect, useState } from "react";
import { toHexString } from "../VN/Components/helpers";
import { truncateText } from "../../utils/helpers";
import ContentCopyIcon from '@mui/icons-material/ContentCopy';
import KeyboardArrowLeftIcon from '@mui/icons-material/KeyboardArrowLeft';
import KeyboardArrowRightIcon from '@mui/icons-material/KeyboardArrowRight';
import saveAs from "file-saver";
import JsonDialog from "../../Components/JsonDialog";

const INDEXER_ADDRESS = "http://localhost:18301";
const PAGE_SIZE = 10;

function EventsLayout() {
  const [events, setEvents] = useState([]);
  const [page, setPage] = useState(0);
  const [jsonDialogOpen, setJsonDialogOpen] = React.useState(false);
  const [selectedPayload, setSelectedPayload] = useState({});
  const [filter, setFilter] = useState({
    topic: null,
    substate_id: null,
  });

  useEffect(() => {
    get_events(page, PAGE_SIZE, filter);
  }, []);

  async function get_events(offset: number, limit: number, filter: object) {
    let graphql_filters = "";
    if (filter.topic) {
      graphql_filters += `topic:"${filter.topic}", `;
    }
    if (filter.substate_id) {
      graphql_filters += `substateId:"${filter.substate_id}", `;
    }

    let res = await fetch(INDEXER_ADDRESS, {
      method: 'POST',

      headers: {
        "Content-Type": "application/json",
        "Accept": "application/json"
      },

      body: JSON.stringify({
        query: `{ getEvents(${graphql_filters} offset:${offset}, limit:${limit}) {substateId, templateAddress, txHash, topic, payload } }`,
        variables: {}
      })
    });

    let res_json = await res.json();
    console.log({ res_json });
    let events = res_json.data.getEvents;

    let rows = events.map((event) => {
      return {
        ...event,
        tx_hash: toHexString(event.txHash),
        template_address: toHexString(event.templateAddress),
      };
    });
    console.log({rows});
    setEvents(rows);
  }

  async function handleCopyClick(text: string) {
    if (text) {
        navigator.clipboard.writeText(text);
    }
  };

  async function handleChangePage(newPage: number) { 
    const offset = newPage * PAGE_SIZE;
    await get_events(offset, PAGE_SIZE, filter);
    setPage(newPage);
  };

  const handlePayloadDownload = (event) => {
    const data = event.payload;
    const json = JSON.stringify(data, null, 2);
    const blob = new Blob([json], { type: "application/json" });
    const filename = `event-${event.tx_hash}-${event.topic}.json`;
    saveAs(blob, filename);
  };

  const handlePayloadView = (event) => {
    setSelectedPayload(event.payload);
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
    await get_events(offset, PAGE_SIZE, newFilter);
    setPage(0);
  };

  return (
    <>
      <Grid item sm={12} md={12} xs={12}>
        <PageHeading>Events</PageHeading>
      </Grid>
      <Grid item>
        <Box className="flex-container" sx={{ marginBottom: 4 }}>
          <TextField
            name="topic"
            label="Topic"
            value={filter.topic}
            onChange={async (e) => onFilterChange(e)}
            style={{ flexGrow: 1 }} />
          <TextField
            name="substate_id"
            label="Substate Id"
            value={filter.substate_id}
            onChange={async (e) => onFilterChange(e)}
            style={{ flexGrow: 1 }}
          />
      </Box>
      </Grid>
      <Grid item sm={12} md={12} xs={12}>
        <StyledPaper>
          <Table sx={{ minWidth: 650 }} aria-label="simple table">
            <TableHead>
              <TableRow>
                <TableCell><Typography variant="h3">Topic</Typography></TableCell>
                <TableCell><Typography variant="h3">Transaction</Typography></TableCell>
                <TableCell><Typography variant="h3">Substate Id</Typography></TableCell>
                <TableCell><Typography variant="h3">Template</Typography></TableCell>
                <TableCell><Typography variant="h3">Payload</Typography></TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {events.map((row) => (
                <TableRow
                  sx={{ '&:last-child td, &:last-child th': { border: 0 } }}
                >
                  <TableCell>{row.topic}</TableCell>
                  <TableCell>
                    {truncateText(row.tx_hash, 20)}
                    <IconButton aria-label="copy" onClick={() => handleCopyClick(row.tx_hash)}>
                      <ContentCopyIcon />
                    </IconButton>
                  </TableCell>
                  <TableCell>
                    {truncateText(row.substateId, 20)}
                    <IconButton aria-label="copy" onClick={() => handleCopyClick(row.substateId)}>
                      <ContentCopyIcon />
                    </IconButton>
                  </TableCell>
                  <TableCell>
                    {truncateText(row.template_address, 20)}
                    <IconButton aria-label="copy" onClick={() => handleCopyClick(row.template_address)}>
                      <ContentCopyIcon />
                    </IconButton>
                  </TableCell>
                  <TableCell>
                    <Stack direction="row" spacing={2} alignItems="left">
                    <Button variant="outlined" onClick={() => handlePayloadView(row)}>
                        View
                    </Button>
                    <Button variant="outlined" onClick={() => handlePayloadDownload(row)}>
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
        data={selectedPayload}/>
    </>
  );
}

export default EventsLayout;
