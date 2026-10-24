//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import {
  Alert,
  Box,
  Button,
  Chip,
  Fade,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableRow,
  Typography,
} from "@mui/material";
import { saveAs } from "file-saver";
import { useState } from "react";
import { useGetTransactionReceipt } from "../../../api/hooks/useTransactionReceipts";
import { Accordion, AccordionDetails, AccordionSummary } from "../../../Components/Accordion";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import { DataTableCell } from "../../../Components/StyledComponents";
import { CURRENCY } from "../../../utils/constants";
import { encodedByteLength, useDecoder } from "../../../utils/encoded";
import { formatCurrency } from "../../../utils/helpers";
import EventsContent from "../../Transaction/components/EventsContent";
import FeeReceipt from "../../Transaction/components/FeeReceipt";
import SubstateChanges from "./SubstateChanges";

function TransactionReceiptDetails({ address }: { address: string }) {
  const [expandedPanels, setExpandedPanels] = useState<string[]>([]);
  const { data, isLoading, error, isError } = useGetTransactionReceipt(address);
  const { decodeTransactionReceipt } = useDecoder();

  if (!address) {
    return <Alert severity="error">No receipt address provided</Alert>;
  }

  const handleChange = (panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) => {
    setExpandedPanels((prev) => (isExpanded ? [...prev, panel] : prev.filter((p) => p !== panel)));
  };

  const expandAll = () => setExpandedPanels(["p1", "p3", "p4", "p5"]);
  const collapseAll = () => setExpandedPanels([]);

  const receipt = data && decodeTransactionReceipt(data.receipt);

  return (
    <FetchStatusCheck
      isLoading={isLoading}
      isError={isError}
      errorMessage={error ? error.message : "Error fetching transaction receipt."}
    >
      <Fade in={!isLoading && !isError}>
        <Box>
          {receipt ? (
            <>
              <TableContainer sx={{ mb: 2 }}>
                <Table>
                  <TableBody>
                    <TableRow>
                      <TableCell>Receipt Address</TableCell>
                      <DataTableCell sx={{ wordBreak: "break-all" }}>{address}</DataTableCell>
                    </TableRow>
                    <TableRow>
                      <TableCell>Outcome</TableCell>
                      <DataTableCell>
                        <Chip
                          label={receipt.outcome}
                          color={
                            receipt.outcome === "Commit" || receipt.outcome === "FeeIntentCommit" ? "success" : "error"
                          }
                          size="small"
                          variant="outlined"
                        />
                      </DataTableCell>
                    </TableRow>
                    <TableRow>
                      <TableCell>Epoch</TableCell>
                      <DataTableCell>{String(receipt.epoch)}</DataTableCell>
                    </TableRow>
                    <TableRow>
                      <TableCell>Total Fees Paid</TableCell>
                      <DataTableCell>
                        {formatCurrency(receipt.fee_receipt.total_fees_paid, CURRENCY.DECIMALS, CURRENCY.SYMBOL)}
                      </DataTableCell>
                    </TableRow>
                    <TableRow>
                      <TableCell>Download</TableCell>
                      <DataTableCell>
                        <Button
                          variant="outlined"
                          size="small"
                          onClick={() => {
                            const json = JSON.stringify({ address, receipt }, null, 2);
                            const blob = new Blob([json], { type: "application/json" });
                            saveAs(blob, `receipt-${address}.json`);
                          }}
                        >
                          Download JSON
                        </Button>
                      </DataTableCell>
                    </TableRow>
                  </TableBody>
                </Table>
              </TableContainer>

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

              {receipt.events.length > 0 && (
                <Accordion expanded={expandedPanels.includes("p1")} onChange={handleChange("p1")}>
                  <AccordionSummary>
                    <Typography variant="h5">Events ({receipt.events.length})</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <EventsContent data={receipt.events} />
                  </AccordionDetails>
                </Accordion>
              )}

              {receipt.diff_summary.upped.length + receipt.diff_summary.downed.length > 0 && (
                <Accordion expanded={expandedPanels.includes("p3")} onChange={handleChange("p3")}>
                  <AccordionSummary>
                    <Typography variant="h5">
                      Substate Changes ({receipt.diff_summary.upped.length + receipt.diff_summary.downed.length})
                    </Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <SubstateChanges upped={receipt.diff_summary.upped} downed={receipt.diff_summary.downed} />
                  </AccordionDetails>
                </Accordion>
              )}

              <Accordion expanded={expandedPanels.includes("p4")} onChange={handleChange("p4")}>
                <AccordionSummary>
                  <Typography variant="h5">Fee Receipt</Typography>
                </AccordionSummary>
                <AccordionDetails>
                  <FeeReceipt data={receipt.fee_receipt} />
                </AccordionDetails>
              </Accordion>

              {receipt.fee_withdrawals.length > 0 && (
                <Accordion expanded={expandedPanels.includes("p5")} onChange={handleChange("p5")}>
                  <AccordionSummary>
                    <Typography variant="h5">Fee Withdrawals ({receipt.fee_withdrawals.length})</Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    <Table>
                      <TableBody>
                        {receipt.fee_withdrawals.map((withdrawal, i) => (
                          <TableRow key={i}>
                            <DataTableCell>{JSON.stringify(withdrawal)}</DataTableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </AccordionDetails>
                </Accordion>
              )}
            </>
          ) : data ? (
            <Alert severity="info">The receipt is not decoded: CBOR, {encodedByteLength(data.receipt)} bytes</Alert>
          ) : (
            <Alert severity="info">No receipt found</Alert>
          )}
        </Box>
      </Fade>
    </FetchStatusCheck>
  );
}

export default TransactionReceiptDetails;
