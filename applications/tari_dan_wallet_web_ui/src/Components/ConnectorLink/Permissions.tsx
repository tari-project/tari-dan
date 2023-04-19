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
