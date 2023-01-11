// import * as React from 'react';
import { useState } from 'react';
import { styled } from '@mui/material/styles';
import CssBaseline from '@mui/material/CssBaseline';
import MuiDrawer from '@mui/material/Drawer';
import Box from '@mui/material/Box';
import MuiAppBar, { AppBarProps as MuiAppBarProps } from '@mui/material/AppBar';
import Toolbar from '@mui/material/Toolbar';
import List from '@mui/material/List';
import Divider from '@mui/material/Divider';
import IconButton from '@mui/material/IconButton';
import Container from '@mui/material/Container';
import MenuOpenOutlinedIcon from '@mui/icons-material/MenuOpenOutlined';
import MenuOutlinedIcon from '@mui/icons-material/MenuOutlined';
import { mainListItems } from './Components/MenuItems';
import Copyright from './Components/Copyright';
import TariLogo from './assets/images/TariLogoBlack.svg';
import { ThemeProvider, Typography } from '@mui/material';
import theme from './theme';
import { Routes, Route, Outlet, Link } from 'react-router-dom';
import Mempool from './routes/Mempool/Mempool';
import Committees from './routes/Committees/Committees';
import ValidatorNode from './routes/VN/ValidatorNode';
import Connections from './routes/Connections/Connections';
import RecentTransactions from './routes/RecentTransactions/RecentTransactions';
import Templates from './routes/Templates/Templates';
import ValidatorNodes from './routes/ValidatorNodes/ValidatorNodes';
import Playground from './Components/Playground';
import ErrorPage from './routes/ErrorPage';

const drawerWidth: number = 300;

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

const Drawer = styled(MuiDrawer, {
  shouldForwardProp: (prop) => prop !== 'open',
})(({ theme, open }) => ({
  '& .MuiDrawer-paper': {
    position: 'relative',
    whiteSpace: 'nowrap',
    borderRight: '1px solid #F5F5F5',
    boxShadow: '10px 14px 28px rgb(35 11 73 / 5%)',
    width: drawerWidth,
    transition: theme.transitions.create('width', {
      easing: theme.transitions.easing.easeOut,
      duration: theme.transitions.duration.enteringScreen,
    }),
    boxSizing: 'border-box',
    ...(!open && {
      overflowX: 'hidden',
      transition: theme.transitions.create('width', {
        easing: theme.transitions.easing.easeOut,
        duration: theme.transitions.duration.leavingScreen,
      }),
      width: theme.spacing(7),
      [theme.breakpoints.up('sm')]: {
        width: theme.spacing(9),
      },
    }),
  },
}));

function Layout() {
  const [open, setOpen] = useState(true);
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
              // backgroundColor: '#FFF',
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
              <Box
                component="img"
                sx={{
                  width: 280,
                  padding: '10px 20px 3px 20px',
                }}
                src={TariLogo}
              />
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
          {/* <Divider /> */}
          <List component="nav">
            {mainListItems}
            {/* <Divider sx={{ my: 1 }} /> */}
          </List>
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
          {/* <Container maxWidth="lg" sx={{ mt: 4, mb: 4 }}> */}
          <div style={{ padding: theme.spacing(10) }}>
            <Outlet />
          </div>
          {/* </Container> */}
          {/* <Copyright /> */}
        </Box>
      </Box>
    </ThemeProvider>
  );
}

export default function App() {
  return (
    <div>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<ValidatorNode />} />
          <Route path="committees" element={<Committees />} />
          <Route path="connections" element={<Connections />} />
          <Route path="transactions" element={<RecentTransactions />} />
          <Route path="templates" element={<Templates />} />
          <Route path="vns" element={<ValidatorNodes />} />
          <Route path="mempool" element={<Mempool />} />
          <Route path="playground" element={<Playground />} />
          <Route path="*" element={<ErrorPage />} />
        </Route>
      </Routes>
    </div>
  );
}
