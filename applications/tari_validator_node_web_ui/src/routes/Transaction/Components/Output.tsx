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

import { useState } from 'react';
import { toHexString } from '../../VN/Components/helpers';
import { renderJson } from '../../../utils/helpers';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import { DataTableCell, CodeBlock } from '../../../Components/StyledComponents';

export default function Output({
  shard,
  output,
}: {
  shard: string;
  output: any[];
}) {
  return (
    <div id={shard} className="output">
      <TableContainer>
        <b>Shard : </b>
        <span className="key">{shard}</span>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>Height</TableCell>
              <TableCell>Node hash</TableCell>
              <TableCell>Pledges</TableCell>
              <TableCell>Justify</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {output.map((row) => {
              let justify = JSON.parse(row.justify);
              return (
                <TableRow key={toHexString(row.node_hash)}>
                  <DataTableCell>{row.height}</DataTableCell>
                  <DataTableCell className="key">
                    {toHexString(row.node_hash)}
                  </DataTableCell>
                  <TableCell>
                    <TableContainer>
                      <Table>
                        <TableHead>
                          <TableRow>
                            <TableCell>Shard</TableCell>
                            <TableCell>Current state</TableCell>
                            <TableCell>Pledged to</TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          { Array.isArray(justify.all_shard_pledges?.pledges) ? justify.all_shard_pledges.pledges.map((pledge:any) => {
                            // This enum gets serialized different ways... should be fixed in the rust
                            let currentState = Object.keys(
                              pledge.pledge.current_state
                            );
                            return (
                              <TableRow key={pledge.shard_id}>
                                <DataTableCell>{pledge.shard_id}</DataTableCell>
                                <DataTableCell>
                                  {currentState[0] !== '0'
                                    ? currentState[0]
                                    : pledge.pledge.current_state}
                                </DataTableCell>
                                <DataTableCell>
                                  {pledge.pledge.pledged_to_payload.id}
                                </DataTableCell>
                              </TableRow>
                            );
                          }) : <TableRow><DataTableCell>No pledges</DataTableCell></TableRow> }
                        </TableBody>
                      </Table>
                    </TableContainer>
                  </TableCell>
                  <TableCell>
                    <pre style={{ height: '200px', overflow: 'scroll' }}>
                      {row.justify ? renderJson(JSON.parse(row.justify)) : ''}
                    </pre>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </TableContainer>
    </div>
  );
}
