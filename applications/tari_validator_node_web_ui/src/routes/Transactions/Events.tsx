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

import { useState } from "react";
import { TableContainer, Table, TableHead, TableRow, TableCell, TableBody, Collapse } from "@mui/material";
import { DataTableCell, AccordionIconButton } from "../../Components/StyledComponents";
import { shortenString } from "../../utils/helpers";
import CopyToClipboard from "../../Components/CopyToClipboard";
import { renderJson } from "../../utils/helpers";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import CodeBlockExpand from "../../Components/CodeBlock";
import type { Event } from "@tariproject/typescript-bindings";

function RowData({ component_address, template_address, topic, tx_hash, payload }: Event, index: number) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <TableRow key={index}>
        <DataTableCell sx={{ borderBottom: "none", textAlign: "center" }}>
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
        </DataTableCell>
        <DataTableCell>{topic}</DataTableCell>
        {component_address && (
          <DataTableCell>
            {shortenString(component_address)}
            <CopyToClipboard copy={component_address} />
          </DataTableCell>
        )}
        <DataTableCell>
          {shortenString(template_address)}
          <CopyToClipboard copy={template_address} />
        </DataTableCell>
        <DataTableCell>
          {shortenString(tx_hash)}
          <CopyToClipboard copy={tx_hash} />
        </DataTableCell>
      </TableRow>
      <TableRow>
        <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={5}>
          <Collapse in={open} timeout="auto" unmountOnExit>
            <CodeBlockExpand title="Payload">{renderJson(payload)}</CodeBlockExpand>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

export default function Events({ data }: { data: Event[] }) {
  return (
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell width={90}>Payload</TableCell>
            <TableCell>Topic</TableCell>
            <TableCell>Component Address</TableCell>
            <TableCell>Template Address</TableCell>
            <TableCell>Transaction Hash</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {data.map(({ component_address, template_address, topic, tx_hash, payload }: Event, index: number) => {
            return (
              <RowData
                component_address={component_address}
                template_address={template_address}
                topic={topic}
                tx_hash={tx_hash}
                payload={payload}
                key={index}
              />
            );
          })}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
