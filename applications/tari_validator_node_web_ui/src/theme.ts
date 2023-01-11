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
        variant: 'contained',
        color: 'primary',
        disableRipple: true,
        style: {
          fontFamily: '"AvenirHeavy", sans-serif',
          padding: '5px 15px',
        },
      },
    },
    MuiPaper: {
      defaultProps: {
        style: {
          //   boxShadow: '10px 14px 28px rgba(35, 11, 73, 0.05)',
        },
      },
    },
  },
});

export default theme;
