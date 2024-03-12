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
import { useLocation } from "react-router-dom";
import { useTransactionDetails } from "../../api/hooks/useTransactions";
import { Accordion, AccordionDetails, AccordionSummary } from "../../Components/Accordion";
import { Grid, Table, TableContainer, TableBody, TableRow, TableCell, Button, Fade, Alert } from "@mui/material";
import Typography from "@mui/material/Typography";
import { saveAs } from "file-saver";
import { DataTableCell, StyledPaper } from "../../Components/StyledComponents";
import PageHeading from "../../Components/PageHeading";
import Events from "./Events";
import Logs from "./Logs";
import FeeInstructions from "./FeeInstructions";
import Instructions from "./Instructions";
import Substates from "./Substates";
import StatusChip from "../../Components/StatusChip";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import Loading from "../../Components/Loading";
import Error from "../../Components/Error";
import type { FinalizeResult, RejectReason, TransactionResult } from "@tariproject/typescript-bindings";
import { getRejectReasonFromTransactionResult, rejectReasonToString } from "@tariproject/typescript-bindings";

export default function TransactionDetails() {
  const [expandedPanels, setExpandedPanels] = useState<string[]>([]);
  const location = useLocation();
  const { data, isLoading, isError, error } = useTransactionDetails(location.pathname.split("/")[2]);

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

  const renderResult = (result: FinalizeResult | null) => {
    if (result) {
      if ("Accept" in result.result) {
        return <span>Accepted</span>;
      }
      return <span>{rejectReasonToString(getRejectReasonFromTransactionResult(result.result))}</span>;
    } else {
      return <span>In progress</span>;
    }
  };

  const renderContent = () => {
    if (isLoading) {
      return <Loading />;
    }

    if (isError) {
      return <Error message={error.message} />;
    }

    if (!data) {
      return null;
    }

    if (data.status === "Pending") {
      return (
        <>
          <Typography variant="body1" style={{ textAlign: "center" }}>
            Transaction is still pending. Please check back later.
          </Typography>
        </>
      );
    }

    const last_update_time = new Date(data.last_update_time);
    const handleDownload = () => {
      const json = JSON.stringify(data, null, 2);
      const blob = new Blob([json], { type: "application/json" });
      const filename = `tx-${data?.transaction?.id}.json` || "tx-unknown_id.json";
      saveAs(blob, filename);
    };

    const getTransactionFailure = (txResult: TransactionResult | undefined): string => {
      if (txResult === undefined || "Accept" in txResult) {
        return "No reason";
      }
      let reason;
      if ("AcceptFeeRejectRest" in txResult) {
        reason = txResult.AcceptFeeRejectRest[1];
      } else {
        reason = txResult.Reject;
      }
      const getReasonString = (x: RejectReason): string => {
        if (typeof x === "string") {
          return x;
        } else if ("ShardsNotPledged" in x) {
          return `ShardsNotPledged: ${x["ShardsNotPledged"]}`;
        } else if ("ExecutionFailure" in x) {
          return `ExecutionFailure: ${x["ExecutionFailure"]}`;
        } else if ("ShardPledgedToAnotherPayload" in x) {
          return `ShardPledgedToAnotherPayload: ${x["ShardPledgedToAnotherPayload"]}`;
        } else if ("ShardRejected" in x) {
          return `ShardRejected: ${x["ShardRejected"]}`;
        } else if ("FeesNotPaid" in x) {
          return `FeesNotPaid: ${x["FeesNotPaid"]}`;
        }
        return "Unknown reason";
      };
      return getReasonString(reason);
    };

    if (data.status === "Rejected" || data.status === "InvalidTransaction") {
      return (
        <>
          <TableContainer>
            <Table>
              <TableBody>
                <TableRow>
                  <TableCell>Transaction Hash</TableCell>
                  <DataTableCell>{data.transaction.id}</DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Timestamp</TableCell>
                  <DataTableCell>{last_update_time.toLocaleString()}</DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Status</TableCell>
                  <DataTableCell>
                    <StatusChip status={data.status} />
                  </DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>JSON</TableCell>
                  <DataTableCell>
                    <Button variant="outlined" onClick={handleDownload}>
                      Download
                    </Button>
                  </DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Reason</TableCell>
                  <DataTableCell>{getTransactionFailure(data?.result?.result)}</DataTableCell>
                </TableRow>
              </TableBody>
            </Table>
          </TableContainer>
        </>
      );
    }

    return (
      <Fade in={!isLoading}>
        <div>
          <>
            <TableContainer>
              <Table>
                <TableBody>
                  <TableRow>
                    <TableCell>Transaction Hash</TableCell>
                    <DataTableCell>{data.transaction.id}</DataTableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>Timestamp</TableCell>
                    <DataTableCell>{last_update_time.toLocaleString()}</DataTableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>Total Fees</TableCell>
                    <DataTableCell>{data?.result?.fee_receipt.total_fees_paid || 0}</DataTableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>Status</TableCell>
                    <DataTableCell>
                      <StatusChip status={data.status} />
                    </DataTableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>Result</TableCell>
                    <DataTableCell>{renderResult(data?.result)}</DataTableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>JSON</TableCell>
                    <DataTableCell>
                      <Button variant="outlined" onClick={handleDownload}>
                        Download
                      </Button>
                    </DataTableCell>
                  </TableRow>
                  {data?.result?.result ? (
                    <TableRow>
                      <TableCell>Reason</TableCell>
                      <DataTableCell>{getTransactionFailure(data?.result?.result)}</DataTableCell>
                    </TableRow>
                  ) : null}
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
          <Accordion expanded={expandedPanels.includes("panel1")} onChange={handleChange("panel1")}>
            <AccordionSummary aria-controls="panel1bh-content" id="panel1bh-header">
              <Typography>Fee Instructions</Typography>
            </AccordionSummary>
            <AccordionDetails>
              {data.transaction?.fee_instructions.length > 0 ? (
                <FeeInstructions data={data.transaction.fee_instructions} />
              ) : (
                <span>Empty</span>
              )}
            </AccordionDetails>
          </Accordion>
          <Accordion expanded={expandedPanels.includes("panel2")} onChange={handleChange("panel2")}>
            <AccordionSummary aria-controls="panel2bh-content" id="panel1bh-header">
              <Typography>Instructions</Typography>
            </AccordionSummary>
            <AccordionDetails>
              {data.transaction?.instructions?.length > 0 ? (
                <Instructions data={data.transaction.instructions} />
              ) : (
                <span>Empty</span>
              )}
            </AccordionDetails>
          </Accordion>
          {data.result && (
            <Accordion expanded={expandedPanels.includes("panel3")} onChange={handleChange("panel3")}>
              <AccordionSummary aria-controls="panel3bh-content" id="panel1bh-header">
                <Typography>Events</Typography>
              </AccordionSummary>
              <AccordionDetails>
                <Events data={data.result.events} />
              </AccordionDetails>
            </Accordion>
          )}
          {data.result && (
            <Accordion expanded={expandedPanels.includes("panel4")} onChange={handleChange("panel4")}>
              <AccordionSummary aria-controls="panel4bh-content" id="panel1bh-header">
                <Typography>Logs</Typography>
              </AccordionSummary>
              <AccordionDetails>
                <Logs data={data.result.logs} />
              </AccordionDetails>
            </Accordion>
          )}
          {data.result && (
            <Accordion expanded={expandedPanels.includes("panel5")} onChange={handleChange("panel5")}>
              <AccordionSummary aria-controls="panel5bh-content" id="panel1bh-header">
                <Typography>Substates</Typography>
              </AccordionSummary>
              <AccordionDetails>
                <Substates data={data.result.result} />
              </AccordionDetails>
            </Accordion>
          )}
        </div>
      </Fade>
    );
  };

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Transaction Details</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>{renderContent()}</StyledPaper>
      </Grid>
    </>
  );
}
