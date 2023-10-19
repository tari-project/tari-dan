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

import CloseIcon from '@mui/icons-material/Close';
import { Box, Fade } from '@mui/material';
import Dialog from '@mui/material/Dialog';
import IconButton from '@mui/material/IconButton';
import Tooltip from '@mui/material/Tooltip';
import Typography from '@mui/material/Typography';
import { styled, useTheme } from '@mui/material/styles';
import useMediaQuery from '@mui/material/useMediaQuery';
import { useState } from 'react';
import {
  IoCopyOutline,
  IoDownloadOutline,
  IoExpandOutline,
  IoContractOutline,
  IoCheckmarkOutline,
} from 'react-icons/io5';
import { renderJson } from '../utils/helpers';

interface ICodeBlockExpand {
  title: string;
  content: string;
}

const CodeBlock = styled(Box)(({ theme }) => ({
  backgroundColor: theme.palette.divider,
  borderRadius: `${theme.spacing(1)} ${theme.spacing(1)} 0 0`,
  padding: theme.spacing(3),
  maxHeight: '400px',
  overflowY: 'scroll',
}));

const StyledToolbar = styled(Box)(({ theme }) => ({
  display: 'flex',
  flexDirection: 'row',
  gap: theme.spacing(1),
  background: theme.palette.background.paper,
  borderRadius: `0 0 ${theme.spacing(1)} ${theme.spacing(1)}`,
  borderRight: `1px solid ${theme.palette.divider}`,
  padding: 4,
  border: `1px solid ${theme.palette.divider}`,
}));

export default function CodeBlockExpand({ title, content }: ICodeBlockExpand) {
  const [open, setOpen] = useState(false);

  const media = useTheme();
  const matches = useMediaQuery(media.breakpoints.down('md'));
  const theme = useTheme();

  const handleClickOpen = () => {
    setOpen(true);
  };

  const handleClose = () => {
    setOpen(false);
  };

  function CodeDialog() {
    return (
      <Dialog
        fullScreen={matches}
        open={open}
        onClose={handleClose}
        maxWidth="xl"
        fullWidth
      >
        <Box
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            background: theme.palette.background.paper,
            padding: '1rem 1.5rem',
            position: 'sticky',
            top: 0,
            borderBottom: `1px solid ${theme.palette.divider}`,
          }}
        >
          <Typography variant="h5" component="div">
            {title}
          </Typography>
          <IconButton
            edge="end"
            color="inherit"
            onClick={handleClose}
            aria-label="close"
          >
            <CloseIcon />
          </IconButton>
        </Box>
        <Box
          sx={{
            padding: '2rem',
            background: theme.palette.background.paper,
          }}
        >
          {renderJson(content)}
        </Box>
        <Box
          style={{
            position: 'sticky',
            bottom: 0,
            left: 0,
            width: '100%',
          }}
        >
          <ToolBar content={content} />
        </Box>
      </Dialog>
    );
  }

  function ToolBar({ content }: { content: string }) {
    const [copied, setCopied] = useState(false);
    const formattedContent = JSON.stringify(content);

    const handleCopy = () => {
      navigator.clipboard.writeText(formattedContent);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 3000);
    };

    const handleDownload = () => {
      const element = document.createElement('a');
      const file = new Blob([formattedContent], { type: 'application/json' });
      element.href = URL.createObjectURL(file);
      element.download = `${title}.json`;
      document.body.appendChild(element);
      element.click();
    };

    const menuItems = [
      {
        tooltip: copied ? 'Copied!' : 'Copy to clipboard',
        icon: copied ? (
          <IoCheckmarkOutline style={{ height: 16, width: 16 }} />
        ) : (
          <IoCopyOutline style={{ height: 16, width: 16 }} />
        ),
        onClick: () => handleCopy(),
      },
      {
        tooltip: 'Download',
        icon: <IoDownloadOutline style={{ height: 16, width: 16 }} />,
        onClick: () => handleDownload(),
      },
      open
        ? {
            tooltip: 'Close',
            icon: <IoContractOutline style={{ height: 16, width: 16 }} />,
            onClick: () => setOpen(false),
          }
        : {
            tooltip: 'Expand',
            icon: <IoExpandOutline style={{ height: 16, width: 16 }} />,
            onClick: () => handleClickOpen(),
          },
    ];

    const renderMenu = menuItems.map((item, index) => {
      return (
        <Tooltip
          TransitionComponent={Fade}
          TransitionProps={{ timeout: 300 }}
          title={item.tooltip}
          placement="top"
          arrow
          key={index}
        >
          <IconButton
            onClick={item.onClick}
            style={{
              borderRadius: 2,
            }}
          >
            {item.icon}
          </IconButton>
        </Tooltip>
      );
    });

    return <StyledToolbar>{renderMenu}</StyledToolbar>;
  }

  return (
    <>
      <Box>
        <CodeBlock>{renderJson(content)}</CodeBlock>
        <ToolBar content={content} />
      </Box>
      <CodeDialog />
    </>
  );
}
