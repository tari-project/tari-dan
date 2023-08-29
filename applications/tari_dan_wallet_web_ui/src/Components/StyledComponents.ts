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

import Paper from '@mui/material/Paper';
import TableCell from '@mui/material/TableCell';
import { styled } from '@mui/material/styles';
import Box from '@mui/material/Box';
import IconButton from '@mui/material/IconButton';
import Typography from '@mui/material/Typography';

export const AccordionIconButton = styled(IconButton)(({ theme }) => ({
  backgroundColor: theme.palette.divider,
  color: theme.palette.primary.main,
  '&:hover': {
    backgroundColor: theme.palette.primary.main,
    color: '#fff',
  },
}));

export const StyledPaper = styled(Paper)(({ theme }) => ({
  padding: theme.spacing(3),
  boxShadow: '10px 14px 28px rgba(35, 11, 73, 0.05)',
  border: '1px solid rgba(255,255,255,0.04)',
}));

export const DataTableCell = styled(TableCell)(({ theme }) => ({
  fontFamily: "'Courier New', Courier, monospace",
}));

export const CodeBlock = styled(Box)(({ theme }) => ({
  backgroundColor: theme.palette.divider,
  borderRadius: theme.shape.borderRadius,
  padding: theme.spacing(3),
  maxHeight: '400px',
  overflowY: 'scroll',
}));

export const BoxHeading = styled(Box)(({ theme }) => ({
  backgroundColor: '#fafafa',
  borderRadius: theme.shape.borderRadius,
  padding: theme.spacing(3),
  fontFamily: "'Courier New', Courier, monospace",
  boxShadow: '0px 5px 5px rgba(35, 11, 73, 0.10)',
  margin: '10px 5px',
}));

export const BoxHeading2 = styled(Box)(({ theme }) => ({
  padding: theme.spacing(2),
  borderBottom: `1px solid ${theme.palette.divider}`,
}));

export const SubHeading = styled(Typography)(() => ({
  marginTop: '20px',
  marginBottom: '20px',
  textAlign: 'center',
}));

export const DialogContainer = styled(Box)(({ theme }) => ({
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  gap: theme.spacing(3),
  width: '100%',
}));
