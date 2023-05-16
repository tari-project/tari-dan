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

import { useEffect, useState } from 'react';
import FormControl from '@mui/material/FormControl';
import FormGroup from '@mui/material/FormGroup';
import FormControlLabel from '@mui/material/FormControlLabel';
import FormHelperText from '@mui/material/FormHelperText';
import Switch from '@mui/material/Switch';
import Divider from '@mui/material/Divider';
import Typography from '@mui/material/Typography';
import './Permissions.css';
import { TariPermission } from '../../utils/tari_permissions';

export default function Permissions({ requiredPermissions, optionalPermissions, setOptionalPermissions }: { requiredPermissions: TariPermission[], optionalPermissions: TariPermission[], setOptionalPermissions: any }) {
  const [permissions, setPermissions] = useState(optionalPermissions.map((permission, index) => {
    return ({ id: index, name: permission.toString(), checked: true });
  }));

  const handleChange = (index: number) => {
    setPermissions(
      permissions.map((item) => {
        if (item.id === index) {
          return {
            ...item,
            checked: !item.checked,
          };
        }
        return item;
      })
    );
  };

  useEffect(() => setOptionalPermissions(permissions.map((item) => item.checked)), [permissions]);

  return (
    <>
      <Typography style={{ textAlign: 'center', marginBottom: '20px' }}>
        Select what the app is allowed to access:
      </Typography>
      <FormControl
        component="fieldset"
        variant="standard"
        style={{ width: '100%' }}
      >
        <Divider />
        <FormGroup>
          {requiredPermissions.map((permissions) => {
            return (
              <>
                <FormControlLabel
                  control={
                    <Switch
                      checked={true}
                      disabled={true}
                    />
                  }
                  label={permissions.toString()}
                  labelPlacement="start"
                  key={permissions.toString()}
                  className="permissions-switch"
                />
                <Divider />
              </>
            );
          })}
        </FormGroup>
        <FormGroup>
          {permissions.map(({ checked, name, id }) => {
            return (
              <>
                <FormControlLabel
                  control={
                    <Switch
                      checked={checked}
                      onChange={() => handleChange(id)}
                      name={name}
                      value={name}
                    />
                  }
                  label={name}
                  labelPlacement="start"
                  key={id}
                  className="permissions-switch"
                />
                <Divider />
              </>
            );
          })}
        </FormGroup>
        <FormHelperText style={{ marginBottom: '20px', marginTop: '20px' }}>
          You may be sharing sensitive information with this site. Approve or
          deny access above.
        </FormHelperText>
      </FormControl>
    </>
  );
}
