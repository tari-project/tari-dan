//  Copyright 2024. The Tari Project
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

import ClearIcon from "@mui/icons-material/Clear";
import GppMaybeIcon from "@mui/icons-material/GppMaybe";
import SearchIcon from "@mui/icons-material/Search";
import VerifiedIcon from "@mui/icons-material/Verified";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  IconButton,
  InputAdornment,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableRow,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import Grid from "@mui/material/Grid";
import { convertCborValue, type Component, type SubstateValue } from "@tari-project/ootle-ts-bindings";
import React, { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import CopyToClipboard from "../../Components/CopyToClipboard";
import PageHeading from "../../Components/PageHeading";
import { CodeBlock, StyledPaper } from "../../Components/StyledComponents";
import { useGetSubstate } from "../../api/hooks/useSubstates";
import { decodeSubstateValue, encodedByteLength } from "../../utils/encoded";
import { renderJson } from "../../utils/helpers";

const SUBSTATE_PREFIXES = [
  "component",
  "resource",
  "vault",
  "nft",
  "tombstone",
  "txreceipt",
  "template",
  "vnfp",
  "utxo",
];

function validateSubstateId(id: string): string | null {
  if (!id) return null;
  const underscoreIndex = id.indexOf("_");
  if (underscoreIndex === -1) {
    return `Invalid format. Expected a prefix followed by "_" (e.g. component_..., resource_...). Valid prefixes: ${SUBSTATE_PREFIXES.join(", ")}`;
  }
  const prefix = id.substring(0, underscoreIndex);
  if (!SUBSTATE_PREFIXES.includes(prefix)) {
    return `Unknown prefix "${prefix}". Valid prefixes: ${SUBSTATE_PREFIXES.join(", ")}`;
  }
  return null;
}

function getSubstateType(substate: SubstateValue): string {
  return Object.keys(substate)[0];
}

function getSubstateData(substate: SubstateValue): unknown {
  return Object.values(substate)[0];
}

function VerifiedChip({ verified }: { verified: boolean }) {
  const title = verified
    ? "This substate's value was verified by the indexer against the shard group committee via a Merkle proof."
    : "Served without a proof — the value is trusted from a single validator and was not cryptographically verified.";
  return (
    <Tooltip title={title}>
      <Chip
        icon={verified ? <VerifiedIcon /> : <GppMaybeIcon />}
        label={verified ? "Verified" : "Unverified"}
        color={verified ? "success" : "warning"}
        variant={verified ? "filled" : "outlined"}
        size="small"
      />
    </Tooltip>
  );
}

function EncodedSubstateDetails({
  encoded,
  version,
  verified,
}: {
  encoded: string;
  version: number;
  verified: boolean;
}) {
  const substate = decodeSubstateValue(encoded);
  if (substate) {
    return <SubstateDetails substate={substate} version={version} verified={verified} />;
  }
  return (
    <Stack spacing={2}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
        <Chip label="CBOR" color="primary" />
        <Typography variant="body2" color="text.secondary">
          Version {version}, {encodedByteLength(encoded)} bytes
        </Typography>
        <Box sx={{ flexGrow: 1 }} />
        <VerifiedChip verified={verified} />
        <CopyToClipboard copy={encoded} />
      </Box>
      <Divider />
      <CodeBlock>{encoded}</CodeBlock>
    </Stack>
  );
}

function SubstateDetails({
  substate,
  version,
  verified,
}: {
  substate: SubstateValue;
  version: number;
  verified: boolean;
}) {
  const type = getSubstateType(substate);
  const data = getSubstateData(substate);

  return (
    <Stack spacing={2}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
        <Chip label={type} color="primary" />
        <Typography variant="body2" color="text.secondary">
          Version {version}
        </Typography>
        <Box sx={{ flexGrow: 1 }} />
        <VerifiedChip verified={verified} />
      </Box>
      <Divider />
      {renderSubstateByType(type, data)}
    </Stack>
  );
}

function renderSubstateByType(type: string, data: unknown): React.ReactNode {
  switch (type) {
    case "Component":
      return <ComponentView data={data as Component} />;
    case "Resource":
      return <ResourceView data={data as any} />;
    case "Vault":
      return <VaultView data={data as any} />;
    case "Template":
      return <TemplateView data={data as any} />;
    case "TransactionReceipt":
      return <TransactionReceiptView data={data as any} />;
    case "ValidatorFeePool":
      return <ValidatorFeePoolView data={data as any} />;
    case "NonFungible":
      return <NonFungibleView data={data as any} />;
    case "Utxo":
      return <UtxoView data={data as any} />;
    case "ClaimedOutputTombstone":
      return <ClaimedOutputTombstoneView data={data as any} />;
    default:
      return <FallbackView data={data} />;
  }
}

function FieldTable({ fields }: { fields: Array<{ label: string; value: React.ReactNode }> }) {
  return (
    <TableContainer>
      <Table size="small">
        <TableBody>
          {fields.map((field) => (
            <TableRow key={field.label}>
              <TableCell sx={{ fontWeight: "bold", width: "200px", whiteSpace: "nowrap" }}>{field.label}</TableCell>
              <TableCell
                sx={{ fontFamily: "'Courier New', Courier, monospace", fontSize: "14px", wordBreak: "break-all" }}
              >
                {field.value}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}

function ComponentView({ data }: { data: Component }) {
  const decodedState = data.body?.state ? convertCborValue(data.body.state) : data.body;

  return (
    <Stack spacing={2}>
      <FieldTable
        fields={[
          { label: "Template Address", value: data.header.template_address },
          { label: "Entity ID", value: String(data.header.entity_id) },
          { label: "Owner Rule", value: <CodeBlock>{renderJson(data.header.owner_rule)}</CodeBlock> },
          { label: "Access Rules", value: <CodeBlock>{renderJson(data.header.access_rules)}</CodeBlock> },
        ]}
      />
      <Typography variant="subtitle2">State</Typography>
      <CodeBlock>{renderJson(decodedState)}</CodeBlock>
    </Stack>
  );
}

function ResourceView({ data }: { data: any }) {
  return (
    <FieldTable
      fields={[
        {
          label: "Resource Type",
          value:
            typeof data.resource_type === "object" ? JSON.stringify(data.resource_type) : String(data.resource_type),
        },
        { label: "Divisibility", value: String(data.divisibility) },
        { label: "Total Supply", value: data.total_supply !== null ? String(data.total_supply) : "Tracking disabled" },
        { label: "Owner Rule", value: <CodeBlock>{renderJson(data.owner_rule)}</CodeBlock> },
        { label: "Access Rules", value: <CodeBlock>{renderJson(data.access_rules)}</CodeBlock> },
        { label: "Metadata", value: <CodeBlock>{renderJson(convertCborValue(data.metadata))}</CodeBlock> },
        ...(data.view_key ? [{ label: "View Key", value: String(data.view_key) }] : []),
        ...(data.auth_hook ? [{ label: "Auth Hook", value: <CodeBlock>{renderJson(data.auth_hook)}</CodeBlock> }] : []),
      ]}
    />
  );
}

function VaultView({ data }: { data: any }) {
  return (
    <FieldTable
      fields={[
        { label: "Resource Container", value: <CodeBlock>{renderJson(data.resource_container)}</CodeBlock> },
        { label: "Freeze Flags", value: <CodeBlock>{renderJson(data.freeze_flags)}</CodeBlock> },
      ]}
    />
  );
}

function TemplateView({ data }: { data: any }) {
  return (
    <FieldTable
      fields={[
        { label: "Template Name", value: data.template_name || "N/A" },
        { label: "Author", value: data.author },
        { label: "Published Epoch", value: String(data.at_epoch) },
        { label: "Metadata Hash", value: data.metadata_hash || "None" },
        { label: "Binary Size", value: data.binary ? `${data.binary.length} bytes` : "N/A" },
      ]}
    />
  );
}

function TransactionReceiptView({ data }: { data: any }) {
  return (
    <Stack spacing={2}>
      <FieldTable
        fields={[
          { label: "Outcome", value: <CodeBlock>{renderJson(data.outcome)}</CodeBlock> },
          { label: "Epoch", value: String(data.epoch) },
          { label: "Fee Receipt", value: <CodeBlock>{renderJson(data.fee_receipt)}</CodeBlock> },
        ]}
      />
      {data.events?.length > 0 && (
        <>
          <Typography variant="subtitle2">Events ({data.events.length})</Typography>
          <CodeBlock>{renderJson(data.events)}</CodeBlock>
        </>
      )}
      <Typography variant="subtitle2">Diff Summary</Typography>
      <CodeBlock>{renderJson(data.diff_summary)}</CodeBlock>
    </Stack>
  );
}

function ValidatorFeePoolView({ data }: { data: any }) {
  return (
    <FieldTable
      fields={[
        { label: "Claim Public Key", value: String(data.claim_public_key) },
        { label: "Amount", value: String(data.amount) },
      ]}
    />
  );
}

function NonFungibleView({ data }: { data: any }) {
  if (data === null) {
    return <Typography color="text.secondary">Empty non-fungible container</Typography>;
  }
  return <CodeBlock>{renderJson(data)}</CodeBlock>;
}

function UtxoView({ data }: { data: any }) {
  return (
    <FieldTable
      fields={[
        { label: "Frozen", value: data.is_frozen ? "Yes" : "No" },
        { label: "Output", value: data.output ? <CodeBlock>{renderJson(data.output)}</CodeBlock> : "None" },
      ]}
    />
  );
}

function ClaimedOutputTombstoneView({ data }: { data: any }) {
  return <FieldTable fields={[{ label: "Value", value: String(data.value) }]} />;
}

function FallbackView({ data }: { data: unknown }) {
  return <CodeBlock>{renderJson(data)}</CodeBlock>;
}

function SubstatesLayout() {
  const [searchParams, setSearchParams] = useSearchParams();
  const initialAddress = searchParams.get("address") || "";
  const [addressInput, setAddressInput] = useState(initialAddress);
  const [fetchAddress, setFetchAddress] = useState<string | null>(initialAddress || null);
  const [validationError, setValidationError] = useState<string | null>(null);

  const { data, isLoading, isError, error } = useGetSubstate({
    address: fetchAddress,
    enabled: !!fetchAddress,
  });

  useEffect(() => {
    const addr = searchParams.get("address") || "";
    if (addr && addr !== fetchAddress) {
      setAddressInput(addr);
      setValidationError(null);
      setFetchAddress(addr);
    }
  }, [searchParams]);

  const handleFetch = useCallback(() => {
    const trimmed = addressInput.trim();
    if (!trimmed) return;
    const error = validateSubstateId(trimmed);
    if (error) {
      setValidationError(error);
      return;
    }
    setValidationError(null);
    setFetchAddress(trimmed);
    setSearchParams({ address: trimmed });
  }, [addressInput, setSearchParams]);

  const handleClear = useCallback(() => {
    setAddressInput("");
    setFetchAddress(null);
    setValidationError(null);
    setSearchParams({});
  }, [setSearchParams]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleFetch();
      }
    },
    [handleFetch],
  );

  return (
    <>
      <Grid size={12}>
        <PageHeading>Substates</PageHeading>
      </Grid>
      <Grid size={12}>
        <StyledPaper>
          <Stack spacing={3}>
            <Typography variant="body2" color="text.secondary">
              Look up any substate from the network by its address (e.g. component_xxx, resource_xxx, vault_xxx).
            </Typography>
            <Box sx={{ display: "flex", gap: 2 }}>
              <TextField
                fullWidth
                label="Substate Address"
                placeholder="component_0000000000000000000000000000000000000000000000000000..."
                value={addressInput}
                onChange={(e) => {
                  setAddressInput(e.target.value);
                  if (validationError) setValidationError(null);
                }}
                onKeyDown={handleKeyDown}
                size="medium"
                error={!!validationError}
                helperText={validationError}
                sx={{ fontFamily: "'Courier New', Courier, monospace" }}
                slotProps={{
                  input: {
                    endAdornment: addressInput ? (
                      <InputAdornment position="end">
                        <IconButton onClick={handleClear} edge="end" size="small">
                          <ClearIcon fontSize="small" />
                        </IconButton>
                      </InputAdornment>
                    ) : null,
                  },
                }}
              />
              <Button
                variant="contained"
                onClick={handleFetch}
                disabled={!addressInput.trim() || isLoading}
                startIcon={isLoading ? <CircularProgress size={20} /> : <SearchIcon />}
                sx={{ minWidth: "120px" }}
              >
                {isLoading ? "Fetching" : "Fetch"}
              </Button>
            </Box>

            {fetchAddress && (
              <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Typography
                  variant="body2"
                  color="text.secondary"
                  sx={{ fontFamily: "'Courier New', Courier, monospace", wordBreak: "break-all" }}
                >
                  {fetchAddress}
                </Typography>
                <CopyToClipboard copy={fetchAddress} />
              </Box>
            )}

            {isError && (
              <Alert severity="error">
                {(error as any)?.message || "Failed to fetch substate. Check the address and try again."}
              </Alert>
            )}

            {data && "up" in data.state && (
              <>
                <Divider />
                <EncodedSubstateDetails
                  encoded={data.state.up.substate}
                  version={data.version}
                  verified={data.verified}
                />
              </>
            )}
          </Stack>
        </StyledPaper>
      </Grid>
    </>
  );
}

export default SubstatesLayout;
