import { useState } from 'react';
import { Snackbar } from '@mui/material';
import IconButton from '@mui/material/IconButton';
import ContentCopyIcon from '@mui/icons-material/ContentCopy';
import { Tooltip } from '@mui/material';

interface CopyProps {
  copy: string;
}

const CopyToClipboard = ({ copy }: CopyProps) => {
  const [open, setOpen] = useState(false);
  const handleClick = (copyThis: string) => {
    setOpen(true);
    navigator.clipboard.writeText(copyThis);
  };

  return (
    <>
      <IconButton
        onClick={() => handleClick(copy)}
        size="small"
        aria-label="copy to clipboard"
        style={{
          // float: 'right',
          marginLeft: '10px',
        }}
      >
        <Tooltip title={copy} arrow>
          <ContentCopyIcon
            color="primary"
            style={{
              width: '16px',
              height: '16px',
            }}
          />
        </Tooltip>
      </IconButton>
      <Snackbar
        open={open}
        onClose={() => setOpen(false)}
        autoHideDuration={2000}
        message="Copied to clipboard"
      />
    </>
  );
};

export default CopyToClipboard;
