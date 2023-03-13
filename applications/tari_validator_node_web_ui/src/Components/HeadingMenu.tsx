import React from 'react';
import Button from '@mui/material/Button';
import Menu from '@mui/material/Menu';
import MenuItem from '@mui/material/MenuItem';
import UnfoldMoreIcon from '@mui/icons-material/UnfoldMore';
import ListItemText from '@mui/material/ListItemText';
import ListItemIcon from '@mui/material/ListItemIcon';
import MenuList from '@mui/material/MenuList';
import FilterListOutlinedIcon from '@mui/icons-material/FilterListOutlined';

interface MenuItem {
  title: string;
  fn: () => void;
  icon?: any;
}

interface Props {
  menuTitle: string;
  menuItems: MenuItem[];
}

function HeadingMenu({ menuTitle, menuItems }: Props) {
  const [anchorEl, setAnchorEl] = React.useState(null);

  function handleClick(event: any) {
    if (anchorEl !== event.currentTarget) {
      setAnchorEl(event.currentTarget);
    }
  }

  function handleClose() {
    setAnchorEl(null);
  }

  return (
    <div>
      <Button
        aria-owns={anchorEl ? 'simple-menu' : undefined}
        aria-haspopup="true"
        onClick={handleClick}
        // onMouseOver={handleClick}
        startIcon={<UnfoldMoreIcon />}
        style={{
          textTransform: 'none',
          color: '#000000',
        }}
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
        <MenuList style={{ outline: 'none' }} className="annoying">
          {menuItems?.map((item, index) => (
            <MenuItem key={index} onClick={item.fn}>
              <ListItemIcon>
                {item.icon ? (
                  item.icon
                ) : (
                  <FilterListOutlinedIcon fontSize="small" />
                )}
              </ListItemIcon>
              <ListItemText>{item.title}</ListItemText>
            </MenuItem>
          ))}
        </MenuList>
      </Menu>
    </div>
  );
}

export default HeadingMenu;
