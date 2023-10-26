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

import Typography from '@mui/material/Typography';
import TextField from '@mui/material/TextField';
import { useState } from 'react';
import { Form } from 'react-router-dom';
import Alert from '@mui/material/Alert';
import Button from '@mui/material/Button';
import Box from '@mui/material/Box';
import useAccountStore from '../../../store/accountStore';
import { useTheme } from '@mui/material/styles';
import IndexerSettings from './IndexerSettings';
import { Divider } from '@mui/material';

function GeneralSettings() {
  const theme = useTheme();
  const items = [
    {
      label: 'Indexer Url',
      content: <IndexerSettings />,
    },
  ];

  const renderItems = items.map((item) => {
    return (
      <>
        <Typography>{item.label}</Typography>
        <Box>{item.content}</Box>
        <Divider />
      </>
    );
  });

  return (
    <Box
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: theme.spacing(3),
        paddingTop: theme.spacing(3),
      }}
    >
      {renderItems}
    </Box>
  );
}

export default GeneralSettings;
