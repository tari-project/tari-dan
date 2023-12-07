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

import TextField from '@mui/material/TextField';
import { useState } from 'react';
import { Form } from 'react-router-dom';
import Alert from '@mui/material/Alert';
import Button from '@mui/material/Button';
import Box from '@mui/material/Box';
import Typography from '@mui/material/Typography';
import Slide from '@mui/material/Slide';
import useAccountStore from '../../../store/accountStore';
import { useTheme } from '@mui/material/styles';
import { useEffect } from 'react';
import { getSettings, setSettings as uploadSettings } from '../../../utils/json_rpc';

function IndexerSettings() {
  // Keep the form and settings in the same format as the real settings in the dan wallet.
  const [accountFormState, setAccountFormState] = useState({
    indexer_url: '',
  });
  const theme = useTheme();
  const [showForm, setShowForm] = useState(false);
  const [settings, setSettings] = useState({ indexer_url: '' });

  useEffect(() => {
    getSettings().then((res) => {
      setSettings(res);
    });
  }, []);

  const onSubmitIndexer = () => {
    setSettings(accountFormState);
    uploadSettings(accountFormState);
    setShowForm(false);
    setAccountFormState({ ...accountFormState, indexer_url: '' })
  };

  const onAccountChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    e.preventDefault();
    setAccountFormState({
      ...accountFormState,
      [e.target.name]: e.target.value,
    });
  };

  return (
    <>
      <Box className="flex-container">
        {showForm ? (
          <Form onSubmit={onSubmitIndexer} className="flex-container">
            <TextField
              name="indexer_url"
              label="Indexer url"
              value={accountFormState.indexer_url}
              onChange={onAccountChange}
              style={{ flexGrow: 1 }}
            />
            <Button variant="contained" type="submit">
              Set Indexer
            </Button>
            <Button variant="outlined" onClick={() => setShowForm(false)}>
              Cancel
            </Button>
          </Form>
        ) : (
          <Box
            className="flex-container"
            style={{
              justifyContent: 'space-between',
              alignItems: 'center',
            }}
          >
            {settings.indexer_url === '' ? (
              <Alert severity="warning" style={{ width: '100%' }}>
                No Indexer Set
              </Alert>
            ) : (
              <Typography variant="body2">{settings.indexer_url}</Typography>
            )}
            <Button
              variant="outlined"
              onClick={() => { setAccountFormState(settings); setShowForm(true); }}
              style={{ minWidth: '130px', height: '56px' }}
            >
              Set new url
            </Button>
          </Box>
        )}
      </Box>
    </>
  );
}

export default IndexerSettings;
