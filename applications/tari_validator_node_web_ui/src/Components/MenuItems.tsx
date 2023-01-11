import * as React from 'react';
import ListItemButton from '@mui/material/ListItemButton';
import ListItemIcon from '@mui/material/ListItemIcon';
import ListItemText from '@mui/material/ListItemText';
import PeopleOutlinedIcon from '@mui/icons-material/PeopleOutlined';
import WebhookOutlinedIcon from '@mui/icons-material/WebhookOutlined';
import AssessmentOutlinedIcon from '@mui/icons-material/AssessmentOutlined';
import AccountTreeOutlinedIcon from '@mui/icons-material/AccountTreeOutlined';
import CopyAllOutlinedIcon from '@mui/icons-material/CopyAllOutlined';
import AttractionsOutlinedIcon from '@mui/icons-material/AttractionsOutlined';
import CottageOutlinedIcon from '@mui/icons-material/CottageOutlined';
import AddchartIcon from '@mui/icons-material/Addchart';
import { Link } from 'react-router-dom';

import Tooltip from '@mui/material/Tooltip';
import Fade from '@mui/material/Fade';

// const [selectedIndex, setSelectedIndex] = React.useState(1);

// const handleListItemClick = (
//   event: React.MouseEvent<HTMLDivElement, MouseEvent>,
//   index: number
// ) => {
//   setSelectedIndex(index);
// };

const mainItems = [
  {
    title: 'Home',
    icon: <CottageOutlinedIcon />,
    link: '/',
  },
  {
    title: 'Committees',
    icon: <PeopleOutlinedIcon />,
    link: 'committees',
  },
  {
    title: 'Connections',
    icon: <WebhookOutlinedIcon />,
    link: 'connections',
  },
  {
    title: 'Mempool',
    icon: <AddchartIcon />,
    link: 'mempool',
  },
  {
    title: 'Recent Transactions',
    icon: <AssessmentOutlinedIcon />,
    link: 'transactions',
  },
  {
    title: 'Templates',
    icon: <CopyAllOutlinedIcon />,
    link: 'templates',
  },
  {
    title: 'Validator Nodes',
    icon: <AccountTreeOutlinedIcon />,
    link: 'vns',
  },
  // {
  //   title: 'Playground',
  //   icon: <AttractionsOutlinedIcon />,
  //   link: 'playground',
  // },
];

const mainMenu = mainItems.map((data) => (
  <Link to={data.link} key={data.title} style={{ textDecoration: 'none' }}>
    <ListItemButton
      sx={{ paddingLeft: '22px', paddingRight: '22px' }}
      disableRipple
      // selected
      // onClick={(event) => handleListItemClick(event, 1)}
    >
      <Tooltip
        TransitionComponent={Fade}
        TransitionProps={{ timeout: 300 }}
        title={data.title}
        followCursor={true}
      >
        <ListItemIcon>{data.icon}</ListItemIcon>
      </Tooltip>
      <ListItemText primary={data.title} />
    </ListItemButton>
  </Link>
));

export const mainListItems = <React.Fragment>{mainMenu}</React.Fragment>;
