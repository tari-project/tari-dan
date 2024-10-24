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
import { decodeShardGroup, primitiveDateTimeToDate, primitiveDateTimeToSecs } from "../../utils/helpers";
import type {
  Block,
  Command,
  ForeignProposalAtom,
  TransactionAtom,
  VNGetIdentityResponse,
  MintConfidentialOutputAtom,
} from "@tari-project/typescript-bindings";

const COMMANDS = [
  "LocalOnly",
  "Prepare",
  "LocalPrepare",
  "AllPrepare",
  "SomePrepare",
  "LocalAccept",
  "AllAccept",
  "SomeAccept",
];

type OtherCommands = Record<string, Array<any>>;
// interface OtherCommands {
//   [key: string]: Array<any>;
// }

export default function BlockDetails() {
  const { blockId } = useParams();
  const [expandedPanels, setExpandedPanels] = useState<string[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<String>();
  const [block, setBlock] = useState<Block>();

  const [blockData, setBlockData] = useState<{ [key: string]: TransactionAtom[] }>({});
  const [otherCommands, setOtherCommands] = useState<OtherCommands>({});

  const [epochEvents, setEpochEvents] = useState<string[]>([]);
  const [identity, setIdentity] = useState<VNGetIdentityResponse>();
  const [blockTime, setBlockTime] = useState<number>(0);
  const [foreignProposals, setForeignProposals] = useState<ForeignProposalAtom[]>([]);
  const [mintedUtxos, setMintedUtxos] = useState<MintConfidentialOutputAtom[]>([]);

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
          setEpochEvents([]);
          const otherCommands: OtherCommands = {};
          const foreignProposals = [];
          const mintedUtxos = [];
          const data: { [key: string]: TransactionAtom[] } = {};
          for (let command of resp.block.commands) {
            if (typeof command === "object") {

              const cmd = Object.keys(command)[0];

              if (COMMANDS.indexOf(cmd) > -1) {
                data[cmd] ||= [];
                data[cmd].push(command[cmd as keyof Command]);
              } else if ("ForeignProposal" in command) {
                foreignProposals.push(command.ForeignProposal);
              } else if ("MintConfidentialOutput" in command) {
                mintedUtxos.push(command.MintConfidentialOutput);
              } else {
                if (Array.isArray(otherCommands[cmd])) {
                  otherCommands[cmd].push(command[cmd as keyof Command]);
                } else {
                  // command[cmd as keyof Command]});
                  Object.assign(otherCommands, { [cmd]: [command[cmd as keyof Command]] });
                }
              }
            } else {
              setEpochEvents((epochEvents: string[]) => [...epochEvents, command as string]);
            }
          }

          setForeignProposals(foreignProposals);
          setMintedUtxos(mintedUtxos);
          setBlockData(data);
          setOtherCommands(otherCommands);

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
    setExpandedPanels((prevExpandedPanels: string[]) => {
      if (isExpanded) {
        return [...prevExpandedPanels, panel];
      } else {
        return prevExpandedPanels.filter((p) => p !== panel);
      }
    });
  };

  const expandAll = () => {
    for (let cmd in COMMANDS) {
      setExpandedPanels((prevExpandedPanels: string[]) => {
        if (!prevExpandedPanels.includes(`panel${cmd}`)) {
          return [...prevExpandedPanels, `panel${cmd}`];
        } else {
          return prevExpandedPanels;
        }
      });
    }
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
                              <StatusChip status={block!.is_committed ? "Commit" : "Pending"} />
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
                          disabled={expandedPanels.length === 0}
                        >
                          Collapse All
                        </Button>
                      </div>
                    </div>
                  </>
                )}
                {COMMANDS.map((cmd, i) => {
                  if (!blockData[cmd]) {
                    return <> </>;
                  }
                  return (
                    <Accordion
                      key={i}
                      expanded={expandedPanels.includes(`panel${cmd}`)}
                      onChange={handleChange(`panel${cmd}`)}
                    >
                      <AccordionSummary aria-controls={`panel${cmd}bh-content`} id={`panel${cmd}bh-header`}>
                        <Typography>{cmd}</Typography>
                      </AccordionSummary>
                      <AccordionDetails>
                        <Transactions transactions={blockData[cmd]} />
                      </AccordionDetails>
                    </Accordion>
                  );
                })}
                {foreignProposals.length > 0 && (
                  <Accordion expanded={expandedPanels.includes("panelForeignProposals")}
                             onChange={handleChange("panelForeignProposals")}>
                    <AccordionSummary aria-controls="panelForeignProposalsbh-content"
                                      id="panelForeignProposalsbh-header">
                      <Typography>Foreign Proposals</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      {foreignProposals.map((proposal, i) => (
                        <div key={i}>
                          Foreign Proposal: {proposal.block_id} {JSON.stringify(decodeShardGroup(proposal.shard_group))}
                        </div>
                      ))}
                    </AccordionDetails>
                  </Accordion>
                )}
                {mintedUtxos.length > 0 && (
                  <Accordion expanded={expandedPanels.includes("panelMintedUtxos")}
                             onChange={handleChange("panelMintedUtxos")}>
                    <AccordionSummary aria-controls="panelMintedUtxosbh-content" id="panelMintedUtxosbh-header">
                      <Typography>Mint Confidential Output</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      {mintedUtxos.map((utxo, i) => (
                        <div key={i}>
                          Unclaimed UTXO: {JSON.stringify(utxo.substate_id)}
                        </div>
                      ))}
                    </AccordionDetails>
                  </Accordion>
                )}
                {epochEvents.length > 0 && (
                  <Accordion expanded={expandedPanels.includes("panelEpochEvents")}
                             onChange={handleChange("panelEpochEvents")}>
                    <AccordionSummary aria-controls="panelEpochEventsbh-content" id="panelEpochEventsbh-header">
                      <Typography>EpochEvent</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <ul>{epochEvents.map((evt, i) => <li key={i}>{evt}</li>)}</ul>
                    </AccordionDetails>
                  </Accordion>
                )}
                {Object.keys(otherCommands).length > 0 && Object.keys(otherCommands).map((key, i) => (
                  <Accordion key={i} expanded={expandedPanels.includes(`panel${key}`)}
                             onChange={handleChange(`panel${key}`)}>
                    <AccordionSummary aria-controls={`panel${key}bh-content`} id={`panel${key}sbh-header`}>
                      <Typography>{key}</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <ul>{otherCommands[key].map((elem, j) => <li key={j}>{JSON.stringify(elem)}</li>)}</ul>
                    </AccordionDetails>
                  </Accordion>
                ))}
              </div>
            </Fade>
          )}
        </StyledPaper>
      </Grid>
    </>
  );
}
