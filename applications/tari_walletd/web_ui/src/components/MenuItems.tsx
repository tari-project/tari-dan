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

import ThemeSwitcher from "@components/ThemeSwitcher";
import Box from "@mui/material/Box";
import Fade from "@mui/material/Fade";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemIcon from "@mui/material/ListItemIcon";
import ListItemText from "@mui/material/ListItemText";
import { useTheme } from "@mui/material/styles";
import Tooltip from "@mui/material/Tooltip";
import useSettingsStore from "@store/settingsStore";
import {
  IoHome,
  IoHomeOutline,
  IoReceipt,
  IoReceiptOutline,
  IoSettings,
  IoSettingsOutline,
  IoTerminal,
  IoTerminalOutline,
} from "react-icons/io5";
import { LuBookOpen, LuCircleCheck, LuKeyRound, LuLayoutTemplate, LuShieldCheck, LuSignature } from "react-icons/lu";
import { NavLink } from "react-router-dom";

function MainListItems() {
  const theme = useTheme();
  const advancedUiFeatures = useSettingsStore((s) => s.advancedUiFeatures);

  const iconStyle = {
    height: 22,
    width: 22,
  };

  const activeIconStyle = {
    height: 22,
    width: 22,
    color: theme.palette.primary.main,
  };

  const mainItems = [
    {
      title: "Home",
      icon: <IoHomeOutline style={iconStyle} />,
      activeIcon: <IoHome style={activeIconStyle} />,
      link: "/",
    },
    {
      title: "Transactions",
      icon: <IoReceiptOutline style={iconStyle} />,
      activeIcon: <IoReceipt style={activeIconStyle} />,
      link: "transactions",
    },
    {
      title: "Templates",
      icon: <LuLayoutTemplate style={iconStyle} />,
      activeIcon: <LuLayoutTemplate style={activeIconStyle} />,
      link: "templates",
    },

    {
      title: "Address Book",
      icon: <LuBookOpen style={iconStyle} />,
      activeIcon: <LuBookOpen style={activeIconStyle} />,
      link: "address-book",
    },
    {
      title: "Requests",
      icon: <LuCircleCheck style={iconStyle} />,
      activeIcon: <LuCircleCheck style={activeIconStyle} />,
      link: "transaction-requests",
    },
    {
      title: "Signing",
      icon: <LuSignature style={iconStyle} />,
      activeIcon: <LuSignature style={activeIconStyle} />,
      link: "signing-requests",
    },
    {
      title: "API Keys",
      icon: <LuKeyRound style={iconStyle} />,
      activeIcon: <LuKeyRound style={activeIconStyle} />,
      link: "api-keys",
    },
    {
      title: "Settings",
      icon: <IoSettingsOutline style={iconStyle} />,
      activeIcon: <IoSettings style={activeIconStyle} />,
      link: "settings",
    },
  ];

  if (advancedUiFeatures.enable_manifest) {
    mainItems.push({
      title: "Manifest",
      icon: <IoTerminalOutline style={iconStyle} />,
      activeIcon: <IoTerminal style={activeIconStyle} />,
      link: "manifest",
    });
  }
  if (advancedUiFeatures.enable_validator_set) {
    mainItems.push({
      title: "Validator Set",
      icon: <LuShieldCheck style={iconStyle} />,
      activeIcon: <LuShieldCheck style={activeIconStyle} />,
      link: "validator-set",
    });
  }
  // if (advancedUiFeatures.enable_flow_editor) {
  //   mainItems.push({
  //     title: "Flow Editor",
  //     icon: <IoGitMerge style={iconStyle} />,
  //     activeIcon: <IoGitMerge style={activeIconStyle} />,
  //     link: "flow-editor",
  //   });
  // }

  return (
    <Box
      style={{
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        height: "calc(100vh - 84px)",
      }}
    >
      <Box>
        {mainItems.map(({ title, icon, activeIcon, link }) => (
          <NavLink
            to={link}
            key={title}
            style={{
              textDecoration: "none",
              color: "inherit",
            }}
          >
            {({ isActive }) => (
              <ListItemButton
                sx={{
                  paddingLeft: "22px",
                  paddingRight: "22px",
                }}
              >
                <Tooltip
                  TransitionComponent={Fade}
                  TransitionProps={{ timeout: 300 }}
                  title={title}
                  followCursor={false}
                  placement="right"
                  arrow
                >
                  <ListItemIcon>{isActive ? activeIcon : icon}</ListItemIcon>
                </Tooltip>
                <ListItemText secondary={title} />
              </ListItemButton>
            )}
          </NavLink>
        ))}
      </Box>
      <ThemeSwitcher />
    </Box>
  );
}

export default MainListItems;
