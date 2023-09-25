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

import { useState } from 'react';
import Button from '@mui/material/Button';
import { styled } from '@mui/material/styles';
import Dialog from '@mui/material/Dialog';
import DialogTitle from '@mui/material/DialogTitle';
import DialogContent from '@mui/material/DialogContent';
import DialogActions from '@mui/material/DialogActions';
import IconButton from '@mui/material/IconButton';
import CloseIcon from '@mui/icons-material/Close';
import Typography from '@mui/material/Typography';
import { Alert, Divider } from '@mui/material';
import Box from '@mui/material/Box';
import TariLogo from '../assets/TariLogo';
import { DialogContainer } from './StyledComponents';
import FailureIcon from '../assets/images/FailureIcon';
import SuccessIcon from '../assets/images/SuccessIcon';
import { useTheme } from '@mui/material/styles';

interface DialogTitleProps {
  id: string;
  children?: React.ReactNode;
  onClose: () => void;
}

export const TransactionMessage = ({ transactionStatus }: any) => {
  const message = () => {
    switch (transactionStatus) {
      case 'Approved':
        return 'Transaction approved';
      case 'Rejected':
        return 'Transaction rejected';
      default:
        return 'Transaction approval request';
    }
  };

  const alertSeverity = () => {
    switch (transactionStatus) {
      case 'Approved':
        return 'success';
      case 'Rejected':
        return 'error';
      default:
        return 'warning';
    }
  };

  return (
    <Alert severity={alertSeverity()} variant="outlined">
      {message()}
    </Alert>
  );
};

const BootstrapDialog = styled(Dialog)(({ theme }) => ({
  '& .MuiDialogContent-root': {
    padding: theme.spacing(2),
  },
  '& .MuiDialogActions-root': {
    padding: theme.spacing(1),
  },
}));

const BoxRow = styled(Box)(({ theme }) => ({
  display: 'flex',
  padding: '0px',
  flexDirection: 'row',
  justifyContent: 'space-between',
  alignItems: 'center',
  gap: theme.spacing(1),
  width: '100%',
}));

function TariDialogTitle(props: DialogTitleProps) {
  const { children, onClose, ...other } = props;

  return (
    <DialogTitle sx={{ m: 0, p: 2 }} {...other}>
      <Box
        sx={{
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        {children}
      </Box>
      {onClose ? (
        <IconButton
          aria-label="close"
          onClick={onClose}
          sx={{
            position: 'absolute',
            right: 8,
            top: 8,
            color: (theme) => theme.palette.grey[600],
          }}
        >
          <CloseIcon />
        </IconButton>
      ) : null}
    </DialogTitle>
  );
}

export default function TransactionRequest() {
  const [open, setOpen] = useState(false);
  const [requestDetails, setRequestDetails] = useState([
    {
      title: 'Amount',
      value: '29.99',
    },
    {
      title: 'Estimated Fee',
      value: '0.05',
    },
    {
      title: 'Date',
      value: '9 August 2023, 12:15',
    },
    {
      title: 'Transaction ID',
      value: '0x1234567890',
    },
  ]);
  const [transactionStatus, setTransactionStatus] = useState<
    'Pending' | 'Approved' | 'Rejected'
  >('Pending');
  const theme = useTheme();

  const renderPage = () => {
    switch (transactionStatus) {
      case 'Pending':
        return (
          <Box
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: theme.spacing(3),
              padding: theme.spacing(3),
            }}
          >
            <Box>
              <Typography
                gutterBottom
                style={{ textAlign: 'center' }}
                variant="h4"
              >
                Transaction approval request
              </Typography>
              <Typography
                gutterBottom
                style={{ textAlign: 'center' }}
                variant="body1"
              >
                Website is requesting approval for Transaction Details. <br />
                Would you like to approve this request?
              </Typography>
            </Box>
            <Divider />
            <Box
              style={{
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                gap: 0,
                width: '100%',
              }}
            >
              {requestDetails.map(({ title, value }, index) => (
                <BoxRow key={index}>
                  <Typography
                    gutterBottom
                    style={{ textAlign: 'center' }}
                    variant="h5"
                  >
                    {title}
                  </Typography>
                  <Typography
                    gutterBottom
                    style={{ textAlign: 'center' }}
                    variant="body1"
                  >
                    {value}
                  </Typography>
                </BoxRow>
              ))}
            </Box>
            <Divider />
            <Box
              style={{
                display: 'flex',
                padding: '0px',
                flexDirection: 'row',
                justifyContent: 'space-between',
                alignItems: 'center',
                width: '100%',
              }}
            >
              <Typography
                gutterBottom
                style={{ textAlign: 'center' }}
                variant="h5"
              >
                Estimated Total
              </Typography>
              <Typography
                gutterBottom
                style={{ textAlign: 'center' }}
                variant="h5"
              >
                {parseFloat(requestDetails[0].value) +
                  parseFloat(requestDetails[1].value)}{' '}
                TXR
              </Typography>
            </Box>
            <Box
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                width: '100%',
                gap: theme.spacing(2),
              }}
            >
              <Button
                variant="outlined"
                onClick={handleReject}
                style={{ width: '175px' }}
              >
                Reject
              </Button>
              <Button
                variant="contained"
                onClick={handleApprove}
                style={{ width: '175px' }}
              >
                Approve
              </Button>
            </Box>
          </Box>
        );
      case 'Approved':
        return (
          <DialogContainer
            style={{
              width: 450,
            }}
          >
            <SuccessIcon />
            <Typography variant="h4">Transaction Approved!</Typography>
            <Typography variant="body1">
              Go back to website to view your purchase
            </Typography>
          </DialogContainer>
        );
      case 'Rejected':
        return (
          <DialogContainer
            style={{
              width: 450,
            }}
          >
            <FailureIcon />
            <Box
              style={{
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                width: '100%',
                gap: 0,
              }}
            >
              <Typography variant="h4">Transaction Failed</Typography>
              <Typography variant="body1">Error message</Typography>
            </Box>
          </DialogContainer>
        );
    }
  };

  const handleClickOpen = () => {
    setOpen(true);
  };
  const handleClose = () => {
    setOpen(false);
    setTimeout(() => {
      setTransactionStatus('Pending');
    }, 2000);
  };

  const handleReject = () => {
    setTransactionStatus('Rejected');
    setTimeout(() => {
      handleClose();
    }, 1000);
  };

  const handleApprove = () => {
    setTransactionStatus('Approved');
    setTimeout(() => {
      handleClose();
    }, 1000);
  };

  return (
    <Box
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: theme.spacing(2),
        padding: theme.spacing(4),
        background: theme.palette.background.paper,
        borderRadius: theme.shape.borderRadius,
      }}
    >
      <Button variant="outlined" onClick={handleClickOpen}>
        Transaction Request
      </Button>
      <TransactionMessage transactionStatus={transactionStatus} />
      <BootstrapDialog
        onClose={handleClose}
        aria-labelledby="customized-dialog-title"
        open={open}
      >
        <TariDialogTitle id="customized-dialog-title" onClose={handleClose}>
          <TariLogo fill={theme.palette.text.primary} />
        </TariDialogTitle>
        <DialogContent>{renderPage()}</DialogContent>
        <DialogActions></DialogActions>
      </BootstrapDialog>
    </Box>
  );
}
