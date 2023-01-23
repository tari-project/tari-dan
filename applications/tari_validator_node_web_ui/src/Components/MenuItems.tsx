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

import * as React from 'react';
import ListItemButton from '@mui/material/ListItemButton';
import ListItemIcon from '@mui/material/ListItemIcon';
import ListItemText from '@mui/material/ListItemText';
import PeopleOutlinedIcon from '@mui/icons-material/PeopleOutlined';
import WebhookOutlinedIcon from '@mui/icons-material/WebhookOutlined';
import AssessmentOutlinedIcon from '@mui/icons-material/AssessmentOutlined';
import AccountTreeOutlinedIcon from '@mui/icons-material/AccountTreeOutlined';
import CopyAllOutlinedIcon from '@mui/icons-material/CopyAllOutlined';
import CottageOutlinedIcon from '@mui/icons-material/CottageOutlined';
import AddchartIcon from '@mui/icons-material/Addchart';
import { Link } from 'react-router-dom';

import Tooltip from '@mui/material/Tooltip';
import Fade from '@mui/material/Fade';

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
];

const mainMenu = mainItems.map((data) => (
  <Link to={data.link} key={data.title} style={{ textDecoration: 'none' }}>
    <ListItemButton
      sx={{ paddingLeft: '22px', paddingRight: '22px' }}
      disableRipple
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
