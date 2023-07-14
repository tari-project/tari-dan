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

import { useState, useEffect } from 'react';
import { getAllTokens } from '../../../utils/json_rpc';
import Button from '@mui/material/Button';
import { IoTrashOutline } from 'react-icons/io5';
import IconButton from '@mui/material/IconButton';
import Dialog from '@mui/material/Dialog';
import DialogActions from '@mui/material/DialogActions';
import DialogContent from '@mui/material/DialogContent';
import DialogContentText from '@mui/material/DialogContentText';
import DialogTitle from '@mui/material/DialogTitle';
import {
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TablePagination,
  CircularProgress,
  Fade,
} from '@mui/material';
import { DataTableCell } from '../../../Components/StyledComponents';
import theme from '../../../theme';
import { shortenString } from '../../../utils/helpers';
import CopyToClipboard from '../../../Components/CopyToClipboard';

interface IToken {
  id: number;
  name: string;
  deleted: boolean;
}

function AlertDialog({ fn, row }: any) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    getAllTokens().then((res) => console.log('Tokens', res));
  }, []);

  const handleClickOpen = () => {
    setOpen(true);
  };

  const handleClose = () => {
    setOpen(false);
  };

  const handleRevokeClose = () => {
    fn();
    setOpen(false);
  };

  return (
    <div>
      <IconButton onClick={handleClickOpen} color="primary">
        <IoTrashOutline />
      </IconButton>
      <Dialog
        open={open}
        onClose={handleClose}
        aria-labelledby="alert-dialog-title"
        aria-describedby="alert-dialog-description"
      >
        <DialogTitle id="alert-dialog-title">Revoke Token</DialogTitle>
        <DialogContent>
          <DialogContentText id="alert-dialog-description">
            Would you like to revoke this token?
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button variant="outlined" onClick={handleClose}>
            No, Cancel
          </Button>
          <Button variant="contained" onClick={handleRevokeClose} autoFocus>
            Yes, Revoke
          </Button>
        </DialogActions>
      </Dialog>
    </div>
  );
}

export default function AccessTokens() {
  const [tokens, setTokens] = useState<IToken[]>([]);
  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);
  const [error, setError] = useState<String>();
  const [loading, setLoading] = useState(true);

  const loadTokens = () => {
    getAllTokens()
      .then((response) => {
        console.log('response', response);
        setTokens(
          response.jwt.map((t: any) => {
            return {
              id: t[0],
              name: t[1],
              deleted: false,
            };
          })
        );
        setError(undefined);
      })
      .catch((err) => {
        setError(
          err && err.message
            ? err.message
            : `Unknown error: ${JSON.stringify(err)}`
        );
      })
      .finally(() => {
        setLoading(false);
      });
  };

  useEffect(() => {
    loadTokens();
  }, []);

  const handleRevoke = (id: any) => {
    setTokens((prevRows) =>
      prevRows.map((row) => {
        if (row.id === id) {
          return { ...row, deleted: true };
        }
        return row;
      })
    );
    setTimeout(() => {
      setTokens((prevRows) => prevRows.filter((row) => row.id !== id));
    }, 500);
  };

  const emptyRows =
    page > 0 ? Math.max(0, (1 + page) * rowsPerPage - tokens.length) : 0;

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
    <TableContainer>
      <Table>
        <TableHead>
          <TableRow>
            <TableCell>Token Name</TableCell>
            <TableCell width="100" align="center">
              Revoke
            </TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {tokens &&
            tokens
              .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
              .map(({ id, name, deleted }: IToken) => {
                if (!deleted) {
                  return (
                    <Fade in={!deleted} key={id}>
                      <TableRow
                        key={id}
                        className={deleted ? 'purple-flash' : ''}
                      >
                        <DataTableCell>
                          {shortenString(name)}
                          <CopyToClipboard copy={name} />
                        </DataTableCell>
                        <DataTableCell align="center">
                          <AlertDialog fn={() => handleRevoke(id)} row={id} />
                        </DataTableCell>
                      </TableRow>
                    </Fade>
                  );
                } else {
                  return (
                    <TableRow key={id}>
                      <DataTableCell
                        colSpan={2}
                        height={73}
                        className="purple-flash"
                      >
                        <div
                          style={{
                            display: 'flex',
                            justifyContent: 'center',
                            alignItems: 'center',
                            width: '100%',
                            gap: '1rem',
                          }}
                        >
                          <CircularProgress
                            style={{
                              color: theme.palette.primary.main,
                              height: '1.5rem',
                              width: '1.5rem',
                            }}
                          />
                        </div>
                      </DataTableCell>
                    </TableRow>
                  );
                }
              })}

          {emptyRows > 0 && (
            <TableRow style={{ height: 57 * emptyRows }}>
              <TableCell colSpan={3} />
            </TableRow>
          )}
        </TableBody>
      </Table>
      <TablePagination
        rowsPerPageOptions={[10, 25, 50]}
        component="div"
        count={tokens.length}
        rowsPerPage={rowsPerPage}
        page={page}
        onPageChange={handleChangePage}
        onRowsPerPageChange={handleChangeRowsPerPage}
      />
    </TableContainer>
  );
}
