// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import {
  useWebauthnListPasskeys,
  useWebauthnRemovePasskey,
  useWebauthnRenamePasskey,
  WEBAUTHN_PASSKEYS_QUERY_KEY,
} from "@api/hooks/useWebauthn";
import queryClient from "@api/queryClient";
import { performPasskeyRegistration } from "@components/auth/web_authn/components/Registration";
import FetchStatusCheck from "@components/FetchStatusCheck";
import { DataTableCell } from "@components/StyledComponents";
import DeleteOutlineIcon from "@mui/icons-material/DeleteOutline";
import EditOutlinedIcon from "@mui/icons-material/EditOutlined";
import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Fade,
  IconButton,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import type { WebauthnPasskeyInfo } from "@tari-project/ootle-ts-bindings";
import { FormEvent, useState } from "react";

function formatTimestamp(unixSeconds: number | null): string {
  if (unixSeconds === null || unixSeconds === undefined) {
    return "Never";
  }
  const date = new Date(Number(unixSeconds) * 1000);
  if (Number.isNaN(date.getTime())) {
    return "Unknown";
  }
  return `${date.toISOString().slice(0, 10)} ${date.toISOString().slice(11, 16)} UTC`;
}

export default function Passkeys() {
  const { data, isLoading, error, isError } = useWebauthnListPasskeys();
  const renameMutation = useWebauthnRenamePasskey();
  const removeMutation = useWebauthnRemovePasskey();

  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [adding, setAdding] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const [renameTarget, setRenameTarget] = useState<WebauthnPasskeyInfo | null>(null);
  const [renameValue, setRenameValue] = useState("");

  const [removeTarget, setRemoveTarget] = useState<WebauthnPasskeyInfo | null>(null);

  const passkeys = data?.passkeys ?? [];
  const isLastPasskey = passkeys.length <= 1;

  const handleOpenAdd = () => {
    setActionError(null);
    setNewLabel("");
    setAddDialogOpen(true);
  };

  const handleAddPasskey = async (e: FormEvent) => {
    e.preventDefault();
    setAdding(true);
    setActionError(null);
    try {
      await performPasskeyRegistration({
        label: newLabel.trim() || undefined,
        updateClientToken: false,
      });
      await queryClient.invalidateQueries({ queryKey: WEBAUTHN_PASSKEYS_QUERY_KEY });
      setAddDialogOpen(false);
      setNewLabel("");
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setAdding(false);
    }
  };

  const handleOpenRename = (passkey: WebauthnPasskeyInfo) => {
    setActionError(null);
    setRenameTarget(passkey);
    setRenameValue(passkey.label);
  };

  const handleRenameSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!renameTarget) return;
    const trimmed = renameValue.trim();
    if (!trimmed) {
      setActionError("Passkey label must not be empty");
      return;
    }
    setActionError(null);
    try {
      await renameMutation.mutateAsync({ id: renameTarget.id, label: trimmed });
      setRenameTarget(null);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleConfirmRemove = async () => {
    if (!removeTarget) return;
    setActionError(null);
    try {
      await removeMutation.mutateAsync(removeTarget.id);
      setRemoveTarget(null);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
      setRemoveTarget(null);
    }
  };

  return (
    <FetchStatusCheck
      isLoading={isLoading}
      isError={isError}
      errorMessage={error?.message || "Error fetching passkeys"}
    >
      <Fade in={!isLoading && !isError}>
        <Box>
          <Stack
            direction={{ xs: "column", sm: "row" }}
            justifyContent="space-between"
            alignItems={{ xs: "stretch", sm: "center" }}
            spacing={2}
            sx={{ mb: 2 }}
          >
            <Typography variant="body2" color="text.secondary">
              Any registered passkey can sign in to this wallet. Register a backup passkey on a second device or
              hardware security key to prevent lockout.
            </Typography>
            <Button variant="contained" onClick={handleOpenAdd}>
              Add Passkey
            </Button>
          </Stack>

          {passkeys.length === 1 && (
            <Alert severity="info" sx={{ mb: 2 }}>
              Only one passkey is registered. Consider adding a backup passkey so you do not lose access to your wallet
              if this authenticator is lost.
            </Alert>
          )}

          {actionError && (
            <Alert severity="error" sx={{ mb: 2 }} onClose={() => setActionError(null)}>
              {actionError}
            </Alert>
          )}

          <TableContainer>
            <Table>
              <TableHead>
                <TableRow>
                  <TableCell>Label</TableCell>
                  <TableCell>Created</TableCell>
                  <TableCell>Last Used</TableCell>
                  <TableCell align="right">Sign Counter</TableCell>
                  <TableCell width="140" align="center">
                    Actions
                  </TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {passkeys.map((passkey) => (
                  <TableRow key={passkey.id}>
                    <DataTableCell>{passkey.label}</DataTableCell>
                    <DataTableCell>{formatTimestamp(passkey.created_at)}</DataTableCell>
                    <DataTableCell>{formatTimestamp(passkey.last_used_at)}</DataTableCell>
                    <DataTableCell sx={{ textAlign: "right" }}>{passkey.counter}</DataTableCell>
                    <DataTableCell sx={{ textAlign: "center" }}>
                      <Tooltip title="Rename passkey">
                        <IconButton
                          aria-label={`Rename ${passkey.label}`}
                          size="small"
                          onClick={() => handleOpenRename(passkey)}
                        >
                          <EditOutlinedIcon fontSize="small" />
                        </IconButton>
                      </Tooltip>
                      <Tooltip
                        title={
                          isLastPasskey
                            ? "Cannot remove the last passkey from the wallet"
                            : "Remove passkey"
                        }
                      >
                        <span>
                          <IconButton
                            aria-label={`Remove ${passkey.label}`}
                            size="small"
                            color="error"
                            disabled={isLastPasskey}
                            onClick={() => setRemoveTarget(passkey)}
                          >
                            <DeleteOutlineIcon fontSize="small" />
                          </IconButton>
                        </span>
                      </Tooltip>
                    </DataTableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </TableContainer>

          <Dialog open={addDialogOpen} onClose={() => !adding && setAddDialogOpen(false)} fullWidth maxWidth="xs">
            <form onSubmit={handleAddPasskey}>
              <DialogTitle>Add Passkey</DialogTitle>
              <DialogContent>
                <DialogContentText sx={{ mb: 2 }}>
                  Enter a label to identify this passkey, then follow your browser or authenticator prompt.
                </DialogContentText>
                <TextField
                  autoFocus
                  fullWidth
                  label="Passkey Label"
                  placeholder="e.g. YubiKey 5C / Laptop Touch ID"
                  value={newLabel}
                  onChange={(e) => setNewLabel(e.target.value)}
                  disabled={adding}
                  size="small"
                />
              </DialogContent>
              <DialogActions>
                <Button variant="outlined" onClick={() => setAddDialogOpen(false)} disabled={adding}>
                  Cancel
                </Button>
                <Button variant="contained" type="submit" disabled={adding}>
                  {adding ? "Registering..." : "Register Passkey"}
                </Button>
              </DialogActions>
            </form>
          </Dialog>

          <Dialog
            open={Boolean(renameTarget)}
            onClose={() => !renameMutation.isPending && setRenameTarget(null)}
            fullWidth
            maxWidth="xs"
          >
            <form onSubmit={handleRenameSubmit}>
              <DialogTitle>Rename Passkey</DialogTitle>
              <DialogContent>
                <TextField
                  autoFocus
                  fullWidth
                  label="Passkey Label"
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  disabled={renameMutation.isPending}
                  size="small"
                  sx={{ mt: 1 }}
                />
              </DialogContent>
              <DialogActions>
                <Button
                  variant="outlined"
                  onClick={() => setRenameTarget(null)}
                  disabled={renameMutation.isPending}
                >
                  Cancel
                </Button>
                <Button variant="contained" type="submit" disabled={renameMutation.isPending || !renameValue.trim()}>
                  Save
                </Button>
              </DialogActions>
            </form>
          </Dialog>

          <Dialog
            open={Boolean(removeTarget)}
            onClose={() => !removeMutation.isPending && setRemoveTarget(null)}
          >
            <DialogTitle>Remove Passkey</DialogTitle>
            <DialogContent>
              <DialogContentText>
                Are you sure you want to remove passkey &ldquo;{removeTarget?.label}&rdquo;? Removing this passkey
                prevents future logins with it, but any active session or refresh token already issued to that device
                remains valid until it expires or is revoked under <strong>Access Tokens</strong>.
              </DialogContentText>
            </DialogContent>
            <DialogActions>
              <Button
                variant="outlined"
                onClick={() => setRemoveTarget(null)}
                disabled={removeMutation.isPending}
              >
                Cancel
              </Button>
              <Button
                variant="contained"
                color="error"
                onClick={handleConfirmRemove}
                disabled={removeMutation.isPending}
              >
                Remove
              </Button>
            </DialogActions>
          </Dialog>
        </Box>
      </Fade>
    </FetchStatusCheck>
  );
}
