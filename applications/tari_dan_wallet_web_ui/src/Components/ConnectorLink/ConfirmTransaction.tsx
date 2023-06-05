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

import { useEffect, useState, useRef } from 'react';
import Button from '@mui/material/Button';
import Typography from '@mui/material/Typography';
import Dialog from '@mui/material/Dialog';
import DialogActions from '@mui/material/DialogActions';
import DialogContent from '@mui/material/DialogContent';
import CloseIcon from '@mui/icons-material/Close';
import IconButton from '@mui/material/IconButton';
import './ConnectorLink.css';
import CheckMark from './CheckMark';
import ConnectorLogo from './ConnectorLogo';

const ConnectorDialog = () => {
  const [page, setPage] = useState(1);
  const [isOpen, setIsOpen] = useState(false);
  const [linkDetected, setLinkDetected] = useState(false);
  const [link, _setLink] = useState('');
  const linkRef = useRef<HTMLInputElement>(null);

  async function getClipboardContent() {
    if (navigator.clipboard && navigator.clipboard.readText) {
      try {
        const clipboardData = await navigator.clipboard.readText();
        // currently checks for the value 'transaction://' - this needs to be replaced with the correct value
        if (clipboardData.startsWith('transaction://')) {
          setIsOpen(true);
          setLinkDetected(true);
          _setLink(clipboardData);
        } else {
          setLinkDetected(false);
          _setLink('');
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

  useEffect(() => {
    getClipboardContent();
  }, []);

  const renderConfirmTransaction = () => {
    switch (page) {
      case 1:
        return (
          <div className="dialog-inner">
            <Typography>Opensea.com wants to send a transaction.</Typography>
            <Typography style={{ marginBottom: '20px' }}>
              Transaction details go here ...
            </Typography>
            <DialogActions>
              <Button onClick={handleClose} variant="outlined">
                Deny
              </Button>
              <Button onClick={() => setPage(page + 1)} variant="contained">
                Approve
              </Button>
            </DialogActions>
          </div>
        );
      case 2:
        return (
          <div className="dialog-inner">
            <div style={{ textAlign: 'center', paddingBottom: '50px' }}>
              <CheckMark />
              <Typography variant="h3">Transaction Approved</Typography>
            </div>
          </div>
        );
      default:
        return null;
    }
  };

  return (
    <>
      {/* <Button variant="contained" color="primary" onClick={handleOpen}>
        Confirm Transaction
      </Button> */}
      <Dialog open={isOpen} onClose={handleClose}>
        <div className="dialog-heading">
          <div style={{ height: '24px', width: '24px' }}></div>
          <ConnectorLogo />
          <IconButton onClick={handleClose}>
            <CloseIcon />
          </IconButton>
        </div>
        <DialogContent>{renderConfirmTransaction()}</DialogContent>
      </Dialog>
    </>
  );
};

export default ConnectorDialog;
