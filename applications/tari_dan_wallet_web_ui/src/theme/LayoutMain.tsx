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

import React, {useEffect, useMemo, useRef, useState} from "react";
import {styled} from "@mui/material/styles";
import CssBaseline from "@mui/material/CssBaseline";
import MuiDrawer from "@mui/material/Drawer";
import Box from "@mui/material/Box";
import MuiAppBar, {AppBarProps as MuiAppBarProps} from "@mui/material/AppBar";
import Toolbar from "@mui/material/Toolbar";
import List from "@mui/material/List";
import IconButton from "@mui/material/IconButton";
import MenuOpenOutlinedIcon from "@mui/icons-material/MenuOpenOutlined";
import MenuOutlinedIcon from "@mui/icons-material/MenuOutlined";
import {mainListItems} from "../Components/MenuItems";
import {ThemeProvider} from "@mui/material";
import theme from "./theme";
import {Outlet, Link} from "react-router-dom";
import Logo from "../assets/Logo";
import Container from "@mui/material/Container";
import ConnectorLink from "../Components/ConnectorLink";
import Breadcrumbs from "../Components/Breadcrumbs";
import {breadcrumbRoutes} from "../App";
import Grid from "@mui/material/Grid";
import {
  acceptPendingRequest,
  authenticate,
  denyPendingRequest,
  getPendingRequest,
  getPendingRequestsCount
} from "../utils/json_rpc";
import Button from "@mui/material/Button";
import DialogContentText from "@mui/material/DialogContentText";
import TextField from "@mui/material/TextField";
import DialogContent from "@mui/material/DialogContent";
import ConnectorLogo from "../Components/ConnectorLink/ConnectorLogo";
import CloseIcon from "@mui/icons-material/Close";

const drawerWidth = 300;

interface AppBarProps extends MuiAppBarProps {
  open?: boolean;
}

const AppBar = styled(MuiAppBar, {
  shouldForwardProp: (prop) => prop !== "open",
})<AppBarProps>(({theme, open}) => ({
  zIndex: theme.zIndex.drawer + 1,
  transition: theme.transitions.create(["width", "margin"], {
    easing: theme.transitions.easing.easeOut,
    duration: theme.transitions.duration.enteringScreen,
  }),
  ...(open && {
    marginLeft: drawerWidth,
    width: `calc(100% - ${drawerWidth}px)`,
    transition: theme.transitions.create(["width", "margin"], {
      easing: theme.transitions.easing.easeOut,
      duration: theme.transitions.duration.enteringScreen,
    }),
  }),
}));

const Drawer = styled(MuiDrawer, {
  shouldForwardProp: (prop) => prop !== "open",
})(({theme, open}) => ({
  "& .MuiDrawer-paper": {
    position: "relative",
    whiteSpace: "nowrap",
    borderRight: "1px solid #F5F5F5",
    boxShadow: "10px 14px 28px rgb(35 11 73 / 5%)",
    width: drawerWidth,
    transition: theme.transitions.create("width", {
      easing: theme.transitions.easing.easeOut,
      duration: theme.transitions.duration.enteringScreen,
    }),
    boxSizing: "border-box",
    ...(!open && {
      overflowX: "hidden",
      transition: theme.transitions.create("width", {
        easing: theme.transitions.easing.easeOut,
        duration: theme.transitions.duration.leavingScreen,
      }),
      width: theme.spacing(7),
      [theme.breakpoints.up("sm")]: {
        width: theme.spacing(9),
      },
    }),
  },
}));

interface Request {
  id: number,
  website_name: string,
  method: string,
  params: any[],
}

const GrayOverlay = (minimized: boolean) => (
  <div className={minimized ? "gray-overlay hide-overlay" : "gray-overlay show-overlay"}
  >
  </div>);

export default function Layout(effect: React.EffectCallback, deps?: React.DependencyList) {
  const [open, setOpen] = useState(false);
  const [requestCnt, setRequestCnt] = useState(0);
  const [request, setRequest] = useState<Request | null>(null);
  const [minimized, setMinimized] = useState(false);
  const [counter, setCounter] = useState(0);
  const [token, setToken] = useState(null);
  const [error, setError] = useState(null);
  const counterRef = useRef(counter);
  const requestRef = useRef(request);
  const passwordRef = useRef("");
  const handleConnect = () => {
    authenticate(passwordRef.current.value).then((token) => {
      setToken(token);
    }).catch((e) => {
      setError(e);
    });
  };
  console.log(error);
  console.log(token);
  useEffect(() => {
    const fetchData = async () => {
      let resp = await getPendingRequestsCount();
      setRequestCnt(resp?.pending_requests_count || 0);
      if (resp?.pending_requests_count > 0) {
        let current_request = await getPendingRequest();
        if (JSON.stringify(current_request) !== JSON.stringify(requestRef.current)) {
          setRequest(current_request);

          const countdown = () => {
            if (counterRef.current == 1) {
              clearInterval(countdown_timer);
            }
            setCounter((counter) => counter - 1);
          };
          setCounter(3);
          const countdown_timer = setInterval(countdown, 1000);
        }
      } else {
        setRequest(null);
      }
    };
    if (token !== null) {
      const timer = setInterval(fetchData, 1000);
      return () => {
        clearInterval(timer);
      };
    }
  }, [token]);

  if (token === null) {
    return (<>
        <div className="dialog-heading">
          <div style={{height: "24px", width: "24px"}}></div>
          <Logo/>
        </div>
        <DialogContent>
          <div className="dialog-inner"><DialogContentText>Password</DialogContentText>
            <form
              style={{
                marginTop: "1rem",
                display: "flex",
                flexDirection: "column",
                gap: "1rem",
              }}
            >
              <TextField
                name="link"
                label="Enter password"
                inputRef={passwordRef}
                fullWidth
              />
              <div className="dialog-actions">
                <Button variant="contained" onClick={handleConnect}>
                  Connect
                </Button>
              </div>
            </form>
          </div>
        </DialogContent></>
    );
  }
  ;
  counterRef.current = counter;
  requestRef.current = request;

  const Accept = () => {
    if (request) {
      setRequest(null);
      acceptPendingRequest(request.id);
    }
  };
  const Reject = () => {
    if (request) {
      setRequest(null);
      denyPendingRequest(request.id);
    }
  };

  const popup = (request: Request) => (<>
    {GrayOverlay(minimized)}
    <div className={minimized ? "popup minimized-popup" : "popup"} style={{}}>
      <div className={minimized ? "minimize-button notification" : "minimize-button"}
           onClick={() => setMinimized(!minimized)}>
        <div className="minimize-label">{minimized ? requestCnt : "x"}</div>
      </div>
      <div className={minimized ? "full-popup hide-full-popup" : "full-popup"}>
        <div className="fields">
          <div>From :</div>
          <div>{request.website_name}</div>
          <div>Method :</div>
          <div> {request.method}</div>
          <div>Params :</div>
          <div>{request.params}</div>
        </div>
        <div className="buttons">
          <Button variant="contained" type="submit" onClick={Accept} sx={{m: 1}}
                  disabled={counter > 0}>{counter ? `Accept(${counter})` : "Accept"}</Button>
          <Button variant="contained" type="submit" onClick={Reject} sx={{m: 1}}
                  disabled={counter > 0}>{counter ? `Reject(${counter})` : "Reject"}</Button>
        </div>
      </div>
    </div>
  </>);
  const toggleDrawer = () => {
    setOpen(!open);
  };
  return (
    <div>
      <ThemeProvider theme={theme}>
        <Box sx={{display: "flex"}}>
          <CssBaseline/>
          <AppBar
            position="absolute"
            open={open}
            color="secondary"
            elevation={0}
            sx={{
              backgroundColor: "#FFF",
              boxShadow: "10px 14px 28px rgb(35 11 73 / 5%)",
            }}
          >
            <Toolbar
              sx={{
                pr: "24px", // keep right padding when drawer closed
              }}
            >
              <IconButton
                edge="start"
                color="inherit"
                aria-label="open drawer"
                onClick={toggleDrawer}
                sx={{
                  marginRight: "36px",
                  color: "#757575",
                  ...(open && {display: "none"}),
                }}
              >
                <MenuOutlinedIcon/>
              </IconButton>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  width: "100%",
                  alignContent: "center",
                }}
              >
                <Link to="/">
                  <Logo/>
                </Link>
                <div
                  style={{
                    marginTop: "2px",
                  }}
                >
                  <ConnectorLink/>
                </div>
              </div>
            </Toolbar>
          </AppBar>
          <Drawer variant="permanent" open={open}>
            <Toolbar
              sx={{
                display: "flex",
                alignItems: "center",
                justifyContent: "flex-end",
                px: [1],
              }}
            >
              <IconButton onClick={toggleDrawer}>
                <MenuOpenOutlinedIcon/>
              </IconButton>
            </Toolbar>
            <List component="nav">{mainListItems}</List>
          </Drawer>
          <Box
            component="main"
            sx={{
              backgroundColor: (theme) =>
                theme.palette.mode === "light"
                  ? theme.palette.grey[100]
                  : theme.palette.grey[900],
              flexGrow: 1,
              height: "100vh",
              overflow: "auto",
            }}
          >
            <Toolbar/>
            <Container
              maxWidth="xl"
              style={{
                paddingTop: theme.spacing(3),
                paddingBottom: theme.spacing(5),
              }}
            >
              <Grid container spacing={3}>
                <Grid item sm={12} md={12} lg={12}>
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      borderBottom: `1px solid #EAEAEA`,
                    }}
                  >
                    <Breadcrumbs items={breadcrumbRoutes}/>
                  </div>
                </Grid>
                <Outlet/>
              </Grid>
            </Container>
          </Box>
        </Box>
      </ThemeProvider>
      {request && popup(request)}
    </div>
  );
}
