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
import { TableContainer, Table, TableRow, TableBody, Collapse } from "@mui/material";
import { DataTableCell, AccordionIconButton } from "../../Components/StyledComponents";
import { renderJson } from "../../utils/helpers";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import CodeBlockExpand from "../../Components/CodeBlock";
import type { Instruction } from "@tariproject/typescript-bindings";

function RowData({ title, data }: { title: string; data: Instruction }, index: number) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <TableRow key={`${index}-1`}>
        <DataTableCell width={90} sx={{ borderBottom: "none", textAlign: "center" }}>
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
        <DataTableCell>{title}</DataTableCell>
      </TableRow>
      <TableRow key={`${index}-2`}>
        <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={2}>
          <Collapse in={open} timeout="auto" unmountOnExit>
            <CodeBlockExpand title={title}>{renderJson(data)}</CodeBlockExpand>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

export default function Instructions({ data }: { data: Instruction[] }, index: number) {
  return (
    <TableContainer>
      <Table>
        <TableBody>
          {data &&
            data.map((item: Instruction) => {
              return <RowData title={Object.keys(item)[0]} data={item} />;
            })}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
