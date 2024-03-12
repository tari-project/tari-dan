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
import { Link } from "react-router-dom";
import TableCell from "@mui/material/TableCell";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, CodeBlock, AccordionIconButton } from "../../Components/StyledComponents";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import IconButton from "@mui/material/IconButton";
import Collapse from "@mui/material/Collapse";
import { Typography } from "@mui/material";
import ManageSearchOutlinedIcon from "@mui/icons-material/ManageSearchOutlined";
import type { ValidatorNode } from "@tariproject/typescript-bindings/validator-node-client";

function Committee({
  begin,
  end,
  members,
  peerId,
}: {
  begin: string;
  end: string;
  members: ValidatorNode[];
  peerId: string;
}) {
  const [openMembers, setOpenMembers] = useState(false);

  return (
    <>
      <TableRow key={begin}>
        <DataTableCell style={{ borderBottom: "none" }}>
          {end < begin ? (
            <>
              [<span>{begin}</span>, <span>ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff</span>] [
              <span>0000000000000000000000000000000000000000000000000000000000000000</span>, <span>{end}</span>]
            </>
          ) : (
            <div>
              [<span>{begin}</span>, <span>{end}</span>]
            </div>
          )}
        </DataTableCell>
        <TableCell
          style={{
            verticalAlign: "top",
            borderBottom: "none",
            textAlign: "center",
          }}
          width="120px"
        >
          <AccordionIconButton
            open={openMembers}
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpenMembers(!openMembers);
            }}
          >
            {openMembers ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </TableCell>
        <TableCell
          style={{
            verticalAlign: "top",
            borderBottom: "none",
            textAlign: "center",
          }}
          width="120px"
        >
          <IconButton
            color="primary"
            component={Link}
            to={`/committees/${begin},${end}`}
            state={{ begin, end, members, peerId }}
          >
            <ManageSearchOutlinedIcon />
          </IconButton>
        </TableCell>
      </TableRow>
      <TableRow>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
          }}
          colSpan={3}
        >
          <Collapse in={openMembers} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: "10px", overflowY: "auto" }}>
              <Typography variant="h6">Public Keys</Typography>
              {members.map((member) => (
                <div className={`member ${member.address === peerId ? "me" : ""}`} key={member.address}>
                  {member.address} (Registration Epoch: {member.epoch})
                </div>
              ))}
            </CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

export default Committee;
