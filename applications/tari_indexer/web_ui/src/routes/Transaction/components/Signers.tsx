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

import {
  Box,
  Chip,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { TransactionSignature } from "@tari-project/ootle-ts-bindings";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import { DataTableCell } from "../../../Components/StyledComponents";

interface SignersProps {
  seal_signature?: TransactionSignature;
  transaction_body?: {
    signatures: TransactionSignature[];
  };
}

export default function Signers({ seal_signature, transaction_body }: SignersProps) {
  const allSigners = [
    ...(seal_signature ? [{ ...seal_signature, type: "Sealed By" }] : []),
    ...(transaction_body?.signatures ?? []).map((sig, i) => ({ ...sig, type: `Signer ${i + 1}` })),
  ];

  if (allSigners.length === 0) {
    return (
      <Box sx={{ p: 3, textAlign: "center" }}>
        <Typography variant="body2" color="text.secondary">
          No signers available
        </Typography>
      </Box>
    );
  }

  return (
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell width={130}></TableCell>
            <TableCell width={250}>Public Key</TableCell>
            <TableCell>Signature Details</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {allSigners.map((signer, index) => (
            <TableRow key={index}>
              <DataTableCell>
                <Typography variant="body2">{signer.type}</Typography>
              </DataTableCell>
              <DataTableCell>
                <Stack direction="row" alignItems="center">
                  <Typography variant="body2" sx={{ fontFamily: "monospace", wordBreak: "break-all" }}>
                    {signer.public_key}
                  </Typography>
                  <CopyToClipboard copy={signer.public_key} />
                </Stack>
              </DataTableCell>
              <DataTableCell>
                <Chip label={`Signature: ${signer.signature}`} size="small" color="default" variant="outlined" />
              </DataTableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
