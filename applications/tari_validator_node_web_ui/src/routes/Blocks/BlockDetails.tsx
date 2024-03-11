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

import { useState, useEffect } from "react";
import { useParams } from "react-router-dom";
// import { transactionsGet } from '../../utils/json_rpc';
import { Accordion, AccordionDetails, AccordionSummary } from "../../Components/Accordion";
import { Grid, Table, TableContainer, TableBody, TableRow, TableCell, Button, Fade, Alert } from "@mui/material";
import Typography from "@mui/material/Typography";
import { DataTableCell, StyledPaper } from "../../Components/StyledComponents";
import PageHeading from "../../Components/PageHeading";
import StatusChip from "../../Components/StatusChip";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import Loading from "../../Components/Loading";
import { getBlock, getIdentity } from "../../utils/json_rpc";
import Transactions from "./Transactions";
import { primitiveDateTimeToDate, primitiveDateTimeToSecs } from "../../utils/helpers";
import type { Block, TransactionAtom } from "@tariproject/typescript-bindings";
import type { GetIdentityResponse } from "@tariproject/typescript-bindings/validator-node-client";

export default function BlockDetails() {
  const { blockId } = useParams();
  const [expandedPanels, setExpandedPanels] = useState<string[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<String>();
  const [block, setBlock] = useState<Block>();
  const [prepare, setPrepare] = useState<TransactionAtom[]>([]);
  const [localPrepared, setLocalPrepared] = useState<TransactionAtom[]>([]);
  const [accept, setAccept] = useState<TransactionAtom[]>([]);
  const [identity, setIdentity] = useState<GetIdentityResponse>();
  const [blockTime, setBlockTime] = useState<number>(0);

  useEffect(() => {
    if (blockId !== undefined) {
      Promise.all([getBlock({ block_id: blockId }), getIdentity()])
        .then(([resp, identity]) => {
          setIdentity(identity);
          setBlock(resp.block);
          if (resp?.block?.justify?.block_id) {
            getBlock({ block_id: resp.block.justify.block_id }).then((justify) => {
              if (resp.block.stored_at && justify.block.stored_at) {
                let blockTime = primitiveDateTimeToSecs(resp.block.stored_at);
                let justifyTime = primitiveDateTimeToSecs(justify.block.stored_at);
                setBlockTime(blockTime - justifyTime);
              }
            });
          }
          setPrepare([]);
          setLocalPrepared([]);
          setAccept([]);
          for (let command of resp.block.commands) {
            if ("Prepare" in command) {
              let newPrepare = command.Prepare;
              setPrepare((prepare: TransactionAtom[]) => [...prepare, newPrepare]);
            } else if ("LocalPrepared" in command) {
              let newLocalPrepared = command.LocalPrepared;
              setLocalPrepared((localPrepared: TransactionAtom[]) => [...localPrepared, newLocalPrepared]);
            } else if ("Accept" in command) {
              let newAccept = command.Accept;
              setAccept((accept: TransactionAtom[]) => [...accept, newAccept]);
            }
          }
        })
        .catch((err) => {
          setError(err && err.message ? err.message : `Unknown error: ${JSON.stringify(err)}`);
        })
        .finally(() => {
          setLoading(false);
        });
    }
  }, [blockId]);

  const handleChange = (panel: string) => (event: React.SyntheticEvent, isExpanded: boolean) => {
    setExpandedPanels((prevExpandedPanels) => {
      if (isExpanded) {
        return [...prevExpandedPanels, panel];
      } else {
        return prevExpandedPanels.filter((p) => p !== panel);
      }
    });
  };

  const expandAll = () => {
    setExpandedPanels(["panel1", "panel2", "panel3", "panel4", "panel5"]);
  };

  const collapseAll = () => {
    setExpandedPanels([]);
  };
  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Block Details</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          {loading ? (
            <Loading />
          ) : (
            <Fade in={!loading}>
              <div>
                {error ? (
                  <Alert severity="error">{error}</Alert>
                ) : (
                  <>
                    <TableContainer>
                      <Table>
                        <TableBody>
                          <TableRow>
                            <TableCell>Block ID</TableCell>
                            <DataTableCell>{blockId}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Epoch</TableCell>
                            <DataTableCell>{block!.epoch}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Height</TableCell>
                            <DataTableCell>{block!.height}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Parent block</TableCell>
                            <DataTableCell>
                              <a href={`/blocks/${block!.parent}`}>{block!.parent}</a>
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Total Fees</TableCell>
                            <DataTableCell>
                              <div className={block!.proposed_by === identity!.public_key ? "my_money" : ""}>
                                {block!.total_leader_fee}
                              </div>
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Status</TableCell>
                            <DataTableCell>
                              <StatusChip status={block!.justify.decision === "Accept" ? "Commit" : "Abort"} />
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Proposed by</TableCell>
                            <DataTableCell>{block!.proposed_by}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Block time</TableCell>
                            <DataTableCell>{blockTime} secs</DataTableCell>
                          </TableRow>
                          {block!.stored_at && (
                            <TableRow>
                              <TableCell>Stored at</TableCell>
                              <DataTableCell>
                                {primitiveDateTimeToDate(block!.stored_at).toLocaleString()}
                              </DataTableCell>
                            </TableRow>
                          )}
                        </TableBody>
                      </Table>
                    </TableContainer>
                    <div
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                        padding: "2rem 1rem 0.5rem 1rem",
                      }}
                      // className="flex-container"
                    >
                      <Typography variant="h5">More Info</Typography>
                      <div
                        style={{
                          display: "flex",
                          justifyContent: "flex-end",
                          gap: "1rem",
                        }}
                      >
                        <Button
                          onClick={expandAll}
                          style={{
                            fontSize: "0.85rem",
                          }}
                          startIcon={<KeyboardArrowDownIcon />}
                        >
                          Expand All
                        </Button>
                        <Button
                          onClick={collapseAll}
                          style={{
                            fontSize: "0.85rem",
                          }}
                          startIcon={<KeyboardArrowUpIcon />}
                          disabled={expandedPanels.length === 0 ? true : false}
                        >
                          Collapse All
                        </Button>
                      </div>
                    </div>
                  </>
                )}
                {prepare.length > 0 && (
                  <Accordion expanded={expandedPanels.includes("panel1")} onChange={handleChange("panel1")}>
                    <AccordionSummary aria-controls="panel1bh-content" id="panel1bh-header">
                      <Typography>Prepare</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Transactions transactions={prepare} />
                    </AccordionDetails>
                  </Accordion>
                )}
                {localPrepared.length > 0 && (
                  <Accordion expanded={expandedPanels.includes("panel2")} onChange={handleChange("panel2")}>
                    <AccordionSummary aria-controls="panel2bh-content" id="panel2bh-header">
                      <Typography>Local prepared</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Transactions transactions={localPrepared} />
                    </AccordionDetails>
                  </Accordion>
                )}
                {accept.length > 0 && (
                  <Accordion expanded={expandedPanels.includes("panel3")} onChange={handleChange("panel3")}>
                    <AccordionSummary aria-controls="panel3bh-content" id="panel3bh-header">
                      <Typography>Accept</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Transactions transactions={accept} />
                    </AccordionDetails>
                  </Accordion>
                )}
              </div>
            </Fade>
          )}
        </StyledPaper>
      </Grid>
    </>
  );
}
