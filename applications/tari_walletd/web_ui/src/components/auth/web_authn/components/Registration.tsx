// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import { APP_NAME, WebauthnProps } from "@components/auth/web_authn/Webauthn";
import Loading from "@components/Loading";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Grid from "@mui/material/Grid";
import { useTheme } from "@mui/material/styles";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import { getClientInstance, webauthnFinishRegistration, webauthnStartRegistration } from "@utils/json_rpc";
import { Buffer } from "buffer";
import { FormEvent, useEffect, useState } from "react";
import { Form } from "react-router-dom";

const WEBAUTHN_RP_ID = import.meta.env.VITE_DAEMON_WEBAUTHN_RP_ID || window.location.hostname;

const createCredential = async (
  rpOptions: { rpId: string; rpName: string },
  username: string,
  challenge: BufferSource,
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
    excludeCredentials: [],
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

function WebauthnRegistration(props: WebauthnProps) {
  const { onAuthenticated } = props;
  const theme = useTheme();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [setupToken, setSetupToken] = useState(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get("token")?.trim() || "";
  });

  useEffect(() => {
    const url = new URL(window.location.href);
    const tokenFromQuery = url.searchParams.get("token")?.trim();
    if (tokenFromQuery) {
      setSetupToken(tokenFromQuery);
      url.searchParams.delete("token");
      window.history.replaceState({}, document.title, url.pathname + url.search + url.hash);
    }
  }, []);

  const handleRegister = async (e: FormEvent) => {
    e.preventDefault();
    const trimmedToken = setupToken.trim();
    if (!trimmedToken) {
      setError("A one-time setup token is required to register the first passkey.");
      return;
    }

    setLoading(true);
    setError("");

    try {
      // start registration by getting challenge
      const startRegisterResponse = await webauthnStartRegistration({
        username: APP_NAME,
        setup_token: trimmedToken,
      });

      if (!startRegisterResponse) {
        throw new Error("Failed to start registration");
      }

      if (!startRegisterResponse.public_key) {
        throw new Error("Failed to start registration: missing public_key");
      }

      const challenge = Buffer.from((startRegisterResponse.public_key as any).challenge, "base64");
      const regSessionId = startRegisterResponse.session_id;

      // get credential
      const credential = await createCredential(
        {
          rpId: WEBAUTHN_RP_ID,
          rpName: "Tari Ootle Wallet Web UI",
        },
        APP_NAME,
        challenge,
      );

      if (!credential) {
        throw new Error("Failed to create credential");
      }

      const { token } = await webauthnFinishRegistration({
        credential,
        session_id: regSessionId,
        setup_token: trimmedToken,
      });

      let client = await getClientInstance();
      client.setToken(token);

      onAuthenticated();
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
          <Box>
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
              <a href="https://webauthn.io/" target="_blank">
                passkey
              </a>{" "}
              key to get started
            </Typography>

            <Form
              onSubmit={handleRegister}
              className="flex-container"
              style={{
                flexDirection: "column",
                gap: theme.spacing(2),
                marginTop: theme.spacing(3),
              }}
            >
              <TextField
                label="One-time setup token"
                placeholder="Paste setup token from wallet daemon log or setup_token file"
                value={setupToken}
                onChange={(e) => setSetupToken(e.target.value)}
                required
                fullWidth
                size="small"
                disabled={loading}
              />
              <Button variant="contained" type="submit" disabled={loading || !setupToken.trim()}>
                Register
              </Button>
              {loadingBar}
            </Form>
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
