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

import { ThemeOptions } from '@mui/material/styles';
import {
  tariPurple,
  blue,
  red,
  green,
  orange,
  grey,
  teal,
  gothic,
} from './colors';

export const componentSettings: ThemeOptions = {
  shape: {
    borderRadius: 8,
  },
  spacing: 8,
  typography: {
    fontFamily: '"AvenirMedium", sans-serif',
    fontSize: 14,
    body1: {},
    body2: {
      lineHeight: '1.5rem',
    },
    h1: {
      fontSize: '2.2rem',
      lineHeight: '3.2rem',
    },
    h2: {
      fontSize: '1.9rem',
      lineHeight: '2.9rem',
    },
    h3: {
      fontSize: '1.6rem',
      lineHeight: '2.6rem',
    },
    h4: {
      fontSize: '1.3rem',
      lineHeight: '2.3rem',
    },
    h5: {
      fontSize: '1rem',
      lineHeight: '2em',
    },
    h6: {
      fontSize: '0.875rem',
      lineHeight: '1.8rem',
    },
  },
  transitions: {
    duration: {
      enteringScreen: 500,
      leavingScreen: 500,
    },
  },
  components: {
    MuiButton: {
      defaultProps: {
        size: 'large',
        sx: {
          textTransform: 'none',
        },
      },
    },
    MuiPaper: {
      defaultProps: {
        sx: {
          background: (theme) => theme.palette.background.paper,
        },
      },
    },
    MuiTableCell: {
      defaultProps: {
        sx: {
          borderBottom: (theme) => `1px solid ${theme.palette.divider}`,
        },
      },
    },
    MuiDivider: {
      defaultProps: {
        sx: {
          borderBottom: (theme) => `1px solid ${theme.palette.divider}`,
        },
      },
    },
    MuiFormControlLabel: {
      defaultProps: {
        sx: {
          '& .MuiTypography-root': {
            fontSize: '0.875rem',
            lineHeight: '1.8rem',
            color: (theme) => theme.palette.text.disabled,
          },
        },
      },
    },
    MuiCircularProgress: {
      defaultProps: {
        thickness: 4,
        sx: {
          color: (theme) => theme.palette.primary.main,
        },
      },
    },
  },
};

export const light: ThemeOptions = {
  palette: {
    mode: 'light',
    primary: {
      main: tariPurple[600],
      dark: tariPurple[700],
      light: tariPurple[500],
    },
    secondary: {
      main: gothic[400],
      dark: gothic[500],
      light: teal[400],
    },
    divider: 'rgba(0,0,0,0.08)',
    text: {
      primary: grey[950],
      secondary: grey[600],
      disabled: grey[400],
    },
    background: {
      default: grey[50],
      paper: '#fff',
    },
    success: {
      main: green[500],
      dark: green[600],
      light: green[400],
      contrastText: '#ffffff',
    },
    warning: {
      main: orange[300],
      dark: orange[400],
      light: orange[200],
      contrastText: '#ffffff',
    },
    error: {
      main: red[500],
      dark: red[600],
      light: red[400],
      contrastText: '#ffffff',
    },
    info: {
      main: blue[500],
      dark: blue[700],
      light: blue[400],
      contrastText: '#ffffff',
    },
  },
};

export const dark: ThemeOptions = {
  palette: {
    mode: 'dark',
    primary: {
      main: tariPurple[500],
      dark: tariPurple[400],
      light: tariPurple[50],
    },
    secondary: {
      main: teal[400],
      dark: teal[300],
      light: gothic[400],
    },
    divider: 'rgba(255,255,255,0.04)',
    text: {
      primary: '#FFFFFF',
      secondary: grey[300],
      disabled: 'rgba(255,255,255,0.4)',
    },
    background: {
      default: grey[950],
      paper: grey[900],
    },
    success: {
      main: green[500],
      dark: green[400],
      light: green[600],
      contrastText: '#ffffff',
    },
    warning: {
      main: orange[300],
      dark: orange[200],
      light: orange[400],
      contrastText: '#ffffff',
    },
    error: {
      main: red[500],
      dark: red[600],
      light: red[500],
      contrastText: '#ffffff',
    },
    info: {
      main: blue[500],
      dark: blue[700],
      light: blue[400],
      contrastText: '#ffffff',
    },
  },
};
