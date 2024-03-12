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

import React, { useCallback, useEffect, useRef, useState } from "react";
import { addPeer, getConnections } from "../../../utils/json_rpc";
import { shortenString } from "./helpers";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, BoxHeading2 } from "../../../Components/StyledComponents";
import AddIcon from "@mui/icons-material/Add";
import Button from "@mui/material/Button";
import { TextField } from "@mui/material";
import { Form } from "react-router-dom";
import Fade from "@mui/material/Fade";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import type { Connection } from "@tariproject/typescript-bindings/validator-node-client";

const useInterval = (fn: () => Promise<unknown>, ms: number) => {
  const timeout = useRef<number>();
  const mountedRef = useRef(false);
  const run = useCallback(async () => {
    await fn();
    if (mountedRef.current) {
      timeout.current = window.setTimeout(run, ms);
    }
  }, [fn, ms]);
  useEffect(() => {
    mountedRef.current = true;
    run();
    return () => {
      mountedRef.current = false;
      window.clearTimeout(timeout.current);
    };
  }, [run]);
};

function Connections() {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [showPeerDialog, setShowAddPeerDialog] = useState(false);
  const [formState, setFormState] = useState({ publicKey: "", address: "" });

  const showAddPeerDialog = (setElseToggle: boolean = !showPeerDialog) => {
    setShowAddPeerDialog(setElseToggle);
  };

  const onSubmitAddPeer = async () => {
    await addPeer({
      public_key: formState.publicKey,
      addresses: formState.address ? [formState.address] : [],
      wait_for_dial: false,
    });
    setFormState({ publicKey: "", address: "" });
    setShowAddPeerDialog(false);
  };
  const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFormState({ ...formState, [e.target.name]: e.target.value });
  };

  let fetchConnections = useCallback(async () => {
    const resp = await getConnections();
    setConnections(resp.connections);
  }, []);
  useInterval(fetchConnections, 5000);

  return (
    <>
      <BoxHeading2>
        {showPeerDialog && (
          <Fade in={showPeerDialog}>
            <Form onSubmit={onSubmitAddPeer} className="flex-container">
              <TextField
                name="publicKey"
                label="Public Key"
                value={formState.publicKey}
                onChange={onChange}
                style={{ flexGrow: 1 }}
              />
              <TextField
                name="address"
                label="Address"
                value={formState.address}
                onChange={onChange}
                style={{ flexGrow: 1 }}
              />
              <Button variant="contained" type="submit">
                Add Peer
              </Button>
              <Button variant="outlined" onClick={() => showAddPeerDialog(false)}>
                Cancel
              </Button>
            </Form>
          </Fade>
        )}
        {!showPeerDialog && (
          <Fade in={!showPeerDialog}>
            <div className="flex-container">
              <Button variant="outlined" startIcon={<AddIcon />} onClick={() => showAddPeerDialog()}>
                Add Peer
              </Button>
            </div>
          </Fade>
        )}
      </BoxHeading2>
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>Peer id</TableCell>
              <TableCell>Address</TableCell>
              <TableCell>Age</TableCell>
              <TableCell>Direction</TableCell>
              <TableCell>Latency</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {connections &&
              connections.map(({ connection_id, address, age, direction, peer_id, ping_latency }) => (
                <TableRow key={connection_id}>
                  <DataTableCell>
                    {peer_id ? shortenString(peer_id) : "--"}
                    <CopyToClipboard copy={peer_id} />
                  </DataTableCell>
                  <DataTableCell>{address}</DataTableCell>
                  <DataTableCell>{displayDuration(age)}</DataTableCell>
                  <DataTableCell>{direction}</DataTableCell>
                  <DataTableCell>{ping_latency ? displayDuration(ping_latency) : "--"}</DataTableCell>
                </TableRow>
              ))}
          </TableBody>
        </Table>
      </TableContainer>
    </>
  );
}

function displayDuration(duration: { secs: number; nanos: number }) {
  if (duration.secs === 0) {
    if (duration.nanos > 1000000) {
      return `${(duration.nanos / 1000000).toFixed(2)}ms`;
    }
    if (duration.nanos > 1000) {
      return `${(duration.nanos / 1000).toFixed(2)}µs`;
    }
    return `${duration.nanos / 1000}ns`;
  }
  if (duration.secs > 60 * 60) {
    return `${(duration.secs / 60 / 60).toFixed(0)}h${(duration.secs / 60).toFixed(0)}m`;
  }
  if (duration.secs > 60) {
    return `${(duration.secs / 60).toFixed(0)}m${(duration.secs % 60).toFixed(0)}s`;
  }
  return `${duration.secs}.${(duration.nanos / 1000000).toFixed(0)}s`;
}

export default Connections;
