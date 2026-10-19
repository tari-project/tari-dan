// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import CopyToClipboard from "@components/CopyToClipboard";
import Error from "@components/Error";
import Loading from "@components/Loading";
import PageHeading from "@components/PageHeading";
import { DataTableCell, StyledPaper } from "@components/StyledComponents";
import Alert from "@mui/material/Alert";
import Grid from "@mui/material/Grid";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableRow from "@mui/material/TableRow";
import Typography from "@mui/material/Typography";
import { useQuery } from "@tanstack/react-query";
import { TrustedValidatorSetInfo } from "@tari-project/ootle-ts-bindings";
import { settingsGet, validatorSetGetStatus } from "@utils/json_rpc";
import { ReactNode } from "react";

const REFRESH_INTERVAL_MS = 30_000;

function formatUnixTime(seconds: number | null): string {
  if (seconds == null) {
    return "Never";
  }
  const date = new Date(seconds * 1000);
  return `${date.toLocaleString()} (${timeAgo(date)})`;
}

function timeAgo(date: Date): string {
  const seconds = Math.max(0, Math.floor((Date.now() - date.getTime()) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <TableRow>
      <TableCell>{label}</TableCell>
      <DataTableCell>{children}</DataTableCell>
    </TableRow>
  );
}

function Hash({ hash }: { hash: string }) {
  return (
    <>
      <span style={{ fontFamily: "monospace", wordBreak: "break-all" }}>{hash}</span>
      <CopyToClipboard copy={hash} />
    </>
  );
}

function TrustedSet({ trusted, networkEpoch }: { trusted: TrustedValidatorSetInfo; networkEpoch: number | null }) {
  return (
    <>
      {trusted.walk_failing_since != null && (
        <Alert severity="warning" sx={{ mb: 2 }}>
          The wallet cannot walk to the indexer's latest checkpoint. It has failed on every sync since{" "}
          {formatUnixTime(trusted.walk_failing_since)}. After a day of failures the wallet trusts its indexer's
          validator set again.
        </Alert>
      )}
      <TableContainer>
        <Table>
          <TableBody>
            <Row label="Latest trusted epoch">{trusted.epoch}</Row>
            <Row label="Network epoch">{networkEpoch ?? "Unknown (indexer unreachable)"}</Row>
            <Row label="Latest validator set hash">
              <Hash hash={trusted.validator_set_hash} />
            </Row>
            <Row label="Anchor epoch">{trusted.anchor_epoch}</Row>
            <Row label="Anchor validator set hash">
              <Hash hash={trusted.anchor_validator_set_hash} />
            </Row>
            <Row label="Anchored by">
              {trusted.anchor === "FirstUse" ? "First use" : "Re-anchor"} — trusted on the indexer's word
            </Row>
            <Row label="Anchored at">{formatUnixTime(trusted.anchored_at)}</Row>
            <Row label="Last synced">{formatUnixTime(trusted.synced_at)}</Row>
            <Row label="Walk failing since">
              {trusted.walk_failing_since == null ? "—" : formatUnixTime(trusted.walk_failing_since)}
            </Row>
          </TableBody>
        </Table>
      </TableContainer>
    </>
  );
}

function ValidatorSetPage() {
  const status = useQuery({
    queryKey: ["validator_set_status"],
    queryFn: validatorSetGetStatus,
    refetchInterval: REFRESH_INTERVAL_MS,
  });
  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: settingsGet,
    refetchInterval: REFRESH_INTERVAL_MS,
  });

  const renderContent = () => {
    if (status.isLoading) {
      return <Loading />;
    }
    if (status.isError) {
      return <Error message={status.error.message} />;
    }
    if (!status.data) {
      return null;
    }
    const { indexer_trust, trusted } = status.data;
    return (
      <>
        {indexer_trust === "trust" ? (
          <Alert severity="info" sx={{ mb: 2 }}>
            This wallet trusts its indexer and does not verify what it serves against a validator set.
          </Alert>
        ) : (
          <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
            The wallet verifies the transaction results and substates its indexer serves against the validator sets it
            trusts. It anchors on the first validator set its indexer serves, and walks forward from it one signed
            checkpoint per epoch.
          </Typography>
        )}
        {trusted ? (
          <TrustedSet trusted={trusted} networkEpoch={settings.data?.current_epoch ?? null} />
        ) : (
          indexer_trust === "verify" && (
            <Alert severity="info">
              The wallet trusts no validator set yet. It anchors on the first checkpoint its indexer serves at its next
              sync, which runs every 10 minutes and before each submission. A wallet that stays unanchored usually
              cannot reach its indexer, or the network has not yet finished its first epoch.
            </Alert>
          )
        )}
      </>
    );
  };

  return (
    <>
      <Grid size={12}>
        <PageHeading>Validator Set</PageHeading>
      </Grid>
      <Grid size={12}>
        <StyledPaper>{renderContent()}</StyledPaper>
      </Grid>
    </>
  );
}

export default ValidatorSetPage;
