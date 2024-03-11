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

import { useState, useEffect } from "react";
import { useParams } from "react-router-dom";
import { getTemplate } from "../../../utils/json_rpc";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import { DataTableCell, BoxHeading, BoxHeading2 } from "../../../Components/StyledComponents";
import PageHeading from "../../../Components/PageHeading";
import Grid from "@mui/material/Grid";
import { StyledPaper } from "../../../Components/StyledComponents";
import { fromHexString } from "./helpers";
import type { ArgDef, GetTemplateResponse } from "@tariproject/typescript-bindings/validator-node-client";

function TemplateFunctions() {
  const { address } = useParams();
  const [info, setInfo] = useState<GetTemplateResponse>();

  useEffect(() => {
    const load = (address: Uint8Array) => {
      getTemplate({ template_address: address }).then((response) => {
        setInfo(response);
      });
    };
    const data = address ? fromHexString(address.replace("0x", "")) : new Uint8Array();
    load(data);
  }, [address]);

  const renderFunctions = (template: GetTemplateResponse) => {
    return (
      <TableContainer>
        <BoxHeading2>{template.abi.template_name}</BoxHeading2>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>Function</TableCell>
              <TableCell>Args</TableCell>
              <TableCell>Returns</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {template.abi.functions.map((fn) => (
              <TableRow key={fn.name}>
                <DataTableCell style={{ textAlign: "left" }}>{fn.name}</DataTableCell>
                <DataTableCell>
                  {fn.arguments
                    .map((a: ArgDef) => {
                      return a.name + ":" + a.arg_type;
                    })
                    .join(", ")}
                </DataTableCell>
                <DataTableCell>{fn.output}</DataTableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>
    );
  };

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Template Functions</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <BoxHeading>Address: {address}</BoxHeading>
          {info ? renderFunctions(info) : ""}
        </StyledPaper>
      </Grid>
    </>
  );
}

export default TemplateFunctions;
