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

import KeyboardArrowLeftIcon from "@mui/icons-material/KeyboardArrowLeft";
import KeyboardArrowRightIcon from "@mui/icons-material/KeyboardArrowRight";
import {
  Box,
  Button,
  IconButton,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import Grid from "@mui/material/Grid";
import { Event, TransactionId } from "@tari-project/ootle-ts-bindings";
import saveAs from "file-saver";
import React, { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import CopyToClipboard from "../../Components/CopyToClipboard";
import JsonDialog from "../../Components/JsonDialog";
import PageHeading from "../../Components/PageHeading";
import { StyledPaper } from "../../Components/StyledComponents";
import { queryTransactionEvents } from "../../utils/api";
import { encodedByteLength, useDecoder } from "../../utils/encoded";
import { truncateText } from "../../utils/helpers";

const PAGE_SIZE = 10;

type Cursor = number | bigint | string;

function EventsLayout() {
  const [encodedEvents, setEncodedEvents] = useState<Array<[string, string]>>([]);
  const { decodeEvent } = useDecoder();
  const events = useMemo(
    () => encodedEvents.map(([txId, encoded]) => ({ txId, encoded, event: decodeEvent(encoded) })),
    [encodedEvents, decodeEvent],
  );
  const [page, setPage] = useState(0);
  // cursors[n] is the before_id that loads page n; page 0 starts from the newest event.
  const [cursors, setCursors] = useState<Array<Cursor | null>>([null]);
  const [nextCursor, setNextCursor] = useState<Cursor | null>(null);
  const [jsonDialogOpen, setJsonDialogOpen] = React.useState(false);
  const [selectedPayload, setSelectedPayload] = useState({});
  const [filter, setFilter] = useState({
    topic: "",
    substate_id: "",
  });

  useEffect(() => {
    loadPage(null, filter);
  }, []);

  async function loadPage(beforeId: Cursor | null, filter: any) {
    const resp = await queryTransactionEvents({
      topic: filter.topic,
      substate_id: filter.substate_id || null,
      limit: PAGE_SIZE,
      offset: null,
      before_id: beforeId,
    });
    setEncodedEvents(resp.events);
    setNextCursor(resp.next_before_id ?? null);
  }

  async function handleChangePage(newPage: number) {
    if (newPage > page) {
      if (nextCursor === null) {
        return;
      }
      setCursors([...cursors.slice(0, newPage), nextCursor]);
      await loadPage(nextCursor, filter);
    } else if (newPage < page) {
      await loadPage(cursors[newPage], filter);
    } else {
      return;
    }
    setPage(newPage);
  }

  const handlePayloadDownload = (txId: TransactionId, event: Event) => {
    const data = event.payload;
    const json = JSON.stringify(data, null, 2);
    const blob = new Blob([json], { type: "application/json" });
    const filename = `event-${txId}-${event.topic}.json`;
    saveAs(blob, filename);
  };

  const handlePayloadView = (event: Event) => {
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

    setCursors([null]);
    await loadPage(null, newFilter);
    setPage(0);
  };

  return (
    <>
      <Grid size={12}>
        <PageHeading>Events</PageHeading>
      </Grid>
      <Grid>
        <Box className="flex-container" sx={{ marginBottom: 4 }}>
          <TextField
            name="topic"
            label="Topic"
            value={filter.topic}
            onChange={async (e: React.ChangeEvent<HTMLInputElement>) => onFilterChange(e)}
            style={{ flexGrow: 1 }}
          />
          <TextField
            name="substate_id"
            label="Substate Id"
            value={filter.substate_id}
            onChange={async (e: React.ChangeEvent<HTMLInputElement>) => onFilterChange(e)}
            style={{ flexGrow: 1 }}
          />
        </Box>
      </Grid>
      <Grid size={12}>
        <StyledPaper>
          <Table sx={{ minWidth: 650 }} aria-label="simple table">
            <TableHead>
              <TableRow>
                <TableCell>Topic</TableCell>
                <TableCell>Transaction</TableCell>
                <TableCell>Substate Id</TableCell>
                <TableCell>Template</TableCell>
                <TableCell>Payload</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {events.map(({ txId, encoded, event }, i) =>
                event ? (
                  <TableRow key={i} sx={{ "&:last-child td, &:last-child th": { border: 0 } }}>
                    <TableCell>{event.topic}</TableCell>
                    <TableCell>
                      {truncateText(txId, 20)}
                      <CopyToClipboard copy={txId} />
                    </TableCell>
                    <TableCell>
                      {event.substate_id ? (
                        <Link to={`/substates?address=${encodeURIComponent(event.substate_id)}`}>
                          {truncateText(event.substate_id, 20)}
                        </Link>
                      ) : (
                        truncateText(event.substate_id, 20)
                      )}
                      <CopyToClipboard copy={event.substate_id || ""} />
                    </TableCell>
                    <TableCell>
                      {event.template_address ? (
                        <Link to={`/templates?address=${encodeURIComponent(event.template_address)}`}>
                          {truncateText(event.template_address, 20)}
                        </Link>
                      ) : (
                        truncateText(event.template_address, 20)
                      )}
                      <CopyToClipboard copy={event.template_address} />
                    </TableCell>
                    <TableCell>
                      <Stack direction="row" spacing={2} alignItems="left">
                        <Button variant="outlined" onClick={() => handlePayloadView(event)}>
                          View
                        </Button>
                        <Button variant="outlined" onClick={() => handlePayloadDownload(txId, event)}>
                          Download
                        </Button>
                      </Stack>
                    </TableCell>
                  </TableRow>
                ) : (
                  <TableRow key={i}>
                    <TableCell colSpan={5}>
                      {truncateText(txId, 20)}: CBOR, {encodedByteLength(encoded)} bytes
                    </TableCell>
                  </TableRow>
                ),
              )}
            </TableBody>
          </Table>
          <Stack direction="row" justifyContent="right" spacing={2} alignItems="center">
            <IconButton aria-label="copy" onClick={() => handleChangePage(Math.max(page - 1, 0))}>
              <KeyboardArrowLeftIcon />
            </IconButton>
            <Typography sx={{}}>{page}</Typography>
            <IconButton aria-label="copy" disabled={nextCursor === null} onClick={() => handleChangePage(page + 1)}>
              <KeyboardArrowRightIcon />
            </IconButton>
          </Stack>
        </StyledPaper>
      </Grid>
      <JsonDialog open={jsonDialogOpen} onClose={handleJsonDialogClose} data={selectedPayload} />
    </>
  );
}

export default EventsLayout;
