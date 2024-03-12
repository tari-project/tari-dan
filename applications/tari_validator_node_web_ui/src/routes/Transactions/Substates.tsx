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
import { renderJson } from "../../utils/helpers";
import { DataTableCell } from "../../Components/StyledComponents";
import { AccordionIconButton } from "../../Components/StyledComponents";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import { IoArrowDownCircle, IoArrowUpCircle } from "react-icons/io5";
import CodeBlockDialog from "../../Components/CodeBlock";
import type { SubstateRecord } from "@tariproject/typescript-bindings";

function RowData({ info, state }: { info: SubstateRecord; state: string }, index: number) {
  const [open, setOpen] = useState(false);
  const itemKey = Object.keys(info.substate_id)[0];
  const itemValue = Object.values(info.substate_id)[0];
  return (
    <>
      <TableRow key={`${index}-1`}>
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
        <DataTableCell>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "flex-start",
              gap: "0.5rem",
            }}
          >
            {state === "Up" ? (
              <IoArrowUpCircle style={{ width: 22, height: 22, color: "#5F9C91" }} />
            ) : (
              <IoArrowDownCircle style={{ width: 22, height: 22, color: "#ECA86A" }} />
            )}
            {state}
          </div>
        </DataTableCell>
        <DataTableCell>{itemKey}</DataTableCell>
        <DataTableCell>
          {itemValue && typeof itemValue === "object" ? JSON.stringify(itemValue) : String(itemValue)}
        </DataTableCell>
      </TableRow>
      <TableRow key={`${index}-2`}>
        <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={4}>
          <Collapse in={open} timeout="auto" unmountOnExit>
            <CodeBlockDialog title="Substate">{renderJson(info)}</CodeBlockDialog>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

export default function Substates({ upData, downData }: { upData: SubstateRecord[]; downData: SubstateRecord[] }) {
  // const down = data.Accept.down_substates;
  const up = upData;
  const down = downData;
  return (
    <TableContainer>
      <Table>
        <TableBody>
          {up.map((item: SubstateRecord, index: number) => {
            return <RowData info={item} state="Up" key={index} />;
          })}
          {down.map((item: SubstateRecord, index: number) => {
            return <RowData info={item} state="Down" key={index} />;
          })}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
