//  Copyright 2025. The Tari Project
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

import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import {
  Alert,
  Box,
  Button,
  Chip,
  Fade,
  Link,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableRow,
  Typography,
} from "@mui/material";
import { useQueryClient } from "@tanstack/react-query";
import type {
  IndexerGetTransactionResultRequest,
  ListRecentTransactionsResponse,
} from "@tari-project/ootle-ts-bindings";
import { saveAs } from "file-saver";
import { useState } from "react";
import { Link as RouterLink } from "react-router-dom";
import { useGetTransactionReceipt } from "../../../api/hooks/useTransactionReceipts";
import { useGetTransaction, useGetTransactionResult } from "../../../api/hooks/useTransactions";
import { Accordion, AccordionDetails, AccordionSummary } from "../../../Components/Accordion";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import StatusChip from "../../../Components/StatusChip";
import { DataTableCell } from "../../../Components/StyledComponents";
import { CURRENCY } from "../../../utils/constants";
import { useDecoder } from "../../../utils/encoded";
import { formatCurrency, isNotFoundError, validateHash } from "../../../utils/helpers";
import SubstateChanges from "../../TransactionReceipts/components/SubstateChanges";
import EventsContent from "./EventsContent";
import FeeReceipt from "./FeeReceipt";
import Inputs from "./Inputs";
import Instructions from "./Instructions";
import LogsContent from "./LogsContent";
import Signers from "./Signers";
import SubstatesContent from "./SubstatesContent";

const isFinalized = (result: any): result is { Finalized: any } =>
  typeof result === "object" && result !== null && "Finalized" in result;

const isRejected = (result: any): result is { Rejected: { details: string; rejected_time: string } } =>
  typeof result === "object" && result !== null && "Rejected" in result;

const isAcceptResult = (result: any): result is { Accept: any } =>
  result && typeof result === "object" && "Accept" in result;

// Fee was charged but the main body was rejected (e.g. it ran out of the per-transaction metering
// budget). The consensus decision is still "Commit", so the execution result is the only place this
// shows up.
const isFeeOnlyResult = (result: any): result is { AcceptFeeRejectRest: [any, any] } =>
  result && typeof result === "object" && "AcceptFeeRejectRest" in result;

function formatRejectReason(reason: any): string {
  if (reason == null) return "";
  if (typeof reason === "string") return reason;
  if (typeof reason === "object") {
    const [key, value] = Object.entries(reason)[0] ?? [];
    if (key === undefined) return "";
    return value == null ? String(key) : `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`;
  }
  return String(reason);
}

const Empty = ({ message }: { message: string }) => (
  <Stack alignItems="center" sx={{ p: 3 }}>
    <Typography variant="body2" color="text.secondary">
      {message}
    </Typography>
  </Stack>
);

function Result({ transaction_id }: Pick<IndexerGetTransactionResultRequest, "transaction_id">) {
  const [expandedPanels, setExpandedPanels] = useState<string[]>([]);
  const { decodeExecuteResult, decodeTransactionReceipt } = useDecoder();
  const normalizedId = transaction_id.toLowerCase();
  const isValidHash = validateHash(normalizedId);
  const result = useGetTransactionResult(normalizedId);

  // Validators prune a transaction's execution result a couple of epochs after it finalizes, after
  // which its committee answers 404. The receipt synced from network state still records how it
  // committed, so it stands in for the result.
  const receiptQuery = useGetTransactionReceipt(normalizedId, isValidHash && result.isError);

  const queryClient = useQueryClient();
  const cachedEntry = queryClient
    .getQueriesData<ListRecentTransactionsResponse>({ queryKey: ["recent_transactions"] })
    .flatMap(([, list]) => list?.transactions ?? [])
    .find((tx) => tx.transaction_id === normalizedId);

  // The recent-transactions list cache is only populated when arriving from the list page. On a fresh
  // page load / direct navigation it's empty, so fetch the transaction body directly as a fallback. The
  // result endpoint never carries instructions, hence this separate fetch.
  const transactionQuery = useGetTransaction(normalizedId, isValidHash && !cachedEntry);

  const txEntry = cachedEntry ?? transactionQuery.data?.transaction;
  const txV1 = txEntry?.transaction?.V1;
  const txBody = txV1?.body;
  const transaction = txBody?.transaction;

  if (!isValidHash) {
    return <Alert severity="error">Invalid Hash</Alert>;
  }

  const finalized = result.data?.result && isFinalized(result.data.result) ? result.data.result.Finalized : undefined;
  const rejected = result.data?.result && isRejected(result.data.result) ? result.data.result.Rejected : undefined;
  const receipt = receiptQuery.data ? decodeTransactionReceipt(receiptQuery.data.receipt) : undefined;
  const executionResult = finalized?.execution_result ? decodeExecuteResult(finalized.execution_result) : undefined;
  const finalize = executionResult?.finalize;
  const execResult: any = finalize?.result;
  const feeOnly = finalized ? isFeeOnlyResult(execResult) : receipt?.outcome === "FeeIntentCommit";
  const feeReceipt = finalize?.fee_receipt ?? receipt?.fee_receipt;
  const events = finalize?.events ?? receipt?.events ?? [];

  // The body only decides whether the page can render at all once the result lookup has failed.
  const isLoading =
    result.isPending ||
    (result.isError && receiptQuery.isPending) ||
    (result.isError && !cachedEntry && transactionQuery.isPending);
  // Each source answers on its own, so the page fails only when none of them knows the transaction.
  const isError = result.isError && !receipt && !txEntry;
  const resultPruned = isNotFoundError(result.error);
  // A failure other than "not found" says nothing about whether the transaction exists, so its
  // message is shown instead of a conclusion drawn from the missing data.
  const lookupFailure = !resultPruned
    ? result.error?.message
    : receiptQuery.isError && !isNotFoundError(receiptQuery.error)
      ? receiptQuery.error.message
      : undefined;

  const handleChange = (panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) => {
    setExpandedPanels((prev) => (isExpanded ? [...prev, panel] : prev.filter((p) => p !== panel)));
  };

  const expandAll = () => setExpandedPanels(["p1", "p2", "p3", "p4", "p5", "p7", "p8", "p9", "p10"]);

  const collapseAll = () => setExpandedPanels([]);

  const status = finalized ? (
    <StatusChip status={finalized.final_decision} showTitle={true} feeOnly={feeOnly} />
  ) : receipt ? (
    <StatusChip status="Commit" showTitle={true} feeOnly={feeOnly} />
  ) : rejected ? (
    <Chip label="Rejected" color="error" variant="filled" />
  ) : result.isError ? (
    <Chip label="Unknown" variant="outlined" />
  ) : (
    <Chip label="Pending" color="warning" variant="filled" />
  );

  return (
    <FetchStatusCheck
      isLoading={isLoading}
      isError={isError}
      errorMessage={result.error ? result.error.message : "Error fetching transaction details."}
    >
      <Fade in={!isLoading && !isError}>
        <Box>
          {receipt && !finalized && (
            <Alert severity="info" sx={{ mb: 2 }}>
              {resultPruned
                ? "Validators no longer hold this transaction's execution result."
                : `Failed to fetch this transaction's execution result: ${result.error?.message}.`}{" "}
              Showing its committed receipt instead; logs and per-instruction results are not part of a receipt.
            </Alert>
          )}
          {result.isError && !receipt && (
            <Alert severity="warning" sx={{ mb: 2 }}>
              {lookupFailure
                ? `Failed to fetch this transaction's outcome: ${lookupFailure}`
                : "No validator holds a result for this transaction and it has no committed receipt. It may have aborted, been rejected, or expired without being sequenced."}
            </Alert>
          )}

          {/* Summary table */}
          <TableContainer sx={{ mb: 2 }}>
            <Table>
              <TableBody>
                <TableRow>
                  <TableCell>Transaction Hash</TableCell>
                  <DataTableCell>{normalizedId}</DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>{finalized || receipt ? "Decision" : "Status"}</TableCell>
                  <DataTableCell>{status}</DataTableCell>
                </TableRow>
                {finalized && isFeeOnlyResult(execResult) && (
                  <TableRow>
                    <TableCell>Rejection Reason</TableCell>
                    <DataTableCell>{formatRejectReason(execResult.AcceptFeeRejectRest[1])}</DataTableCell>
                  </TableRow>
                )}
                {rejected && (
                  <>
                    <TableRow>
                      <TableCell>Rejection Reason</TableCell>
                      <DataTableCell>{rejected.details}</DataTableCell>
                    </TableRow>
                    <TableRow>
                      <TableCell>Rejected Time</TableCell>
                      <DataTableCell>{rejected.rejected_time}</DataTableCell>
                    </TableRow>
                  </>
                )}
                {!rejected && !finalized && txEntry?.rejected_reason && (
                  <TableRow>
                    <TableCell>Rejection Reason</TableCell>
                    <DataTableCell>{txEntry.rejected_reason}</DataTableCell>
                  </TableRow>
                )}
                {(finalized || receipt) && (
                  <TableRow>
                    <TableCell>Finalized Time</TableCell>
                    <DataTableCell>
                      {finalized?.finalized_time || txEntry?.summary?.finalized_at || "N/A"}
                    </DataTableCell>
                  </TableRow>
                )}
                {finalized && (
                  <TableRow>
                    <TableCell>Execution Time</TableCell>
                    <DataTableCell>
                      {executionResult?.execution_time
                        ? `${executionResult.execution_time.secs}s ${Math.round(
                            executionResult.execution_time.nanos / 1_000_000,
                          )}ms`
                        : "N/A"}
                    </DataTableCell>
                  </TableRow>
                )}
                {receipt && (
                  <TableRow>
                    <TableCell>Receipt</TableCell>
                    <DataTableCell>
                      <Link component={RouterLink} to={`/transaction-receipts/${normalizedId}`}>
                        Committed in epoch {String(receipt.epoch)}
                      </Link>
                    </DataTableCell>
                  </TableRow>
                )}
                {(finalized || receipt) && (
                  <TableRow>
                    <TableCell>Total Fees</TableCell>
                    <DataTableCell>
                      {feeReceipt?.total_fees_paid
                        ? formatCurrency(feeReceipt.total_fees_paid, CURRENCY.DECIMALS, CURRENCY.SYMBOL)
                        : "--"}
                    </DataTableCell>
                  </TableRow>
                )}
                {finalized?.abort_details && (
                  <TableRow>
                    <TableCell>Abort Details</TableCell>
                    <DataTableCell>{finalized.abort_details}</DataTableCell>
                  </TableRow>
                )}
                {txEntry && (
                  <TableRow>
                    <TableCell>Seen Via</TableCell>
                    <DataTableCell>{txEntry.source === "gossip" ? "Network gossip" : "Submitted here"}</DataTableCell>
                  </TableRow>
                )}
                {transaction?.min_epoch != null && (
                  <TableRow>
                    <TableCell>Min Epoch</TableCell>
                    <DataTableCell>{transaction.min_epoch.toString()}</DataTableCell>
                  </TableRow>
                )}
                {transaction?.max_epoch != null && (
                  <TableRow>
                    <TableCell>Max Epoch</TableCell>
                    <DataTableCell>{transaction.max_epoch.toString()}</DataTableCell>
                  </TableRow>
                )}
                <TableRow>
                  <TableCell>Download</TableCell>
                  <DataTableCell>
                    <Button
                      variant="outlined"
                      size="small"
                      onClick={() => {
                        const json = JSON.stringify(
                          { result: result.data?.result, receipt, transaction: txEntry?.transaction },
                          null,
                          2,
                        );
                        const blob = new Blob([json], { type: "application/json" });
                        saveAs(blob, `tx-${normalizedId}.json`);
                      }}
                    >
                      Download JSON
                    </Button>
                  </DataTableCell>
                </TableRow>
              </TableBody>
            </Table>
          </TableContainer>

          {(finalized || receipt || txEntry) && (
            <>
              {/* Expand / Collapse controls */}
              <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ px: 1, pb: 1 }}>
                <Typography variant="h5">Details</Typography>
                <Stack direction="row" spacing={1}>
                  <Button size="small" startIcon={<KeyboardArrowDownIcon />} onClick={expandAll}>
                    Expand All
                  </Button>
                  <Button
                    size="small"
                    startIcon={<KeyboardArrowUpIcon />}
                    onClick={collapseAll}
                    disabled={expandedPanels.length === 0}
                  >
                    Collapse All
                  </Button>
                </Stack>
              </Stack>

              {/* Fee Instructions */}
              <Accordion expanded={expandedPanels.includes("p1")} onChange={handleChange("p1")}>
                <AccordionSummary>
                  <Typography variant="h5">Fee Instructions</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  {transaction?.fee_instructions?.length ? (
                    <Instructions data={transaction.fee_instructions} />
                  ) : (
                    <Empty message="No fee instructions available" />
                  )}
                </AccordionDetails>
              </Accordion>

              {/* Instructions */}
              <Accordion expanded={expandedPanels.includes("p2")} onChange={handleChange("p2")}>
                <AccordionSummary>
                  <Typography variant="h5">Instructions</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  {transaction?.instructions?.length ? (
                    <Instructions data={transaction.instructions} />
                  ) : (
                    <Empty message="No instructions available" />
                  )}
                </AccordionDetails>
              </Accordion>

              {/* Blobs */}
              <Accordion expanded={expandedPanels.includes("p10")} onChange={handleChange("p10")}>
                <AccordionSummary>
                  <Typography variant="h5">Blobs ({transaction?.blob_hashes?.length ?? 0})</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <BlobsContent hashes={transaction?.blob_hashes || []} sizes={transaction?.blob_sizes || []} />
                </AccordionDetails>
              </Accordion>

              {/* Events */}
              {events.length ? (
                <Accordion expanded={expandedPanels.includes("p3")} onChange={handleChange("p3")}>
                  <AccordionSummary>
                    <Typography variant="h5">Events ({events.length})</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <EventsContent data={events} />
                  </AccordionDetails>
                </Accordion>
              ) : null}

              {/* Logs */}
              {finalize?.logs?.length ? (
                <Accordion expanded={expandedPanels.includes("p4")} onChange={handleChange("p4")}>
                  <AccordionSummary>
                    <Typography variant="h5">Logs ({finalize.logs.length})</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <LogsContent data={finalize.logs} />
                  </AccordionDetails>
                </Accordion>
              ) : null}

              {/* Substates */}
              {execResult && isAcceptResult(execResult) ? (
                <Accordion expanded={expandedPanels.includes("p5")} onChange={handleChange("p5")}>
                  <AccordionSummary>
                    <Typography variant="h5">Substates</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <SubstatesContent result={execResult} />
                  </AccordionDetails>
                </Accordion>
              ) : !finalized && receipt ? (
                <Accordion expanded={expandedPanels.includes("p5")} onChange={handleChange("p5")}>
                  <AccordionSummary>
                    <Typography variant="h5">
                      Substate Changes ({receipt.diff_summary.upped.length + receipt.diff_summary.downed.length})
                    </Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <SubstateChanges upped={receipt.diff_summary.upped} downed={receipt.diff_summary.downed} />
                  </AccordionDetails>
                </Accordion>
              ) : null}

              {/* Fee Receipt */}
              {feeReceipt && (
                <Accordion expanded={expandedPanels.includes("p7")} onChange={handleChange("p7")}>
                  <AccordionSummary>
                    <Typography variant="h5">Fee Receipt</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <FeeReceipt data={feeReceipt} />
                  </AccordionDetails>
                </Accordion>
              )}

              {/* Inputs */}
              <Accordion expanded={expandedPanels.includes("p8")} onChange={handleChange("p8")}>
                <AccordionSummary>
                  <Typography variant="h5">Inputs</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <Inputs data={transaction?.inputs || []} />
                </AccordionDetails>
              </Accordion>

              {/* Signers */}
              <Accordion expanded={expandedPanels.includes("p9")} onChange={handleChange("p9")}>
                <AccordionSummary>
                  <Typography variant="h5">Signers</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <Signers seal_signature={txV1?.seal_signature} transaction_body={txBody} />
                </AccordionDetails>
              </Accordion>
            </>
          )}
        </Box>
      </Fade>
    </FetchStatusCheck>
  );
}

function formatBlobSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
}

function BlobsContent({ hashes, sizes }: { hashes: string[]; sizes: number[] }) {
  if (hashes.length === 0) {
    return <Empty message="This transaction has no blobs." />;
  }
  return (
    <Table size="small">
      <TableBody>
        <TableRow>
          <TableCell sx={{ fontWeight: 600 }}>Index</TableCell>
          <TableCell sx={{ fontWeight: 600 }}>Hash</TableCell>
          <TableCell sx={{ fontWeight: 600 }}>Size</TableCell>
        </TableRow>
        {hashes.map((hash, i) => {
          const size = sizes[i];
          return (
            <TableRow key={`${i}-${hash}`}>
              <DataTableCell sx={{ width: "10%" }}>{i}</DataTableCell>
              <DataTableCell sx={{ fontFamily: "monospace", fontSize: "0.8rem", wordBreak: "break-all" }}>
                {hash}
              </DataTableCell>
              <DataTableCell sx={{ width: "15%" }}>{size != null ? formatBlobSize(size) : "—"}</DataTableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export default Result;
