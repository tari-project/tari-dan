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
import { Link, useParams } from "react-router-dom";
import { Accordion, AccordionDetails, AccordionSummary } from "../../Components/Accordion";
import {
  Grid,
  Table,
  TableContainer,
  TableBody,
  TableRow,
  TableCell,
  Button,
  Fade,
  Alert,
  Box,
  Tooltip,
} from "@mui/material";
import Typography from "@mui/material/Typography";
import { DataTableCell, StyledPaper } from "../../Components/StyledComponents";
import PageHeading from "../../Components/PageHeading";
import StatusChip from "../../Components/StatusChip";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import HelpOutlineIcon from "@mui/icons-material/HelpOutline";
import Loading from "../../Components/Loading";
import { getBlock, getIdentity } from "../../utils/json_rpc";
import Transactions from "./Transactions";
import type {
  Block,
  Command,
  ForeignProposalAtom,
  TransactionAtom,
  VNGetIdentityResponse,
} from "@tari-project/ootle-ts-bindings";

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

function BudgetCell({ used, budget, tooltip }: { used: number; budget: number; tooltip: string }) {
  if (!budget) {
    return <>{used.toLocaleString()}</>;
  }
  const percent = (used / budget) * 100;
  // Sub-0.01% blocks are the common case on a quiet network, so distinguish them from empty rather than rounding
  // them to zero.
  const share = !used ? "0" : percent < 0.01 ? "<0.01" : percent.toFixed(2);
  return (
    <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
      {used.toLocaleString()} ({share}%)
      <Tooltip arrow title={tooltip}>
        <HelpOutlineIcon sx={{ fontSize: "1rem", opacity: 0.6, cursor: "help" }} />
      </Tooltip>
    </Box>
  );
}

// The per-block budget governs WASM and native points together, so only their sum may be shown as a share of it.
function ExecutionPointsCell({ wasm, native, max }: { wasm: bigint; native: bigint; max: bigint }) {
  return (
    <BudgetCell
      used={Number(wasm) + Number(native)}
      budget={Number(max)}
      tooltip="Compute metered for this block, against the per-block budget."
    />
  );
}

// The sum follows the validation rule, so it belongs with the validation bound and no other budget.
function BlockWeightCell({ weight, max }: { weight: bigint; max: bigint }) {
  return (
    <BudgetCell
      used={Number(weight)}
      budget={Number(max)}
      tooltip="Size and IO cost of this block's transactions, against the most a replica will vote for."
    />
  );
}

function BlockBurnCell({ burn, leaderFee }: { burn: bigint | null; leaderFee: bigint }) {
  if (burn === null) {
    return <span>…</span>;
  }
  const collected = leaderFee + burn;
  const percent = collected > 0n ? Number((burn * 1000n) / collected) / 10 : null;
  return (
    <span>
      {burn.toString()}
      {percent !== null ? ` (${percent}% of ${collected.toString()} collected)` : ""}
    </span>
  );
}

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
  const [wasmPoints, setWasmPoints] = useState<bigint>(0n);
  const [nativePoints, setNativePoints] = useState<bigint>(0n);
  const [maxExecutionPoints, setMaxExecutionPoints] = useState<bigint>(0n);
  const [blockWeight, setBlockWeight] = useState<bigint>(0n);
  const [maxBlockWeight, setMaxBlockWeight] = useState<bigint>(0n);
  const [foreignProposals, setForeignProposals] = useState<ForeignProposalAtom[]>([]);
  const [blockExhaustBurn, setBlockExhaustBurn] = useState<bigint | null>(null);

  useEffect(() => {
    if (blockId !== undefined) {
      Promise.all([getBlock({ block_id: blockId }), getIdentity()])
        .then(([resp, identity]) => {
          setIdentity(identity);
          setBlock(resp.block);
          setWasmPoints(resp.total_wasm_execution_points);
          setNativePoints(resp.total_native_execution_points);
          setMaxExecutionPoints(resp.max_block_execution_points);
          setBlockWeight(resp.total_block_execution_weight);
          setMaxBlockWeight(resp.max_block_validation_weight);
          getBlock({ block_id: resp.block.header.parent }).then((justify_block) => {
            if (resp.block.stored_at && justify_block.block.stored_at) {
              let blockTime = resp.block.block_time || 0;
              let justifyTime = justify_block.block.block_time || 0;
              setBlockTime(Math.floor(new Date(blockTime * 1000).getTime() / 1000) - Math.floor(new Date(justifyTime * 1000).getTime() / 1000));
            }
            // The header burn accumulates within an epoch, so this block's burn is the step from
            // the parent, or the whole figure when the parent belongs to the previous epoch.
            const accumulated = BigInt(resp.block.header.accumulated_data.total_exhaust_burn);
            const parentAccumulated =
              justify_block.block.header.epoch === resp.block.header.epoch
                ? BigInt(justify_block.block.header.accumulated_data.total_exhaust_burn)
                : 0n;
            setBlockExhaustBurn(accumulated > parentAccumulated ? accumulated - parentAccumulated : 0n);
          });
          setEpochEvents([]);
          const otherCommands: OtherCommands = {};
          const foreignProposals = [];
          const data: { [key: string]: TransactionAtom[] } = {};
          for (let command of resp.block.commands) {
            if (typeof command === "object") {

              const cmd = Object.keys(command)[0];

              if (COMMANDS.indexOf(cmd) > -1) {
                data[cmd] ||= [];
                data[cmd].push(command[cmd as keyof Command]);
              } else if ("ForeignProposal" in command) {
                foreignProposals.push(command.ForeignProposal);
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

  const handleChange = (panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) => {
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
      <Grid size={12}>
        <PageHeading>Block Details</PageHeading>
      </Grid>
      <Grid size={12}>
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
                            <DataTableCell>{block!.header.epoch} (ShardGroup {block!.header.shard_group.start}-{block!.header.shard_group.end_inclusive})</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Height</TableCell>
                            <DataTableCell>{block!.header.height}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Protocol version</TableCell>
                            <DataTableCell>V{block!.header.protocol_version}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Proposal Certificate</TableCell>
                            <DataTableCell>{block!.justify.height} ({block!.justify.signatures.length} signatures)</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Parent block</TableCell>
                            <DataTableCell>
                              <Link to={`/blocks/${block!.header.parent}`}>{block!.header.parent}</Link>
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Total Fees</TableCell>
                            <DataTableCell>
                              <div className={block!.header.proposed_by === identity!.public_key ? "my_money" : ""}>
                                {block!.header.total_leader_fee}
                              </div>
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell title="The share of the fees collected in this block that is burnt rather than paid to the leader">
                              Fee Burn
                            </TableCell>
                            <DataTableCell>
                              <BlockBurnCell burn={blockExhaustBurn} leaderFee={BigInt(block!.header.total_leader_fee)} />
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell
                              title="The total fees burnt by this shard group for this epoch as part of the exhaust">Accumulated
                              Fee Burn</TableCell>
                            <DataTableCell>
                              {block!.header.accumulated_data.total_exhaust_burn.toString()}
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Execution Weight</TableCell>
                            <DataTableCell>
                              <BlockWeightCell weight={blockWeight} max={maxBlockWeight} />
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Execution Points</TableCell>
                            <DataTableCell>
                              <ExecutionPointsCell wasm={wasmPoints} native={nativePoints} max={maxExecutionPoints} />
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Status</TableCell>
                            <DataTableCell>
                              {/* For some reason, typescript cannot find the commit_qc_id  in Block even though it is there
                              @ts-ignore */}
                              <StatusChip status={block!.commit_qc_id ? "Commit" : "Pending"} />
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Proposed by</TableCell>
                            <DataTableCell>{block!.header.proposed_by}</DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Timeout Height</TableCell>
                            <DataTableCell>
                              {block!.timeout_certificate
                                ? `${block!.timeout_certificate.height} (${block!.timeout_certificate.signatures.length} signatures)`
                                : "--"}
                            </DataTableCell>
                          </TableRow>
                          <TableRow>
                            <TableCell>Block time</TableCell>
                            <DataTableCell>{blockTime} secs</DataTableCell>
                          </TableRow>
                          {block!.stored_at && (
                            <TableRow>
                              <TableCell>Stored at</TableCell>
                              <DataTableCell>
                                {new Date(block!.stored_at).toLocaleString()}
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
                          Foreign Proposal: {proposal.block_id} {JSON.stringify(proposal.shard_group)}
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
