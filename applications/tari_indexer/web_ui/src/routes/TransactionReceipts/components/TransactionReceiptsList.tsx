//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
import { ChevronRight } from "@mui/icons-material";
import { Chip, Stack } from "@mui/material";
import Fade from "@mui/material/Fade";
import IconButton from "@mui/material/IconButton";
import { useTheme } from "@mui/material/styles";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TablePagination from "@mui/material/TablePagination";
import TableRow from "@mui/material/TableRow";
import useMediaQuery from "@mui/material/useMediaQuery";
import type { TransactionReceipt, TransactionReceiptAddress } from "@tari-project/ootle-ts-bindings";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useListTransactionReceipts } from "../../../api/hooks/useTransactionReceipts";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import { DataTableCell } from "../../../Components/StyledComponents";
import { CURRENCY } from "../../../utils/constants";
import { encodedByteLength, useDecoder } from "../../../utils/encoded";
import { formatCurrency } from "../../../utils/helpers";
import TransactionFilter from "../../RecentTransactions/components/SearchFilter";
import { shortenString } from "../../VN/Components/helpers";

type ReceiptEntry = {
  id: string;
  address: TransactionReceiptAddress;
  /** The receipt, or undefined while it is not decoded. */
  receipt?: TransactionReceipt;
  encoded: string;
  show?: boolean;
};

function OutcomeChip({ outcome }: { outcome: string }) {
  if (outcome === "Commit" || outcome === "FeeIntentCommit") {
    return <Chip label={outcome} color="success" size="small" variant="outlined" />;
  }
  return <Chip label={outcome} color="error" size="small" variant="outlined" />;
}

function ReceiptRow({ entry }: { entry: ReceiptEntry }) {
  const { address, receipt, encoded } = entry;
  const theme = useTheme();
  const isSm = useMediaQuery(theme.breakpoints.down("sm"));
  const shortLen = isSm ? 4 : 8;

  return (
    <TableRow>
      <DataTableCell>
        <Link
          to={`/transaction-receipts/${encodeURIComponent(address)}`}
          style={{ textDecoration: "none", color: theme.palette.text.secondary }}
        >
          {shortenString(address, shortLen, shortLen)}
        </Link>
      </DataTableCell>
      {receipt ? (
        <>
          <DataTableCell>
            <OutcomeChip outcome={receipt.outcome} />
          </DataTableCell>
          <DataTableCell>
            {formatCurrency(receipt.fee_receipt.total_fees_paid, CURRENCY.DECIMALS, CURRENCY.SYMBOL)}
          </DataTableCell>
          <DataTableCell>{receipt.events.length}</DataTableCell>
          <DataTableCell>{String(receipt.epoch)}</DataTableCell>
        </>
      ) : (
        <DataTableCell colSpan={4}>CBOR, {encodedByteLength(encoded)} bytes</DataTableCell>
      )}
      <DataTableCell>
        <IconButton
          component={Link}
          to={`/transaction-receipts/${encodeURIComponent(address)}`}
          style={{ color: theme.palette.text.secondary }}
        >
          <ChevronRight />
        </IconButton>
      </DataTableCell>
    </TableRow>
  );
}

function TransactionReceiptsList() {
  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);
  const [filteredReceipts, setFilteredReceipts] = useState<ReceiptEntry[]>([]);

  const { data, isLoading, isError, error } = useListTransactionReceipts(100);
  const { decodeTransactionReceipt } = useDecoder();

  const receipts = useMemo(
    () =>
      (data?.receipts || []).map(([address, encoded]) => ({
        id: address,
        address,
        receipt: decodeTransactionReceipt(encoded),
        encoded,
      })),
    [data?.receipts, decodeTransactionReceipt],
  );

  const visibleReceipts = filteredReceipts.filter((r) => r.show !== false);
  const paginatedReceipts = visibleReceipts.slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage);

  const handleChangePage = (_event: React.MouseEvent<HTMLButtonElement> | null, newPage: number) => {
    setPage(newPage);
  };
  const handleChangeRowsPerPage = (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  return (
    <FetchStatusCheck
      isLoading={isLoading}
      isError={isError}
      errorMessage={error ? error.message : "Error fetching transaction receipts."}
    >
      <Stack spacing={1}>
        <TransactionFilter
          setPage={setPage}
          stateObject={receipts}
          setStateObject={setFilteredReceipts}
          filterItems={[
            {
              title: "Receipt Address",
              value: "id",
              filterFn: (value, row) => row.address.toLowerCase().includes(value.toLowerCase()),
            },
          ]}
          placeholder="Search receipts"
          defaultSearch="id"
        />
        <Fade in={!isLoading && !isError}>
          <TableContainer>
            <Table>
              <TableHead>
                <TableRow>
                  <TableCell>Receipt Address</TableCell>
                  <TableCell>Outcome</TableCell>
                  <TableCell>Total Fees</TableCell>
                  <TableCell>Events</TableCell>
                  <TableCell>Epoch</TableCell>
                  <TableCell>Details</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {paginatedReceipts.map((entry) => (
                  <ReceiptRow key={entry.address} entry={entry} />
                ))}
              </TableBody>
            </Table>
            <TablePagination
              component="div"
              count={visibleReceipts.length}
              page={page}
              onPageChange={handleChangePage}
              rowsPerPage={rowsPerPage}
              onRowsPerPageChange={handleChangeRowsPerPage}
              rowsPerPageOptions={[5, 10, 25, 50]}
            />
          </TableContainer>
        </Fade>
      </Stack>
    </FetchStatusCheck>
  );
}

export default TransactionReceiptsList;
