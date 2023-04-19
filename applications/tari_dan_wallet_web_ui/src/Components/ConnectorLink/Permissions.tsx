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
import Box from '@mui/material/Box';
import FormLabel from '@mui/material/FormLabel';
import FormControl from '@mui/material/FormControl';
import FormGroup from '@mui/material/FormGroup';
import FormControlLabel from '@mui/material/FormControlLabel';
import FormHelperText from '@mui/material/FormHelperText';
import Checkbox from '@mui/material/Checkbox';

export default function Permissions() {
  const [permissions, setPermissions] = React.useState([
    {
      id: 1,
      name: 'Choose an identity (account public key) to log in with',
      checked: true,
    },
    {
      id: 2,
      name: 'Choose an NFT as a profile picture',
      checked: true,
    },
    {
      id: 3,
      name: 'Read all NFTs of a specific contract (in your wallet)',
      checked: true,
    },
    {
      id: 4,
      name: 'Generate a proof of ownership of NFT',
      checked: true,
    },
    {
      id: 5,
      name: 'Send funds',
      checked: true,
    },
    {
      id: 6,
      name: 'Invoke generic method',
      checked: true,
    },
  ]);

  const handleChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    setPermissions(
      permissions.map((item) => {
        if (item.name === event.target.name) {
          return {
            ...item,
            checked: event.target.checked,
          };
        }
        return item;
      })
    );
  };

  return (
    <Box sx={{ display: 'flex' }}>
      <FormControl sx={{ m: 3 }} component="fieldset" variant="standard">
        <FormLabel component="legend">This app would like to:</FormLabel>
        <FormGroup>
          {permissions.map(({ checked, name }) => {
            return (
              <FormControlLabel
                control={
                  <Checkbox
                    checked={checked}
                    onChange={handleChange}
                    name={name}
                  />
                }
                label={name}
              />
            );
          })}
        </FormGroup>
        <FormHelperText>Approve or deny permissions</FormHelperText>
      </FormControl>
    </Box>
  );
}
