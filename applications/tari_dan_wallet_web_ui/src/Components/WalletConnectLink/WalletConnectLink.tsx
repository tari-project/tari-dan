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
import { TariPermission, TariPermissionAccountList, TariPermissionKeyList, TariPermissionTransactionGet, TariPermissionTransactionSend } from "../../utils/tari_permissions";

const projectId: string | null = import.meta.env.VITE_WALLET_CONNECT_PROJECT_ID || null;

const ConnectorDialog = () => {
  const [page, setPage] = useState(1);
  const [isOpen, setIsOpen] = useState(false);
  const [linkDetected, setLinkDetected] = useState(false);
  const [link, setLink] = useState("");
  const linkRef = useRef<HTMLInputElement>(null);
  const theme = useTheme();
  const [_chosenOptionalPermissions, setChosenOptionalPermissions] = useState<boolean[]>([]);

  // TODO: send permissions on WC request
  const permissions: TariPermission[] = [
    new TariPermissionKeyList(),
    new TariPermissionTransactionGet(),
    new TariPermissionTransactionSend(),
  ];
  const optionalPermissions: TariPermission[] = [];

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

  const handleAuth = () => {

    // TODO: pairing

    setPage(page + 1);

    // TODO: auth
    console.log("WC auth");
    console.log({projectId});
  };

  useEffect(() => {
    getClipboardContent();
  }, []);

  useEffect(() => setChosenOptionalPermissions(Array(optionalPermissions.length).fill(true)), [optionalPermissions]);

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
              <Button onClick={handleAuth} variant="contained">
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
        console.log("default");
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
