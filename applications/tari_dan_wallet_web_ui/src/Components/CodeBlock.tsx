import { IoExpandOutline } from 'react-icons/io5';

import { useState } from 'react';
import { DialogTitle, DialogContent } from '@mui/material';
import Dialog from '@mui/material/Dialog';
import AppBar from '@mui/material/AppBar';
import Toolbar from '@mui/material/Toolbar';
import IconButton from '@mui/material/IconButton';
import Typography from '@mui/material/Typography';
import CloseIcon from '@mui/icons-material/Close';
import { Box } from '@mui/material';
import { CodeBlock } from './StyledComponents';
import useMediaQuery from '@mui/material/useMediaQuery';
import { useTheme } from '@mui/material/styles';

export default function CodeBlockExpand({ title, children }: any) {
  const [open, setOpen] = useState(false);
  const theme = useTheme();
  const matches = useMediaQuery(theme.breakpoints.down('md'));

  const handleClickOpen = () => {
    setOpen(true);
  };

  const handleClose = () => {
    setOpen(false);
  };

  return (
    <div>
      <CodeBlock
        style={{
          position: 'relative',
        }}
      >
        {children}
        <IconButton
          onClick={handleClickOpen}
          style={{
            position: 'sticky',
            bottom: '0',
            float: 'right',
          }}
        >
          <IoExpandOutline style={{ height: 16, width: 16 }} />
        </IconButton>
      </CodeBlock>
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
            background: '#FFFFFF',
            padding: '1rem 1.5rem',
            position: 'sticky',
            top: 0,
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
            background: '#f5f5f7',
          }}
        >
          {children}
        </Box>
      </Dialog>
    </div>
  );
}
