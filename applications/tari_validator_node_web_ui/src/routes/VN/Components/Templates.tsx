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
import './Templates.css';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import { DataTableCell } from '../../../Components/StyledComponents';

function Templates() {
  const [templates, setTemplates] = useState([]);
  const [info, setInfo] = useState<{ [id: string]: ITemplate }>();
  const [loading, setLoading] = useState<{ [id: string]: Boolean }>();
  useEffect(() => {
    getTemplates(10).then((response) => {
      setTemplates(response.templates);
    });
  }, []);
  const load = (address: string) => {
    if (info?.[address] || loading?.[address]) {
      return;
    }
    setLoading({ ...loading, [address]: true });
    getTemplate(address).then((response) => {
      setInfo({ ...info, [address]: response });
    });
  };
  const toHex = (str: Uint8Array) => {
    return (
      '0x' +
      Array.prototype.map
        .call(str, (x: number) => ('00' + x.toString(16)).slice(-2))
        .join('')
    );
  };
  const renderFunctions = (template: ITemplate) => {
    return (
      <TableContainer>
        <div className="caption">{template.abi.template_name}</div>
        <Table>
          <TableHead>
            <TableCell>Function</TableCell>
            <TableCell>Args</TableCell>
            <TableCell>Returns</TableCell>
          </TableHead>
          <TableBody>
            {template.abi.functions.map((fn) => (
              <TableRow>
                <DataTableCell style={{ textAlign: 'left' }}>
                  {fn.name}
                </DataTableCell>
                <DataTableCell>{fn.arguments.join(', ')}</DataTableCell>
                <DataTableCell>{fn.output}</DataTableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>
    );
  };
  return (
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell>Address</TableCell>
            <TableCell>Download URL</TableCell>
            <TableCell>Mined Height</TableCell>
            <TableCell>Status</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {templates.map(({ address, binary_sha, height, url }) => (
            <TableRow key={address}>
              <DataTableCell
                onMouseOver={() => load(address)}
                className="tooltip"
              >
                <span>{toHex(address)}</span>
                {info?.[address] !== undefined ? (
                  <span className="tooltiptext">
                    {renderFunctions(info[address])}
                  </span>
                ) : (
                  <></>
                )}
              </DataTableCell>
              <DataTableCell>
                <a href={url}>{url}</a>
              </DataTableCell>
              <DataTableCell>{height}</DataTableCell>
              <DataTableCell>Active</DataTableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}

export default Templates;
