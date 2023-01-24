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
import JsonTooltip from '../../../Components/JsonTooltip';
import { renderJson } from '../../../utils/helpers';
import { toHexString } from '../../VN/Components/helpers';
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
} from '../../../Components/StyledComponents';
import IconButton from '@mui/material/IconButton';
import KeyboardArrowDownIcon from '@mui/icons-material/KeyboardArrowDown';
import KeyboardArrowUpIcon from '@mui/icons-material/KeyboardArrowUp';
import Collapse from '@mui/material/Collapse';
import SecondaryHeading from '../../../Components/SecondaryHeading';

function RowData({ substate }: any) {
  const [open1, setOpen1] = useState(false);
  const [open2, setOpen2] = useState(false);
  const [open3, setOpen3] = useState(false);

  return (
    <>
      <TableRow>
        <DataTableCell sx={{ borderBottom: 'none' }}>
          {toHexString(substate.shard_id)}
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <AccordionIconButton
            open={open1}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen1(!open1);
              setOpen2(false);
              setOpen3(false);
            }}
          >
            {open1 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <AccordionIconButton
            open={open2}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen1(false);
              setOpen2(!open2);
              setOpen3(false);
            }}
          >
            {open2 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <AccordionIconButton
            open={open3}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen1(false);
              setOpen2(false);
              setOpen3(!open3);
            }}
          >
            {open3 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </DataTableCell>
      </TableRow>
      <TableRow>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
            borderBottom: 'none',
          }}
          colSpan={4}
        >
          <Collapse in={open1} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: '10px' }}>
              <pre>{renderJson(JSON.parse(substate.data))}</pre>
            </CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
      <TableRow>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
            borderBottom: 'none',
          }}
          colSpan={4}
        >
          <Collapse in={open2} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: '10px' }}>
              <pre>
                {substate.created_justify
                  ? renderJson(JSON.parse(substate.created_justify))
                  : ''}
              </pre>
            </CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
      <TableRow>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
          }}
          colSpan={4}
        >
          <Collapse in={open3} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: '10px' }}>
              <pre>
                {substate.destroyed_justify
                  ? renderJson(JSON.parse(substate.destroyed_justify))
                  : ''}
              </pre>
            </CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

export default function Substates({ substates }: any) {
  if (substates.size == 0) {
    return <div className="caption">No substates</div>;
  }
  console.log(substates);
  substates.map((substate: any) => {
    // console.log("parsing json", substate.justify, JSON.parse(substate.justify));
  });
  return (
    <TableContainer>
      <SecondaryHeading>Substates</SecondaryHeading>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell>Shard</TableCell>
            <TableCell sx={{ textAlign: 'center', width: '120px' }}>
              Data
            </TableCell>
            <TableCell sx={{ textAlign: 'center', width: '120px' }}>
              Created
            </TableCell>
            <TableCell sx={{ textAlign: 'center', width: '120px' }}>
              Destroyed
            </TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {substates.map((substate: any) => (
            <RowData substate={substate} />
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
