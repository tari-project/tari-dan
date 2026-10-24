// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import { APP_NAME, WebauthnProps } from "@components/auth/web_authn/Webauthn";
import Loading from "@components/Loading";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Grid from "@mui/material/Grid";
import Stack from "@mui/material/Stack";
import { useTheme } from "@mui/material/styles";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import { getClientInstance, webauthnFinishRegistration, webauthnStartRegistration } from "@utils/json_rpc";
import { Buffer } from "buffer";
import { FormEvent, useState } from "react";
import { Form } from "react-router-dom";

const WEBAUTHN_RP_ID = import.meta.env.VITE_DAEMON_WEBAUTHN_RP_ID || window.location.hostname;

const createCredential = async (
  rpOptions: { rpId: string; rpName: string },
  username: string,
  challenge: BufferSource,
  excludeCredentials: PublicKeyCredentialDescriptor[] = [],
) => {
  const publicKeyCredentialCreationOptions: PublicKeyCredentialCreationOptions = {
    rp: {
      name: rpOptions.rpName,
      id: rpOptions.rpId,
    },
    user: {
      id: Uint8Array.from(username, (c) => c.charCodeAt(0)),
      name: username,
      displayName: username,
    },
    challenge,
    pubKeyCredParams: [
      {
        type: "public-key",
        // Ed25519
        alg: -8,
      },
      {
        type: "public-key",
        // ES256
        alg: -7,
      },
      {
        type: "public-key",
        // RS256
        alg: -257,
      },
    ],
    timeout: 60000,
    excludeCredentials,
    attestation: "none",
    extensions: {
      credProps: true,
    },
    authenticatorSelection: {
      userVerification: "required",
    },
  };
  return await navigator.credentials.create({
    publicKey: publicKeyCredentialCreationOptions,
  });
};

export async function performPasskeyRegistration(options?: {
  label?: string;
  updateClientToken?: boolean;
}): Promise<string | null | undefined> {
  const trimmedLabel = options?.label?.trim() || undefined;
  const startRegisterResponse = await webauthnStartRegistration({
    username: APP_NAME,
    label: trimmedLabel ?? null,
  });

  if (!startRegisterResponse) {
    throw new Error("Failed to start registration");
  }

  const publicKeyOptions = startRegisterResponse.public_key as any;
  if (!publicKeyOptions) {
    throw new Error("Failed to start registration: missing public_key");
  }

  const challenge = Buffer.from(publicKeyOptions.challenge, "base64");
  const excludeCredentials: PublicKeyCredentialDescriptor[] = Array.isArray(publicKeyOptions.excludeCredentials)
    ? publicKeyOptions.excludeCredentials.map((cred: any) => ({
        type: cred.type || "public-key",
        id: Buffer.from(cred.id, "base64"),
        transports: cred.transports,
      }))
    : [];
  const regSessionId = startRegisterResponse.session_id;

  const credential = await createCredential(
    {
      rpId: WEBAUTHN_RP_ID,
      rpName: "Tari Ootle Wallet Web UI",
    },
    APP_NAME,
    challenge,
    excludeCredentials,
  );

  if (!credential) {
    throw new Error("Failed to create credential");
  }

  const { token } = await webauthnFinishRegistration({
    credential,
    session_id: regSessionId,
  });

  if (options?.updateClientToken !== false && token) {
    const client = await getClientInstance();
    client.setToken(token);
  }

  return token;
}

function WebauthnRegistration(props: WebauthnProps) {
  const { onAuthenticated } = props;
  const theme = useTheme();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [label, setLabel] = useState("");
  const [backupLabel, setBackupLabel] = useState("Backup Passkey");
  const [showBackupPrompt, setShowBackupPrompt] = useState(false);

  const handleRegister = async (e: FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");

    try {
      await performPasskeyRegistration({
        label,
        updateClientToken: true,
      });
      setShowBackupPrompt(true);
    } catch (error) {
      console.error("Error registering WebAuthn key:", error);
      if (error instanceof Error) {
        setError(error.message);
      } else {
        setError(error as string);
      }
    } finally {
      setLoading(false);
    }
  };

  const handleAddBackupPasskey = async (e: FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");

    try {
      await performPasskeyRegistration({
        label: backupLabel || "Backup Passkey",
        updateClientToken: false,
      });
      onAuthenticated();
    } catch (error) {
      console.error("Error registering backup WebAuthn key:", error);
      if (error instanceof Error) {
        setError(error.message);
      } else {
        setError(error as string);
      }
    } finally {
      setLoading(false);
    }
  };

  const loadingBar = loading ? <Loading /> : null;

  return (
    <>
      <Grid size={12}>
        <Box
          style={{
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            flexDirection: "column",
          }}
        >
          <Box sx={{ maxWidth: 480, width: "100%" }}>
            {!showBackupPrompt ? (
              <>
                <Typography
                  variant="h3"
                  style={{
                    textAlign: "center",
                  }}
                >
                  Welcome to the Tari Asset Vault
                </Typography>
                <Typography
                  variant="h5"
                  style={{
                    textAlign: "center",
                  }}
                >
                  Register your{" "}
                  <a href="https://webauthn.io/" target="_blank" rel="noreferrer">
                    passkey
                  </a>{" "}
                  to get started
                </Typography>

                <Form
                  onSubmit={handleRegister}
                  className="flex-container"
                  style={{
                    flexDirection: "column",
                    marginTop: theme.spacing(3),
                    gap: theme.spacing(2),
                  }}
                >
                  <TextField
                    label="Passkey Label (optional)"
                    placeholder="e.g. This Device"
                    value={label}
                    onChange={(e) => setLabel(e.target.value)}
                    disabled={loading}
                    size="small"
                    fullWidth
                  />
                  <Button variant="contained" type="submit" disabled={loading}>
                    Register
                  </Button>
                  {loadingBar}
                </Form>
              </>
            ) : (
              <>
                <Typography
                  variant="h3"
                  style={{
                    textAlign: "center",
                  }}
                >
                  Add a Backup Passkey
                </Typography>
                <Typography
                  variant="body1"
                  style={{
                    textAlign: "center",
                    marginTop: theme.spacing(1),
                  }}
                >
                  Your primary passkey is registered. Add a backup passkey now (such as a hardware security key or a
                  second device) so you can still sign in if you lose access to this authenticator. You can also manage
                  passkeys later in Settings &rarr; Passkeys.
                </Typography>

                <Form
                  onSubmit={handleAddBackupPasskey}
                  className="flex-container"
                  style={{
                    flexDirection: "column",
                    marginTop: theme.spacing(3),
                    gap: theme.spacing(2),
                  }}
                >
                  <TextField
                    label="Backup Passkey Label"
                    placeholder="e.g. YubiKey / Phone"
                    value={backupLabel}
                    onChange={(e) => setBackupLabel(e.target.value)}
                    disabled={loading}
                    size="small"
                    fullWidth
                  />
                  <Stack direction="row" spacing={2} justifyContent="center">
                    <Button variant="outlined" onClick={() => onAuthenticated()} disabled={loading}>
                      Skip for now
                    </Button>
                    <Button variant="contained" type="submit" disabled={loading}>
                      Add Backup Passkey
                    </Button>
                  </Stack>
                  {loadingBar}
                </Form>
              </>
            )}
            {error && (
              <Box
                style={{
                  marginTop: theme.spacing(2),
                  width: "100%",
                  display: "flex",
                  justifyContent: "center",
                }}
              >
                <Alert severity={"error"}>
                  <Typography variant="h6" style={{ color: "red" }}>
                    {error.toString()}
                  </Typography>
                </Alert>
              </Box>
            )}
          </Box>
        </Box>
      </Grid>
    </>
  );
}

export default WebauthnRegistration;
