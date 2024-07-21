//  Copyright 2022. The Tari Project
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

import React, { useEffect, useState, useRef } from "react";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import Dialog from "@mui/material/Dialog";
import DialogActions from "@mui/material/DialogActions";
import DialogContent from "@mui/material/DialogContent";
import DialogContentText from "@mui/material/DialogContentText";
import CloseIcon from "@mui/icons-material/Close";
import IconButton from "@mui/material/IconButton";
import "./ConnectorLink.css";
import Permissions from "./Permissions";
import CheckMark from "./CheckMark";
import ConnectorLogo from "./ConnectorLogo";
import ConfirmTransaction from "./ConfirmTransaction";
import { useTheme } from "@mui/material/styles";
import { TariPermission, TariPermissionKeyList, TariPermissionTransactionGet, TariPermissionTransactionSend } from "../../utils/tari_permissions";
import { Core } from '@walletconnect/core'
import { Web3Wallet } from '@walletconnect/web3wallet'
import { accountsGetBalances, accountsGetDefault, confidentialViewVaultBalance, keysCreate, substatesGet, templatesGet, transactionsGet, transactionsGetResult, transactionsSubmit } from "../../utils/json_rpc";

const projectId: string | null = import.meta.env.VITE_WALLET_CONNECT_PROJECT_ID || null;

const ConnectorDialog = () => {
  const [page, setPage] = useState(1);
  const [isOpen, setIsOpen] = useState(false);
  const [linkDetected, setLinkDetected] = useState(false);
  const [link, setLink] = useState("");
  const linkRef = useRef<HTMLInputElement>(null);
  const theme = useTheme();
  const [_chosenOptionalPermissions, setChosenOptionalPermissions] = useState<boolean[]>([]);

  const [web3wallet, setWeb3wallet] = useState<Web3Wallet | undefined>();

  // TODO: send permissions on WC request
  const permissions: TariPermission[] = [
    new TariPermissionKeyList(),
    new TariPermissionTransactionGet(),
    new TariPermissionTransactionSend(),
  ];
  const optionalPermissions: TariPermission[] = [];

  async function createWallet(): Web3Wallet | null {
    const core = new Core({ projectId });
    const wallet = await Web3Wallet.init({
      core: core,
      metadata: {
        name: 'Example WalletConnect Wallet',
        description: 'Example WalletConnect Integration',
        url: 'myexamplewallet.com',
        icons: []
      }
    });

    wallet.on('session_proposal', async proposal => {
      console.log({ proposal });

      const session = await wallet.approveSession({
        id: proposal.id,
        namespaces: {
          tari: {
            methods: [
              'tari_getSubstate',
              'tari_getDefaultAccount',
              'tari_getAccountBalances',
              'tari_submitTransaction',
              'tari_getTransactionResult',
              'tari_getTemplate',
              'tari_createKey',
              'tari_viewConfidentialVaultBalance'
            ],
            chains: [
              'tari:devnet',
            ],
            events: ['chainChanged", "accountsChanged'],
            accounts: [
              "tari:devnet:component_d43f1d674a0df0579354659d1b0c8dd4a397b072afa9dd027e41c8bc"
            ],
          }
        }
      })

      // create response object
      const response = { id: proposal.id, result: 'session approved', jsonrpc: '2.0' }

      // respond to the dapp request with the approved session's topic and response
      await wallet.respondSessionRequest({ topic: session.topic, response })
    });

    wallet.on('session_request', async requestEvent => {
      console.log({ requestEvent });
      const { params, id, topic } = requestEvent;
      const { request } = params;
      
      const result = await executed_method(request.method, request.params);
      
      const response = { id, result, jsonrpc: '2.0' }
      await wallet.respondSessionRequest({ topic, response });
    });

    return wallet;
  }

  async function executed_method(method: string, params: any) {
    switch(method) {
      case "tari_getSubstate":
        return substatesGet(params);
      case "tari_getDefaultAccount":
        return accountsGetDefault(params);
      case "tari_getAccountBalances":
        return accountsGetBalances(params);
      case "tari_submitTransaction":
        return transactionsSubmit(params);
      case "tari_getTransactionResult":
        return transactionsGetResult(params);
      case "tari_getTemplate":
        return templatesGet(params);
      case "tari_createKey":
        return keysCreate(params);
      case "tari_viewConfidentialVaultBalance":
        return confidentialViewVaultBalance(params);
      default:
        throw new Error("Invalid method")
    }
  }

  async function getClipboardContent() {
    if (navigator.clipboard && navigator.clipboard.readText) {
      try {
        const clipboardData = await navigator.clipboard.readText();
        if (clipboardData.startsWith("wc:")) {
          setLinkDetected(true);
          setLink(clipboardData);
          setIsOpen(true);
        } else {
          setLinkDetected(false);
          setLink("");
        }
      } catch (err) {
        console.error(`Failed to read clipboard contents: ${err}`);
      }
    } else {
      console.warn("Clipboard API not supported in this browser");
    }
  }

  const handleOpen = () => {
    getClipboardContent();
    setIsOpen(true);
  };

  const handleClose = () => {
    setIsOpen(false);
    setTimeout(() => {
      setPage(1);
    }, 500);
  };

  const handleConnect = () => {
    linkRef.current && setLink(linkRef.current.value);
    setPage(page + 1);
  };

  const handleConnectWithLink = () => {
    setPage(page + 1);
  };

  const handleAuth = async () => {
    let wallet = web3wallet;
    if (!wallet) {
      wallet = await createWallet();
      setWeb3wallet(wallet);
    }

    if (!link) {
      console.error("No WalletConnect link found");
      return;
    }

    console.log({wallet});
    console.log({link});

    const result = await wallet.pair({ uri: link });
    console.log({ result });

    setPage(page + 1);
  };

  useEffect(() => {
    getClipboardContent();
  }, []);

  const renderPage = () => {
    switch (page) {
      case 1:
        if (linkDetected) {
          return (
            <div className="dialog-inner">
              <DialogContentText style={{ paddingBottom: "20px" }}>
                A WalletConnect link was detected. <br />
                Would you like to connect to <code style={{ color: "purple", fontSize: "14px" }}>{link}</code>?
              </DialogContentText>
              <DialogActions>
                <Button variant="outlined" onClick={handleClose}>
                  No
                </Button>
                <Button variant="contained" onClick={handleConnectWithLink}>
                  Yes, Connect
                </Button>
              </DialogActions>
            </div>
          );
        } else {
          return (
            <div className="dialog-inner">
              <DialogContentText style={{ paddingBottom: "20px" }}>
                To connect your wallet, add a wallet connect link here:
              </DialogContentText>
              <TextField name="link" label="Connector Link" inputRef={linkRef} fullWidth />
              <DialogActions>
                <Button variant="outlined" onClick={handleClose}>
                  Cancel
                </Button>
                <Button variant="contained" onClick={handleConnect}>
                  Connect
                </Button>
              </DialogActions>
            </div>
          );
        }
      case 2:
        return (
          <div className="dialog-inner">
            <Permissions
              requiredPermissions={permissions}
              optionalPermissions={optionalPermissions}
              setOptionalPermissions={setChosenOptionalPermissions}
            />
            <DialogActions>
              <Button onClick={handleClose} variant="outlined">
                Cancel
              </Button>
              <Button onClick={async () => await handleAuth()} variant="contained">
                Authorize
              </Button>
            </DialogActions>
          </div>
        );
      case 3:
        return (
          <div className="dialog-inner">
            <div style={{ textAlign: "center", paddingBottom: "50px" }}>
              <CheckMark />
              <Typography variant="h3">Wallet Connected</Typography>
            </div>
          </div>
        );
      default:
        return (<></>);
    }
  };

  // Don't render anything if wallet connect is not set up in the wallet daemon
  if (!projectId) {
    return (<></>);
  } else {
    return (
      <>
        <Button
          variant="contained"
          color="primary"
          onClick={handleOpen}
          style={{
            height: "48px",
          }}
        >
          Connect with WalletConnect
        </Button>
        <Dialog open={isOpen} onClose={handleClose}>
          <div className="dialog-heading">
            <div style={{ height: "24px", width: "24px" }}></div>
            <ConnectorLogo fill={theme.palette.text.primary} />
            <IconButton onClick={handleClose}>
              <CloseIcon />
            </IconButton>
          </div>
          <DialogContent>{renderPage()}</DialogContent>
        </Dialog>
        <ConfirmTransaction />
      </>
    )
  };
};

export default ConnectorDialog;
