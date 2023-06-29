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

import { createTheme } from '@mui/material/styles';

const theme = createTheme({
  palette: {
    primary: {
      main: '#9330FF',
    },
    secondary: {
      main: '#40388A',
    },
  },
  shape: {
    borderRadius: 10,
  },
  typography: {
    fontFamily: '"AvenirMedium", sans-serif',
    body1: {
      color: '#000000',
    },
    body2: {
      color: '#000000',
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
        disableRipple: true,
        sx: {
          minHeight: '55px',
          boxShadow: 'none',
          textTransform: 'none',
          fontSize: '1rem',
          fontWeight: 500,
          fontFamily: '"AvenirMedium", sans-serif',
          letterSpacing: '0.5px',
        },
      },
    },
    MuiTableCell: {
      defaultProps: {
        sx: {
          borderBottom: '1px solid #f5f5f5',
        },
      },
    },
    MuiDivider: {
      defaultProps: {
        sx: {
          borderBottom: '1px solid #f5f5f5',
        },
      },
    },
    MuiFormControlLabel: {
      defaultProps: {
        sx: {
          '& .MuiTypography-root': {
            fontSize: '0.875rem',
            lineHeight: '1.8rem',
            color: 'rgba(0, 0, 0, 0.6)',
          },
        },
      },
    },
    MuiCircularProgress: {
      defaultProps: {
        // size: 20,
        thickness: 4,
        sx: {
          color: '#EAEAEA',
        },
      },
    },
  },
});

export default theme;
