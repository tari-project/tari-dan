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
import Error from './Error';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import { keysCreate, keysList, keysSetActive } from '../../../utils/json_rpc';
import { BoxHeading2 } from '../../../Components/StyledComponents';
import AddIcon from '@mui/icons-material/Add';
import Fade from '@mui/material/Fade';
import { Form } from 'react-router-dom';
import Button from '@mui/material/Button/Button';
import { DataTableCell } from '../../../Components/StyledComponents';
import { Alert } from '@mui/material';

function Key(key: any, setActive: any) {
  return (
    <TableRow key={key[0]}>
      <DataTableCell>{key[0]}</DataTableCell>
      <DataTableCell>{key[1]}</DataTableCell>
      <DataTableCell>
        {key[2] ? (
          <b>Active</b>
        ) : (
          <div onClick={() => setActive(key[0])}>Activate</div>
        )}
      </DataTableCell>
    </TableRow>
  );
}

function Keys() {
  const [state, setState] = useState<any>();
  const [error, setError] = useState<String>();
  const [showKeyDialog, setShowAddKeyDialog] = useState(false);
  const [formState, setFormState] = useState({ publicKey: '', address: '' });

  const showAddKeyDialog = (setElseToggle: boolean = !showKeyDialog) => {
    setShowAddKeyDialog(setElseToggle);
  };
  const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFormState({ ...formState, [e.target.name]: e.target.value });
  };
  const loadKeys = () => {
    keysList()
      .then((response) => {
        console.log(response);
        setState(response);
        setError(undefined);
      })
      .catch((err) => {
        setError(
          err && err.message
            ? err.message
            : `Unknown error: ${JSON.stringify(err)}`
        );
      });
  };
  const setActive = (index: number) => {
    keysSetActive(index).then((response) => {
      loadKeys();
    });
    console.log('click');
  };
  const onSubmitAddKey = () => {
    keysCreate().then((response) => {
      loadKeys();
    });
    setFormState({ publicKey: '', address: '' });
    setShowAddKeyDialog(false);
  };
  useEffect(() => {
    loadKeys();
  }, []);
  if (error) {
    return <Alert severity="error">{error}</Alert>;
  }
  return (
    <>
      <BoxHeading2>
        {showKeyDialog && (
          <Fade in={showKeyDialog}>
            <Form onSubmit={onSubmitAddKey} className="flex-container">
              <Button variant="contained" type="submit">
                Add Key
              </Button>
              <Button
                variant="outlined"
                onClick={() => showAddKeyDialog(false)}
              >
                Cancel
              </Button>
            </Form>
          </Fade>
        )}
        {!showKeyDialog && (
          <Fade in={!showKeyDialog}>
            <div className="flex-container">
              <Button
                variant="outlined"
                startIcon={<AddIcon />}
                onClick={() => showAddKeyDialog()}
              >
                Add Key
              </Button>
            </div>
          </Fade>
        )}
      </BoxHeading2>{' '}
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>Index</TableCell>
              <TableCell>Public key</TableCell>
              <TableCell>Active</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {state && state.keys.map((key: any) => Key(key, setActive))}
          </TableBody>
        </Table>
      </TableContainer>
    </>
  );
}

export default Keys;
