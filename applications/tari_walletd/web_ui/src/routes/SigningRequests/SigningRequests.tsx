//  Copyright 2026 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import {
  useApproveSigningRequest,
  useListAllSigningRequests,
  useListPendingSigningRequests,
  useRejectSigningRequest,
} from "@api/hooks/useSigningRequests";
import { Accordion, AccordionDetails, AccordionSummary } from "@components/Accordion";
import FetchStatusCheck from "@components/FetchStatusCheck";
import PageHeading from "@components/PageHeading";
import { StyledPaper } from "@components/StyledComponents";
import Alert from "@mui/material/Alert";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import CircularProgress from "@mui/material/CircularProgress";
import Divider from "@mui/material/Divider";
import Grid from "@mui/material/Grid";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import type {
  KeyId,
  SigningRequestEffectiveStatus,
  SigningRequester,
  SigningRequestInfo,
} from "@tari-project/ootle-ts-bindings";
import { useEffect, useState } from "react";
import Inputs from "../Transactions/Inputs";
import Instructions from "../Transactions/Instructions";
import { describeFee, describeGovernanceCall, fingerprint, networkName } from "./describe";

function formatKeyId(keyId: KeyId): string {
  if ("Derived" in keyId) {
    return `${keyId.Derived.key_branch.replace(/_/g, " ")} key #${keyId.Derived.index}`;
  }
  return `imported key #${keyId.Imported.local_key_id}`;
}

function requesterLabel(requester: SigningRequester): string {
  if (requester === "WalletSession") {
    return "This wallet's session";
  }
  if (requester === "LocalClient") {
    return "A local client";
  }
  if (requester === "ConnectedApp") {
    return "A connected app";
  }
  return `API key "${requester.ApiKey.name}"`;
}

function statusColor(status: SigningRequestEffectiveStatus): "default" | "warning" | "success" | "error" {
  switch (status) {
    case "Pending":
      return "warning";
    case "Signed":
      return "success";
    case "Rejected":
      return "error";
    default:
      return "default";
  }
}

/// The current time in seconds, updated every second.
function useNow(): number {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(t);
  }, []);
  return now;
}

function Countdown({ expiresAt, now }: { expiresAt: bigint; now: number }) {
  const remaining = Math.max(0, Math.floor(Number(expiresAt) - now));
  if (remaining === 0) {
    return <Chip label="expired" size="small" variant="outlined" />;
  }
  const hours = Math.floor(remaining / 3600);
  const mins = Math.floor((remaining % 3600) / 60);
  const secs = remaining % 60;
  const label = hours > 0 ? `${hours}h ${mins}m` : `${mins}:${secs.toString().padStart(2, "0")}`;
  return <Chip label={`expires in ${label}`} size="small" variant="outlined" />;
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <Grid size={{ xs: 12, md: 6 }}>
      <Typography variant="body2" sx={{ opacity: 0.7 }}>
        {label}
      </Typography>
      <Typography variant="body2" component="div" sx={{ wordBreak: "break-all" }}>
        {children}
      </Typography>
    </Grid>
  );
}

/// What the signature authorises, in words, for the instructions this page
/// knows how to read.
function Summary({ request }: { request: SigningRequestInfo }) {
  const v1 = request.transaction.V1;
  const instructions = v1?.instructions ?? [];
  const governance = instructions.map(describeGovernanceCall);
  const described = governance.filter((d) => d !== null);
  const undescribed = governance.length - described.length;

  if (described.length === 0) {
    return null;
  }
  return (
    <Alert severity="warning" icon={false} sx={{ mb: 2 }}>
      {described.map((d, i) => (
        <div key={i}>
          <Typography variant="h6" sx={{ fontWeight: 600 }}>
            {d.headline}
          </Typography>
          {d.details.map((line, j) => (
            <Typography key={j} variant="body2" sx={{ fontFamily: "monospace", wordBreak: "break-all" }}>
              {line}
            </Typography>
          ))}
        </div>
      ))}
      {undescribed > 0 && (
        <Typography variant="body2" sx={{ mt: 1 }}>
          Plus {undescribed} other instruction(s); review them below.
        </Typography>
      )}
    </Alert>
  );
}

function RequestCard({ request }: { request: SigningRequestInfo }) {
  const approve = useApproveSigningRequest();
  const reject = useRejectSigningRequest();
  const [expanded, setExpanded] = useState<string | null>(null);
  const now = useNow();
  const isActionable = request.status === "Pending";
  // The server refuses a decision once the window closes; the next poll then
  // shows the request as expired.
  const isExpired = Number(request.expires_at) <= now;
  const busy = approve.isPending || reject.isPending || isExpired;
  const error = isActionable ? (approve.error ?? reject.error) : null;
  const params = { request_id: request.request_id };

  const v1 = request.transaction.V1;
  const instructions = v1?.instructions ?? [];
  const feeInstructions = v1?.fee_instructions ?? [];
  const inputs = v1?.inputs ?? [];
  const fees = feeInstructions.map(describeFee).filter((f) => f !== null);
  const isMainnet = v1?.network === 0x00;

  const togglePanel = (panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) =>
    setExpanded(isExpanded ? panel : null);

  return (
    <StyledPaper sx={{ mb: 2 }}>
      <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ mb: 1 }} flexWrap="wrap" gap={1}>
        <Typography variant="h5">{requesterLabel(request.requester)} requests a signature</Typography>
        <Stack direction="row" spacing={1} alignItems="center">
          {request.requester === "ConnectedApp" && (
            <Chip label="WebRTC" size="small" color="warning" variant="outlined" />
          )}
          {v1 && (
            <Chip
              label={networkName(v1.network)}
              size="small"
              color={isMainnet ? "error" : "default"}
              variant={isMainnet ? "filled" : "outlined"}
            />
          )}
          {isActionable && <Countdown expiresAt={request.expires_at} now={now} />}
          <Chip label={request.status} size="small" color={statusColor(request.status)} />
        </Stack>
      </Stack>

      <Summary request={request} />

      <Stack direction="row" spacing={2} alignItems="baseline" sx={{ mb: 2 }} flexWrap="wrap">
        <Typography variant="body2" sx={{ opacity: 0.7 }}>
          Fingerprint
        </Typography>
        <Typography variant="h5" sx={{ fontFamily: "monospace", letterSpacing: 1 }}>
          {fingerprint(request.message_hash)}
        </Typography>
        <Typography variant="body2" sx={{ opacity: 0.7 }}>
          Confirm it with the proposer over a separate channel before approving.
        </Typography>
      </Stack>

      {request.memo && (
        <Alert severity="info" icon={false} sx={{ mb: 2 }}>
          <Typography variant="body2" sx={{ opacity: 0.7 }}>
            Memo from the requester (unverified)
          </Typography>
          <Typography variant="body1" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
            {request.memo}
          </Typography>
        </Alert>
      )}

      <Grid container spacing={2} sx={{ mb: 2 }}>
        <Field label="Signs with">
          {formatKeyId(request.key_id)}
          <br />
          <span style={{ fontFamily: "monospace" }}>{request.signer_public_key}</span>
        </Field>
        <Field label="Sealed by">
          <span style={{ fontFamily: "monospace" }}>{request.seal_public_key}</span>
          {v1?.main_signer === "SealSigner" && (
            <Chip label="seal also authorizes" size="small" color="warning" variant="outlined" sx={{ ml: 1 }} />
          )}
        </Field>
        <Field label="Main signer">
          {v1?.main_signer === "SealSigner" && "the sealer"}
          {(v1?.main_signer === "None" || v1 === undefined) && "none"}
          {v1 && typeof v1.main_signer === "object" && (
            <>
              <span style={{ fontFamily: "monospace" }}>{v1.main_signer.Authorizer}</span>
              {v1.main_signer.Authorizer === request.signer_public_key && (
                <Chip label="this key" size="small" color="warning" variant="outlined" sx={{ ml: 1 }} />
              )}
            </>
          )}
        </Field>
        <Field label="Valid epochs">
          {v1?.min_epoch != null ? `${v1.min_epoch} to ${v1.max_epoch}` : `until ${v1?.max_epoch}`}
        </Field>
        <Field label="Max fee">
          {fees.length > 0 ? fees.map((f) => <div key={f}>{f}</div>) : `${feeInstructions.length} fee instruction(s)`}
        </Field>
      </Grid>

      <Accordion expanded={expanded === "instructions"} onChange={togglePanel("instructions")}>
        <AccordionSummary aria-controls="signing-request-instructions-content">
          <Typography variant="h6">Instructions ({instructions.length})</Typography>
        </AccordionSummary>
        <AccordionDetails>
          <Instructions data={instructions} />
        </AccordionDetails>
      </Accordion>
      <Accordion expanded={expanded === "fees"} onChange={togglePanel("fees")}>
        <AccordionSummary aria-controls="signing-request-fee-instructions-content">
          <Typography variant="h6">Fee Instructions ({feeInstructions.length})</Typography>
        </AccordionSummary>
        <AccordionDetails>
          <Instructions data={feeInstructions} />
        </AccordionDetails>
      </Accordion>
      <Accordion expanded={expanded === "inputs"} onChange={togglePanel("inputs")}>
        <AccordionSummary aria-controls="signing-request-inputs-content">
          <Typography variant="h6">Inputs ({inputs.length})</Typography>
        </AccordionSummary>
        <AccordionDetails>
          <Inputs data={inputs} />
        </AccordionDetails>
      </Accordion>

      {error && (
        <Alert severity="error" sx={{ mt: 1 }}>
          {error.message}
        </Alert>
      )}

      {isActionable && (
        <>
          <Divider sx={{ my: 2 }} />
          <Stack direction="row" spacing={1} justifyContent="flex-end">
            <Button
              variant="outlined"
              color="error"
              disabled={busy}
              startIcon={reject.isPending ? <CircularProgress size={16} color="inherit" /> : undefined}
              onClick={() => reject.mutate(params)}
            >
              Reject
            </Button>
            <Button
              variant="contained"
              disabled={busy}
              startIcon={approve.isPending ? <CircularProgress size={16} color="inherit" /> : undefined}
              onClick={() => approve.mutate(params)}
            >
              Approve &amp; Sign
            </Button>
          </Stack>
        </>
      )}
    </StyledPaper>
  );
}

export default function SigningRequests() {
  const { data, isFetching, isError, error } = useListPendingSigningRequests();
  const { data: all } = useListAllSigningRequests();

  const pending = data?.requests ?? [];
  const rest = (all?.requests ?? []).filter((r) => r.status !== "Pending");

  return (
    <Grid container spacing={5}>
      <Grid size={12}>
        <PageHeading>Signing Requests</PageHeading>
      </Grid>
      <Grid size={12}>
        <FetchStatusCheck isLoading={isFetching && !data} isError={isError} errorMessage={error?.message ?? ""}>
          {pending.length === 0 && (
            <StyledPaper>
              <Typography variant="body1" sx={{ opacity: 0.7 }}>
                No pending signing requests.
              </Typography>
            </StyledPaper>
          )}
          {pending.map((r) => (
            <RequestCard key={r.request_id} request={r} />
          ))}
          {rest.length > 0 && (
            <>
              <Typography variant="h5" sx={{ mt: 4, mb: 2 }}>
                History
              </Typography>
              {rest.map((r) => (
                <RequestCard key={r.request_id} request={r} />
              ))}
            </>
          )}
        </FetchStatusCheck>
      </Grid>
    </Grid>
  );
}
