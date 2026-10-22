// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

import { DEFAULT_PERMISSIONS, WebauthnProps } from "@components/auth/web_authn/Webauthn";
import Loading from "@components/Loading";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Grid from "@mui/material/Grid";
import { useTheme } from "@mui/material/styles";
import Typography from "@mui/material/Typography";
import { WebauthnFinishAuthRequest } from "@tari-project/ootle-ts-bindings";
import { getClientInstance, webauthnStartAuth } from "@utils/json_rpc";
import { Buffer } from "buffer";
import { FormEvent, useState } from "react";
import { Form } from "react-router-dom";

export const getCredential = async (challenge: any, allowCredentials: any) => {
  const publicKeyCredentialRequestOptions: PublicKeyCredentialRequestOptions = {
    challenge: challenge,
    allowCredentials: allowCredentials,
    timeout: 60000,
    userVerification: "required",
  };
  return await navigator.credentials.get({
    publicKey: publicKeyCredentialRequestOptions,
  });
};

function WebauthnLogin(props: WebauthnProps) {
  const { onAuthenticated } = props;
  const theme = useTheme();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const handleLogin = async (e: FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");

    try {
      // start authentication by getting challenge
      const startAuthResponse = await webauthnStartAuth();

      if (!startAuthResponse) {
        throw new Error("Failed to start authentication");
      }

      if (!startAuthResponse.challenge) {
        throw new Error("Failed to get challenge");
      }

      const challengeResponse = startAuthResponse.challenge;
      // @ts-ignore
      const challenge = Buffer.from(startAuthResponse.challenge.publicKey.challenge, "base64");
      const loginSessionId = startAuthResponse.session_id;
      // @ts-ignore
      let allowCredentials = challengeResponse.publicKey.allowCredentials.map((value: any) => {
        return {
          id: Buffer.from(value.id, "base64"),
          type: value.type,
        };
      });
      let credential = await getCredential(challenge, allowCredentials);

      if (!credential) {
        throw new Error("Failed to get credential");
      }

      const client = await getClientInstance();
      const webauthnFinishAuthRequest: WebauthnFinishAuthRequest = {
        credential,
        session_id: loginSessionId,
      };

      const token = await client.authRequest(DEFAULT_PERMISSIONS, { WebAuthN: webauthnFinishAuthRequest });
      client.setToken(token);

      onAuthenticated();
    } catch (error) {
      console.error(error);
      if (error instanceof Error) {
        setError(error.message);
      } else if (typeof error === "string") {
        setError(error);
      } else {
        setError("An unknown error occurred: " + JSON.stringify(error as any));
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <>
      <Grid size={12}>
        <Box
          style={{
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
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
              Please login with your passkey
            </Typography>
            <Form
              onSubmit={handleLogin}
              className="flex-container"
              style={{
                flexDirection: "column",
                marginTop: theme.spacing(3),
              }}
            >
              <Button variant="contained" type="submit" disabled={loading}>
                Use Passkey
              </Button>
              {loading ? <Loading /> : null}
            </Form>
            {error && (
              <Typography
                variant="h4"
                style={{
                  textAlign: "center",
                  color: "red",
                }}
              >
                {error.toString()}
              </Typography>
            )}
          </Box>
        </Box>
      </Grid>
    </>
  );
}

export default WebauthnLogin;
