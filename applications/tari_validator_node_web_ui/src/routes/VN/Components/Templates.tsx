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
import { ITemplate } from '../../../utils/interfaces';
import { getTemplate, getTemplates } from '../../../utils/json_rpc';
import { shortenString } from './helpers';
import './Templates.css';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import { DataTableCell } from '../../../Components/StyledComponents';
import { Link } from 'react-router-dom';
import CopyToClipboard from '../../../Components/CopyToClipboard';
import IconButton from '@mui/material/IconButton';
import KeyboardArrowRightIcon from '@mui/icons-material/KeyboardArrowRight';

function Templates() {
  const [templates, setTemplates] = useState([]);
  useEffect(() => {
    getTemplates(10).then((response) => {
      setTemplates(response.templates);
    });
  }, []);
  const toHex = (str: Uint8Array) => {
    return (
      '0x' +
      Array.prototype.map
        .call(str, (x: number) => ('00' + x.toString(16)).slice(-2))
        .join('')
    );
  };
  return (
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell>Address</TableCell>
            <TableCell>Download URL</TableCell>
            <TableCell style={{ textAlign: 'center' }}>Mined Height</TableCell>
            <TableCell style={{ textAlign: 'center' }}>Status</TableCell>
            <TableCell style={{ textAlign: 'center' }}>Functions</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {templates.map(({ address, binary_sha, height, url }) => (
            <TableRow key={address}>
              <DataTableCell>
                <Link
                  to={`/template/${toHex(address)}`}
                  state={[address]}
                  style={{ textDecoration: 'none' }}
                >
                  {shortenString(toHex(address))}
                </Link>
                <CopyToClipboard copy={toHex(address)} />
              </DataTableCell>
              <DataTableCell>
                <a href={url}>{url}</a>
              </DataTableCell>
              <DataTableCell style={{ textAlign: 'center' }}>
                {height}
              </DataTableCell>
              <DataTableCell style={{ textAlign: 'center' }}>
                Active
              </DataTableCell>
              <DataTableCell style={{ textAlign: 'center' }}>
                <Link to={`/template/${toHex(address)}`} state={[address]}>
                  <IconButton>
                    <KeyboardArrowRightIcon color="primary" />
                  </IconButton>
                </Link>
              </DataTableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}

export default Templates;
