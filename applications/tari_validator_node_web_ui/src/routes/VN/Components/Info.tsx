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

import { useState } from "react";
import { registerValidatorNode } from "../../../utils/json_rpc";
import "./Info.css";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableRow from "@mui/material/TableRow";
import Button from "@mui/material/Button";
import { DataTableCell } from "../../../Components/StyledComponents";
import { TextField } from "@mui/material";
import type {
  GetEpochManagerStatsResponse,
  GetIdentityResponse,
} from "@tariproject/typescript-bindings/validator-node-client";

function Info({
  epoch,
  identity,
  shardKey,
}: {
  epoch: GetEpochManagerStatsResponse;
  identity: GetIdentityResponse;
  shardKey: string | null;
}) {
  const [registering, setRegistering] = useState(false);
  const [registerMessage, setRegisterMessage] = useState("");
  const [feeClaimPublicKey, setRegisterFeeClaimPublicKey] = useState("");
  const register = () => {
    setRegistering(true);
    registerValidatorNode({ fee_claim_public_key: feeClaimPublicKey }).then((response) => {
      setRegisterMessage(`Registration successful, the TxId ${response.transaction_id}`);
    });
  };
  const renderShardKey = () => {
    if (shardKey === null)
      return (
        <>
          {/* <TableRow>
            <TableCell>Shard key</TableCell>
            <DataTableCell>
              <span
                className={`${registering ? 'disabled-button' : 'button'}`}
                id="register"
                onClick={registering ? () => {} : register}
              >
                Register
              </span>
              {registerMessage ? <span>{registerMessage}</span> : null}
            </DataTableCell>
          </TableRow> */}
          <TableRow>
            <TableCell>Shard key</TableCell>
            <DataTableCell>
              <TextField
                disabled={registering}
                name="feeClaimFublicKey"
                label="Fee Claim Public Key"
                style={{ flexGrow: 1 }}
                value={feeClaimPublicKey}
                onChange={(e) => setRegisterFeeClaimPublicKey(e.target.value)}
              />
              <Button disabled={registering} variant="contained" onClick={registering ? () => {} : register}>
                Register
              </Button>
              {registerMessage ? <span style={{ marginLeft: "20px" }}>{registerMessage}</span> : null}
            </DataTableCell>
          </TableRow>
        </>
      );
    return (
      <TableRow>
        <TableCell>Shard key</TableCell>
        <DataTableCell className="key">{shardKey}</DataTableCell>
      </TableRow>
    );
  };
  return (
    <div>
      <TableContainer>
        <Table>
          <TableBody>
            <TableRow>
              <TableCell>Epoch</TableCell>
              <DataTableCell>
                {epoch.current_epoch} ({epoch.is_valid ? "Valid" : "Not valid"})
              </DataTableCell>
            </TableRow>
            <TableRow>
              <TableCell>Peer id</TableCell>
              <DataTableCell>{identity.peer_id}</DataTableCell>
            </TableRow>
            <TableRow>
              <TableCell>Listen addresses</TableCell>
              <DataTableCell>{identity.public_addresses?.join("\n")}</DataTableCell>
            </TableRow>
            <TableRow>
              <TableCell>Public key</TableCell>
              <DataTableCell>{identity.public_key}</DataTableCell>
            </TableRow>
            <TableRow>
              <TableCell>Committee info</TableCell>
              <DataTableCell>
                {epoch.committee_shard ? (
                  <>
                    Bucket: {epoch.committee_shard.shard}
                    <br />
                    Num committees: {epoch.committee_shard.num_committees}
                    <br />
                    Num members: {epoch.committee_shard.num_members}
                  </>
                ) : (
                  "Validator not registered"
                )}
              </DataTableCell>
            </TableRow>
            {renderShardKey()}
          </TableBody>
        </Table>
      </TableContainer>
    </div>
  );
}

export default Info;
