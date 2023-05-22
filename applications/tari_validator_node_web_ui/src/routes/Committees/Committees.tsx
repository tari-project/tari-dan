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
import Committee from './Committee';
import Table from '@mui/material/Table';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableBody from '@mui/material/TableBody';
import TableRow from '@mui/material/TableRow';
import TablePagination from '@mui/material/TablePagination';
import { Typography } from '@mui/material';
import CommitteesWaterfall from './CommitteesWaterfall';
import { get_all_committees } from './helpers';

function Committees({
  currentEpoch,
  shardKey,
  publicKey,
}: {
  currentEpoch: number;
  shardKey: string;
  publicKey: string;
}) {
  const [committees, setCommittees] = useState<
    Array<[string, string, Array<string>]>
  >([]);
  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);

  useEffect(() => {
    if (publicKey !== null) {
      get_all_committees(currentEpoch, shardKey, publicKey).then((response) => {
        if (response) setCommittees(response);
      });
    }
  }, [currentEpoch, shardKey, publicKey]);
  if (!committees) {
    return <Typography>Committees are loading</Typography>;
  }

  const emptyRows =
    page > 0 ? Math.max(0, (1 + page) * rowsPerPage - committees.length) : 0;

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (
    event: React.ChangeEvent<HTMLInputElement>
  ) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };
  return (
    <>
      <CommitteesWaterfall committees={committees} />
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>Range</TableCell>
              <TableCell style={{ textAlign: 'center' }}>Members</TableCell>
              <TableCell style={{ textAlign: 'center' }}>Details</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {committees.map(([begin, end, committee]) => (
              <Committee
                key={begin}
                begin={begin}
                end={end}
                members={committee}
                publicKey={publicKey}
              />
            ))}
            {emptyRows > 0 && (
              <TableRow
                style={{
                  height: 67 * emptyRows,
                }}
              >
                <TableCell colSpan={2} />
              </TableRow>
            )}
          </TableBody>
        </Table>
        <TablePagination
          rowsPerPageOptions={[10, 25, 50]}
          component="div"
          count={committees.length}
          rowsPerPage={rowsPerPage}
          page={page}
          onPageChange={handleChangePage}
          onRowsPerPageChange={handleChangeRowsPerPage}
        />
      </TableContainer>
    </>
  );
}

export default Committees;
