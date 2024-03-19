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

import { useState } from "react";
import Button from "@mui/material/Button";
import Menu from "@mui/material/Menu";
import MenuItem from "@mui/material/MenuItem";
import UnfoldMoreIcon from "@mui/icons-material/UnfoldMore";
import ListItemText from "@mui/material/ListItemText";
import ListItemIcon from "@mui/material/ListItemIcon";
import MenuList from "@mui/material/MenuList";
import FilterListOutlinedIcon from "@mui/icons-material/FilterListOutlined";
import IconButton from "@mui/material/IconButton";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";

interface IMenuItem {
  title: string;
  fn: () => void;
  icon?: any;
}

interface Props {
  menuTitle: string;
  menuItems?: IMenuItem[];
  showArrow?: boolean;
  lastSort?: any;
  columnName?: string;
  sortFunction?: any;
}

function HeadingMenu({ menuTitle, menuItems, showArrow, lastSort, columnName, sortFunction }: Props) {
  const [anchorEl, setAnchorEl] = useState(null);

  function handleClick(event: any) {
    if (anchorEl !== event.currentTarget) {
      setAnchorEl(event.currentTarget);
    }
  }

  function handleClose() {
    setAnchorEl(null);
  }

  return (
    <div className="heading-menu">
      <>
        <Button
          aria-owns={anchorEl ? "simple-menu" : undefined}
          aria-haspopup="true"
          onClick={handleClick}
          // onMouseOver={handleClick}
          startIcon={showArrow && <UnfoldMoreIcon />}
          style={{
            textTransform: "none",
            color: "#000000",
          }}
          disabled={!showArrow}
        >
          {menuTitle}
        </Button>
        <Menu
          id="simple-menu"
          anchorEl={anchorEl}
          open={Boolean(anchorEl)}
          onClose={handleClose}
          MenuListProps={{ onMouseLeave: handleClose }}
        >
          <MenuList style={{ outline: "none" }} className="annoying">
            {menuItems?.map((item, index) => (
              <MenuItem key={index} onClick={item.fn}>
                <ListItemIcon>{item.icon ? item.icon : <FilterListOutlinedIcon fontSize="small" />}</ListItemIcon>
                <ListItemText>{item.title}</ListItemText>
              </MenuItem>
            ))}
          </MenuList>
        </Menu>
      </>
      {lastSort && (
        <IconButton>
          {lastSort.column === columnName ? (
            lastSort.order === 1 ? (
              <KeyboardArrowUpIcon onClick={() => sortFunction(columnName, -1)} />
            ) : (
              <KeyboardArrowDownIcon onClick={() => sortFunction(columnName, 1)} />
            )
          ) : (
            ""
          )}
        </IconButton>
      )}
    </div>
  );
}

export default HeadingMenu;
