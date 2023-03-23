import {jsonRpc} from './utils/json_rpc'
import { Routes, Route, Outlet, Link } from 'react-router-dom';
import './App.css'
import Accounts from './routes/Accounts/Accounts';
import Keys from './routes/Keys/Keys';
import ErrorPage from './routes/ErrorPage';
import { ThemeProvider } from '@emotion/react';
import { useState } from 'react';
import theme from './theme';
import Box from '@mui/material/Box';
import CssBaseline from '@mui/material/CssBaseline';
import MuiAppBar, { AppBarProps as MuiAppBarProps } from '@mui/material/AppBar';
import styled from '@emotion/styled';
import Toolbar from '@mui/material/Toolbar';
import IconButton from '@mui/material/IconButton';
import MenuOutlinedIcon from '@mui/icons-material/MenuOutlined';
import Logo from './Components/Logo';
import { Container, Drawer, List } from '@mui/material';
import MenuOpenOutlinedIcon from '@mui/icons-material/MenuOpenOutlined';
import { mainListItems } from './Components/MenuItems';
import Wallet from './routes/Wallet/Wallet';

interface AppBarProps extends MuiAppBarProps {
  open?: boolean;
}
const AppBar = styled(MuiAppBar, {
  shouldForwardProp: (prop) => prop !== 'open',
})<AppBarProps>(({ theme, open }) => ({
  zIndex: theme.zIndex.drawer + 1,
  transition: theme.transitions.create(['width', 'margin'], {
    easing: theme.transitions.easing.easeOut,
    duration: theme.transitions.duration.enteringScreen,
  }),
  ...(open && {
    marginLeft: drawerWidth,
    width: `calc(100% - ${drawerWidth}px)`,
    transition: theme.transitions.create(['width', 'margin'], {
      easing: theme.transitions.easing.easeOut,
      duration: theme.transitions.duration.enteringScreen,
    }),
  }),
}));

function Layout() {
  const [open, setOpen] = useState(false);
  const toggleDrawer = () => {
    setOpen(!open);
  };
  return (
    <ThemeProvider theme={theme}>
      <Box sx={{ display: 'flex' }}>
        <CssBaseline />
        <AppBar
          position="absolute"
          open={open}
          color="secondary"
          elevation={0}
          sx={{
            backgroundColor: '#FFF',
            boxShadow: '10px 14px 28px rgb(35 11 73 / 5%)',
          }}
        >
          <Toolbar
            sx={{
              pr: '24px', // keep right padding when drawer closed
            }}
          >
            <IconButton
              edge="start"
              color="inherit"
              aria-label="open drawer"
              onClick={toggleDrawer}
              sx={{
                marginRight: '36px',
                color: '#757575',
                ...(open && { display: 'none' }),
              }}
            >
              <MenuOutlinedIcon />
            </IconButton>
            <Link to="/">
              <Logo />
            </Link>
          </Toolbar>
        </AppBar>
        <Drawer variant="permanent" open={open}>
          <Toolbar
            sx={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'flex-end',
              px: [1],
            }}
          >
            <IconButton onClick={toggleDrawer}>
              <MenuOpenOutlinedIcon />
            </IconButton>
          </Toolbar>
          <List component="nav">{mainListItems}</List>
        </Drawer>
        <Box
          component="main"
          sx={{
            backgroundColor: (theme) =>
              theme.palette.mode === 'light'
                ? theme.palette.grey[100]
                : theme.palette.grey[900],
            flexGrow: 1,
            height: '100vh',
            overflow: 'auto',
          }}
        >
          <Toolbar />
          <Container
            maxWidth="xl"
            style={{
              paddingTop: theme.spacing(5),
              paddingBottom: theme.spacing(5),
            }}
          >
            <Outlet />
          </Container>
        </Box>
      </Box>
    </ThemeProvider>
  );
}
function App() {
  return (
    <div>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<Wallet />} />
          <Route path="accounts" element={<Accounts />} />
          <Route path="keys" element={<Keys />} />
          <Route path="*" element={<ErrorPage />} />
        </Route>
      </Routes>
    </div>
  )
}

export default App
