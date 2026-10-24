//  Copyright 2026 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import {
  useApproveTransactionRequest,
  useListTransactionRequests,
  useRejectTransactionRequest,
  useSubmitTransactionRequest,
} from "@api/hooks/useTransactionRequests";
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
  EffectiveStatus,
  KeyId,
  TransactionRequestInfo,
  TransactionRequestMainSigner,
} from "@tari-project/ootle-ts-bindings";
import { useEffect, useState } from "react";
import Inputs from "../Transactions/Inputs";
import Instructions from "../Transactions/Instructions";

function formatKeyId(keyId: KeyId): string {
  if ("Derived" in keyId) {
    return `${keyId.Derived.key_branch.replace(/_/g, " ")} key #${keyId.Derived.index}`;
  }
  return `imported key #${keyId.Imported.local_key_id}`;
}

function describeMainSigner(mainSigner: TransactionRequestMainSigner): {
  label: string;
  publicKey?: string;
  color: "default" | "warning" | "error";
} {
  if (mainSigner === "SealSigner") {
    return { label: "the seal signer", color: "warning" };
  }
  if (mainSigner === "None") {
    return { label: "none", color: "default" };
  }
  if ("WalletKey" in mainSigner) {
    return { label: "a key of this wallet", publicKey: mainSigner.WalletKey.public_key, color: "warning" };
  }
  return { label: "a key outside this wallet", publicKey: mainSigner.External.public_key, color: "error" };
}

function statusColor(status: EffectiveStatus): "default" | "warning" | "success" | "error" | "info" {
  switch (status) {
    case "Pending":
      return "warning";
    case "Approved":
      return "info";
    case "Submitting":
      return "info";
    case "Submitted":
      return "success";
    case "Rejected":
      return "error";
    default:
      return "default";
  }
}

function Countdown({ expiresAt }: { expiresAt: bigint }) {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(t);
  }, []);

  const remaining = Math.max(0, Math.floor(Number(expiresAt) - now));
  const mins = Math.floor(remaining / 60);
  const secs = remaining % 60;
  return <Chip label={`expires in ${mins}:${secs.toString().padStart(2, "0")}`} size="small" variant="outlined" />;
}

function ValueSummary({ request }: { request: TransactionRequestInfo }) {
  // A stealth transfer's instructions are commitments and range proofs, so a
  // person cannot read an amount out of them. The wallet owns the masks and is
  // the only party that can say what leaves. Without this the approver is
  // authorising an opaque blob.
  if (!request.value_summary) {
    return null;
  }
  const { amount_leaving, inputs_total, change_total, resource_address } = request.value_summary;
  return (
    <Alert severity="warning" icon={false} sx={{ mb: 2 }}>
      <Typography variant="h6" sx={{ fontWeight: 600 }}>
        {amount_leaving.toLocaleString()} µT leaves this wallet
      </Typography>
      <Typography variant="body2" sx={{ opacity: 0.85 }}>
        {inputs_total.toLocaleString()} µT spent, {change_total.toLocaleString()} µT returned as change
      </Typography>
      <Typography variant="body2" sx={{ opacity: 0.7, wordBreak: "break-all" }}>
        {resource_address}
      </Typography>
    </Alert>
  );
}

function RequestCard({ request }: { request: TransactionRequestInfo }) {
  const approve = useApproveTransactionRequest();
  const reject = useRejectTransactionRequest();
  const submit = useSubmitTransactionRequest();
  const [expanded, setExpanded] = useState<string | null>(null);
  const isActionable = request.status === "Pending";
  // Approve and submit are separate permissions and separate RPCs, but this UI
  // holds both, so approving chains straight into submitting. If the submit
  // half fails (or another principal beat it), the request stays Approved and
  // this standalone button finishes the job.
  const isSubmittable = request.status === "Approved";
  const busy = approve.isPending || reject.isPending || submit.isPending;
  // A mutation error is only worth showing while the decision is still open.
  // Two principals can race (an approve here vs. a tool's submit); the loser's
  // error describes a state this card no longer displays once the list
  // refetches, so it would read as the action having failed outright.
  const error = isActionable || isSubmittable ? (approve.error ?? reject.error ?? submit.error) : null;

  const params = { request_id: request.request_id };

  const v1 = request.transaction.V1;
  const instructions = v1?.instructions ?? [];
  const feeInstructions = v1?.fee_instructions ?? [];
  const inputs = v1?.inputs ?? [];
  const isSealSignerAuthorized = v1?.main_signer === "SealSigner";
  const mainSigner = describeMainSigner(request.main_signer);

  const togglePanel = (panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) =>
    setExpanded(isExpanded ? panel : null);

  return (
    <StyledPaper sx={{ mb: 2 }}>
      <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ mb: 1 }} flexWrap="wrap" gap={1}>
        <Typography variant="h5">
          {request.requested_by === "local-cookie"
            ? "A local client"
            : request.requested_by
              ? `"${request.requested_by}"`
              : "A wallet session"}{" "}
          requests approval
        </Typography>
        <Stack direction="row" spacing={1} alignItems="center">
          {isActionable && <Countdown expiresAt={request.expires_at} />}
          <Chip label={request.status} size="small" color={statusColor(request.status)} />
        </Stack>
      </Stack>

      <ValueSummary request={request} />

      <Grid container spacing={2} sx={{ mb: 1 }}>
        <Grid size={12}>
          <Typography variant="body2" sx={{ opacity: 0.7 }}>
            Seal signer
          </Typography>
          <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap">
            <Typography variant="body2">{formatKeyId(request.seal_signer)}</Typography>
            {/* Not an implementation detail: when set, the seal signature also
                authorises the transaction, lending the sealer's account
                authority to these instructions. */}
            <Chip
              label={isSealSignerAuthorized ? "authorized" : "not authorized"}
              size="small"
              color={isSealSignerAuthorized ? "warning" : "default"}
              variant="outlined"
            />
          </Stack>
        </Grid>
        <Grid size={12}>
          <Typography variant="body2" sx={{ opacity: 0.7 }}>
            Main signer
          </Typography>
          {/* Whatever the transaction creates as "owned by signer" belongs to
              this key, and templates that ask for the transaction's signer
              see it. */}
          <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap">
            <Chip label={mainSigner.label} size="small" color={mainSigner.color} variant="outlined" />
            {mainSigner.publicKey && (
              <Typography variant="body2" sx={{ fontFamily: "monospace", wordBreak: "break-all" }}>
                {mainSigner.publicKey}
              </Typography>
            )}
          </Stack>
        </Grid>
        {request.external_signers.length > 0 && (
          <Grid size={12}>
            <Typography variant="body2" sx={{ opacity: 0.7 }}>
              Also signed by ({request.external_signers.length} key(s) outside this wallet)
            </Typography>
            {request.external_signers.map((publicKey) => (
              <Typography key={publicKey} variant="body2" sx={{ fontFamily: "monospace", wordBreak: "break-all" }}>
                {publicKey}
              </Typography>
            ))}
          </Grid>
        )}
      </Grid>

      <Accordion expanded={expanded === "instructions"} onChange={togglePanel("instructions")}>
        <AccordionSummary aria-controls="request-instructions-content">
          <Typography variant="h6">Instructions ({instructions.length})</Typography>
        </AccordionSummary>
        <AccordionDetails>
          <Instructions data={instructions} />
        </AccordionDetails>
      </Accordion>
      <Accordion expanded={expanded === "fees"} onChange={togglePanel("fees")}>
        <AccordionSummary aria-controls="request-fee-instructions-content">
          <Typography variant="h6">Fee Instructions ({feeInstructions.length})</Typography>
        </AccordionSummary>
        <AccordionDetails>
          <Instructions data={feeInstructions} />
        </AccordionDetails>
      </Accordion>
      <Accordion expanded={expanded === "inputs"} onChange={togglePanel("inputs")}>
        <AccordionSummary aria-controls="request-inputs-content">
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

      {(isActionable || isSubmittable) && (
        <>
          <Divider sx={{ my: 2 }} />
          <Stack direction="row" spacing={1} justifyContent="flex-end">
            {isActionable && (
              <>
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
                  startIcon={
                    approve.isPending || submit.isPending ? <CircularProgress size={16} color="inherit" /> : undefined
                  }
                  onClick={() => approve.mutate(params, { onSuccess: () => submit.mutate(params) })}
                >
                  Approve &amp; Submit
                </Button>
              </>
            )}
            {isSubmittable && (
              <Button
                variant="contained"
                disabled={busy}
                startIcon={submit.isPending ? <CircularProgress size={16} color="inherit" /> : undefined}
                onClick={() => submit.mutate({ request_id: request.request_id })}
              >
                Submit
              </Button>
            )}
          </Stack>
        </>
      )}
    </StyledPaper>
  );
}

export default function TransactionRequests() {
  const { data, isFetching, isError, error } = useListTransactionRequests();

  const requests = data?.requests ?? [];
  const pending = requests.filter((r) => r.status === "Pending");
  const rest = requests.filter((r) => r.status !== "Pending");

  return (
    <Grid container spacing={5}>
      <Grid size={12}>
        <PageHeading>Transaction Requests</PageHeading>
      </Grid>
      <Grid size={12}>
        <FetchStatusCheck isLoading={isFetching && !data} isError={isError} errorMessage={error?.message ?? ""}>
          {pending.length === 0 && (
            <StyledPaper>
              <Typography variant="body1" sx={{ opacity: 0.7 }}>
                No pending requests.
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
