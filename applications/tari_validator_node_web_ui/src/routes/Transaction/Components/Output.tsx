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
import { toHexString, shortenString } from '../../VN/Components/helpers';
import { renderJson } from '../../../utils/helpers';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import {
  DataTableCell,
  CodeBlock,
  AccordionIconButton,
  BoxHeading,
} from '../../../Components/StyledComponents';
import KeyboardArrowDownIcon from '@mui/icons-material/KeyboardArrowDown';
import KeyboardArrowUpIcon from '@mui/icons-material/KeyboardArrowUp';
import Collapse from '@mui/material/Collapse';
import CopyToClipboard from '../../../Components/CopyToClipboard';
import CommitOutlinedIcon from '@mui/icons-material/CommitOutlined';

function RowData({ row, justify }: any) {
  const [open, setOpen] = useState(false);

  return (
    <>
      <TableRow
        key={toHexString(row.node_hash)}
        style={{ verticalAlign: 'top', borderBottom: 'none' }}
      >
        <DataTableCell style={{ borderBottom: 'none' }}>
          {row.height}
        </DataTableCell>
        <DataTableCell style={{ borderBottom: 'none' }} className="key">
          {shortenString(toHexString(row.node_hash))}
          <CopyToClipboard copy={toHexString(row.node_hash)} />
        </DataTableCell>
        <TableCell style={{ borderBottom: 'none', padding: 0 }}>
          <TableContainer>
            <Table>
              <TableHead>
                <TableRow>
                  <TableCell>Shard</TableCell>
                  <TableCell>Current state</TableCell>
                  <TableCell>Pledged to</TableCell>
                  <TableCell>Proposed by</TableCell>
                  <TableCell>Leader round</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {Array.isArray(justify.all_shard_pledges?.pledges) ? (
                  justify.all_shard_pledges.pledges.map((pledge: any) => {
                    // This enum gets serialized different ways... should be fixed in the rust
                    let currentState = Object.keys(pledge.pledge.current_state);
                    return (
                      <TableRow
                        key={pledge.shard_id}
                        sx={{ borderBottom: 'none' }}
                      >
                        <DataTableCell>
                          {shortenString(pledge.shard_id)}
                          <CopyToClipboard copy={pledge.shard_id} />
                        </DataTableCell>
                        <DataTableCell>
                          {currentState[0] !== '0'
                            ? currentState[0]
                            : pledge.pledge.current_state}
                        </DataTableCell>
                        <DataTableCell>
                          {shortenString(pledge.pledge.pledged_to_payload)}
                          <CopyToClipboard
                            copy={pledge.pledge.pledged_to_payload}
                          />
                        </DataTableCell>
                        <DataTableCell>
                          {row.proposed_by}
                        </DataTableCell>
                        <DataTableCell>
                          {row.leader_round}
                        </DataTableCell>
                      </TableRow>
                    );
                  })
                ) : (
                  <TableRow>
                    <DataTableCell colSpan={3} style={{ borderBottom: 'none' }}>
                      No pledges
                    </DataTableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </TableContainer>
        </TableCell>
        <TableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <AccordionIconButton
            open={open}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen(!open);
            }}
          >
            {open ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </TableCell>
      </TableRow>
      <TableRow>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
          }}
          colSpan={4}
        >
          <Collapse in={open} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: '10px' }}>
              {row.justify ? renderJson(JSON.parse(row.justify)) : ''}
            </CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

export default function Output({
  shard,
  output,
  current_state,
}: {
  shard: string;
  output: any[];
  current_state: [string,number,string] | undefined;
}) {
  return (
    <div id={shard} className="output">
      <TableContainer>
        <BoxHeading
          style={{
            marginBottom: '20px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'flex-start',
            gap: '10px',
          }}
        >
          <CommitOutlinedIcon style={{ color: 'rgba(35, 11, 73, 0.20)' }} />
          Shard: {shard}
          <br/>
          Current leader : {current_state?current_state[0]:"Unknown"}
          <br/>
          Leader round : {current_state?current_state[1]:"Unknown"}
          <br/>
          Leader timestamp : {current_state?new Date(current_state[2]).toLocaleString():"Unknown"}
        </BoxHeading>
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
              return <RowData row={row} justify={justify} />;
            })}
          </TableBody>
        </Table>
      </TableContainer>
    </div>
  );
}
