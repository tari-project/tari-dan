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

import React, { useEffect, useState, useRef } from 'react';
import Button from '@mui/material/Button';
import TextField from '@mui/material/TextField';
import Typography from '@mui/material/Typography';
import Dialog from '@mui/material/Dialog';
import DialogActions from '@mui/material/DialogActions';
import DialogContent from '@mui/material/DialogContent';
import DialogContentText from '@mui/material/DialogContentText';
import CopyToClipboard from '../CopyToClipboard';
import CloseIcon from '@mui/icons-material/Close';
import IconButton from '@mui/material/IconButton';
import './ConnectorLink.css';
import Permissions from './Permissions';
import CheckMark from './CheckMark';
import connectorLogo from './assets/tari-connector.png';
import ConnectorLogo from './ConnectorLogo';
import { parse } from '../../utils/tari_permissions';
import { webrtc } from '../../utils/json_rpc';

const ConnectorDialog = () => {
  const [page, setPage] = useState(1);
  const [isOpen, setIsOpen] = useState(false);
  const [linkDetected, setLinkDetected] = useState(false);
  const [link, _setLink] = useState('');
  const [signalingServerJWT, setSignalingServerJWT] = useState('');
  const [permissions, setPermissions] = useState([]);
  const [optionalPermissions, setOptionalPermissions] = useState([]);
  const [chosenOptionalPermissions, setChosenOptionalPermissions] = useState<boolean[]>([])
  const linkRef = useRef<HTMLInputElement>(null);

  const setLink = (value: string) => {
    const re = /tari:\/\/([a-zA-Z0-9\-_]+\.[a-zA-Z0-9\-_]+\.[a-zA-Z0-9\-_]+)\/(.*)\/(.*)/i
    let groups;
    if (groups = re.exec(value)) {
      setSignalingServerJWT(groups[1]);
      setPermissions(JSON.parse(groups[2]).map((permission: any) => parse(permission)));
      setOptionalPermissions(JSON.parse(groups[3]).map((permission: any) => parse(permission)));
    }
    _setLink(value);
  }

  useEffect(() => setChosenOptionalPermissions(Array(optionalPermissions.length).fill(true)), [optionalPermissions])

  async function getClipboardContent() {
    if (navigator.clipboard && navigator.clipboard.readText) {
      try {
        const clipboardData = await navigator.clipboard.readText();
        if (clipboardData.startsWith('tari://')) {
          setIsOpen(true);
          setLinkDetected(true);
          setLink(clipboardData);
        } else {
          setLinkDetected(false);
          setLink('');
        }
      } catch (err) {
        console.error(`Failed to read clipboard contents: ${err}`);
      }
    } else {
      console.warn('Clipboard API not supported in this browser');
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

  const handleAuth = () => {
    console.log('chosenOptionalPermissions', chosenOptionalPermissions)
    let allowedPermissions = [...permissions, ...optionalPermissions.filter((value, index) => chosenOptionalPermissions[index])];
    console.log('jwt', signalingServerJWT);
    console.log('perm', JSON.stringify(allowedPermissions));
    webrtc(signalingServerJWT, JSON.stringify(allowedPermissions)).then((resp) => {
      setPage(page + 1);
    });
  };

  const handleConnect = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    linkRef.current && setLink(linkRef.current.value);
    setPage(page + 1);
  };

  const handleConnectWithLink = () => {
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
              <DialogContentText style={{ paddingBottom: '20px' }}>
                A connector link was detected. <br />
                Would you like to connect to{' '}
                <code style={{ color: 'purple', fontSize: '14px' }}>
                  {link}
                </code>
                ?
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
              <DialogContentText>
                To connect your wallet, add a connector link here:
              </DialogContentText>
              <form
                onSubmit={handleConnect}
                style={{
                  marginTop: '1rem',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '1rem',
                }}
              >
                <TextField
                  name="link"
                  label="Connector Link"
                  inputRef={linkRef}
                  fullWidth
                />
                <div className="dialog-actions">
                  <Button variant="outlined" onClick={handleClose}>
                    Cancel
                  </Button>
                  <Button type="submit" variant="contained">
                    Connect
                  </Button>
                </div>
              </form>
            </div>
          );
        }
      case 2:
        return (
          <div className="dialog-inner">
            <Permissions requiredPermissions={permissions} optionalPermissions={optionalPermissions} setOptionalPermissions={setChosenOptionalPermissions} />
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
            <div style={{ textAlign: 'center', paddingBottom: '50px' }}>
              <CheckMark />
              <Typography variant="h3">Wallet Connected</Typography>
            </div>
          </div>
        );
      default:
        return null;
    }
  };

  return (
    <>
      <Button variant="contained" color="primary" onClick={handleOpen}>
        Connect
      </Button>
      <Dialog open={isOpen} onClose={handleClose}>
        <div className="dialog-heading">
          <div style={{ height: '24px', width: '24px' }}></div>
          <ConnectorLogo />
          <IconButton onClick={handleClose}>
            <CloseIcon />
          </IconButton>
        </div>
        <DialogContent>{renderPage()}</DialogContent>
      </Dialog>
    </>
  );
};

export default ConnectorDialog;
