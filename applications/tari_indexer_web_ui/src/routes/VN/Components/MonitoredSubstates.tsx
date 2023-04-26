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
import {
  addAddress,
  deleteAddress,
  getAddresses,
  getSubstate,
} from '../../../utils/json_rpc';
import { Form } from 'react-router-dom';
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
  BoxHeading2,
} from '../../../Components/StyledComponents';
import KeyboardArrowDownIcon from '@mui/icons-material/KeyboardArrowDown';
import KeyboardArrowUpIcon from '@mui/icons-material/KeyboardArrowUp';
import Collapse from '@mui/material/Collapse';
import TablePagination from '@mui/material/TablePagination';
import Typography from '@mui/material/Typography';
import { Button, TextField } from '@mui/material';
import AddIcon from '@mui/icons-material/Add';
import { ConfirmDialog } from '../../../Components/AlertDialog';
import Fade from '@mui/material/Fade';

interface ITableAddresses {
  id: string;
  address: string;
  version: number;
}

type ColumnKey = keyof ITableAddresses;

function RowData({
  id,
  address,
  version,
  onDelete,
}: {
  id: string;
  address: string;
  version: number;
  onDelete: (address: string) => void;
}) {
  const [open1, setOpen1] = useState(false);
  const [data, setData] = useState<string | null>(null);
  return (
    <>
      <TableRow key={id} sx={{ borderBottom: 'none' }}>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
            borderBottom: 'none',
          }}
          colSpan={1}
        >
          {address}
        </DataTableCell>
        <DataTableCell
          style={{
            paddingBottom: 0,
            paddingTop: 0,
            borderBottom: 'none',
          }}
          colSpan={1}
        >
          {version}
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <AccordionIconButton
            open={open1}
            aria-label="expand row"
            size="small"
            onClick={() => {
              if (data === null) {
                getSubstate(address)
                  .then((resp) => {
                    setData(JSON.stringify(resp));
                  })
                  .catch((error) => {
                    console.log('Error', error);
                  });
              }
              setOpen1(!open1);
            }}
          >
            {open1 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </AccordionIconButton>
        </DataTableCell>
        <DataTableCell>
          <ConfirmDialog
            buttonTitle="Delete"
            confirmTitle="Yes, Delete Address"
            confirmFunction={() => onDelete(address)}
            cancelTitle="Cancel"
            dialogTitle="Delete Address"
            dialogDescription="You are about to delete this address. Are you sure?"
          />
        </DataTableCell>
      </TableRow>
      <TableRow key={`${id}-2`}>
        <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={4}>
          <Collapse in={open1} timeout="auto" unmountOnExit>
            <CodeBlock style={{ marginBottom: '10px' }}>
              {data ? renderJson(JSON.parse(data)) : null}
            </CodeBlock>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

function MonitoredSubstates() {
  const [addresses, setAddresses] = useState<ITableAddresses[]>([]);
  const [lastSort, setLastSort] = useState({ column: '', order: -1 });

  const [page, setPage] = useState(0);
  const [rowsPerPage, setRowsPerPage] = useState(10);

  const [showAddressDialog, setShowAddAddressDialog] = useState(false);
  const [formState, setFormState] = useState({ address: '' });

  const showAddAddressDialog = (
    setElseToggle: boolean = !showAddressDialog
  ) => {
    setShowAddAddressDialog(setElseToggle);
  };
  const onSubmitAddAddress = () => {
    addAddress(formState.address).then((resp) => {
      updatedAddresses();
    });
    setFormState({ address: '' });
    setShowAddAddressDialog(false);
  };
  const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFormState({ ...formState, [e.target.name]: e.target.value });
  };

  // Avoid a layout jump when reaching the last page with empty rows.
  const emptyRows =
    page > 0 ? Math.max(0, (1 + page) * rowsPerPage - addresses.length) : 0;

  const handleChangePage = (event: unknown, newPage: number) => {
    setPage(newPage);
  };

  const handleChangeRowsPerPage = (
    event: React.ChangeEvent<HTMLInputElement>
  ) => {
    setRowsPerPage(parseInt(event.target.value, 10));
    setPage(0);
  };

  const updatedAddresses = () => {
    getAddresses().then((resp) => {
      setAddresses(
        resp.map(([address, version]: [string, number]) => ({
          id: address,
          address: address,
          version: version,
        }))
      );
    });
  };

  useEffect(() => {
    updatedAddresses();
  }, []);

  const onDelete = (address: string) => {
    deleteAddress(address).then((resp) => {
      console.log(`Address ${address} deleted`);
      updatedAddresses();
    });
  };
  const sort = (column: ColumnKey) => {
    let order = 1;
    if (lastSort.column === column) {
      order = -lastSort.order;
    }
    setAddresses(
      [...addresses].sort((r0, r1) =>
        r0[column] > r1[column] ? order : r0[column] < r1[column] ? -order : 0
      )
    );
    setLastSort({ column, order });
  };
  if (addresses === undefined) {
    return (
      <Typography variant="h4">Monitored addresses ... loading</Typography>
    );
  }

  return (
    <TableContainer>
      <BoxHeading2 style={{ minHeight: '75px' }}>
        {showAddressDialog && (
          <Fade in={showAddressDialog}>
            <Form onSubmit={onSubmitAddAddress} className="add-confirm-form">
              <TextField
                name="address"
                label="Address"
                value={formState.address}
                onChange={onChange}
                style={{ flexGrow: 1 }}
              />
              <Button variant="contained" type="submit">
                Add Address
              </Button>
              <Button
                variant="outlined"
                onClick={() => showAddAddressDialog(false)}
              >
                Cancel
              </Button>
            </Form>
          </Fade>
        )}
        {!showAddressDialog && (
          <Fade in={!showAddressDialog}>
            <div className="flex-container">
              <Button
                startIcon={<AddIcon />}
                onClick={() => showAddAddressDialog()}
                variant="outlined"
              >
                Add address
              </Button>
            </div>
          </Fade>
        )}
      </BoxHeading2>

      <Table>
        <TableHead>
          <TableRow>
            <TableCell
              onClick={() => sort('address')}
              style={{ textAlign: 'center' }}
            >
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'flex-start',
                  alignItems: 'center',
                  gap: '5px',
                }}
              >
                Address
                {lastSort.column === 'address' ? (
                  lastSort.order === 1 ? (
                    <KeyboardArrowUpIcon />
                  ) : (
                    <KeyboardArrowDownIcon />
                  )
                ) : (
                  ''
                )}
              </div>
            </TableCell>
            <TableCell
              onClick={() => sort('version')}
              style={{ textAlign: 'center' }}
            >
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'flex-start',
                  alignItems: 'center',
                  gap: '5px',
                }}
              >
                Version
                {lastSort.column === 'version' ? (
                  lastSort.order === 1 ? (
                    <KeyboardArrowUpIcon />
                  ) : (
                    <KeyboardArrowDownIcon />
                  )
                ) : (
                  ''
                )}
              </div>
            </TableCell>
            <TableCell style={{ textAlign: 'center' }}>Substate</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {addresses
            .slice(page * rowsPerPage, page * rowsPerPage + rowsPerPage)
            .map(({ id, address, version }) => (
              <RowData
                key={id}
                id={id}
                address={address}
                version={version}
                onDelete={onDelete}
              />
            ))}
          {emptyRows > 0 && (
            <TableRow
              style={{
                height: 67 * emptyRows,
              }}
            >
              <TableCell colSpan={4} />
            </TableRow>
          )}
        </TableBody>
      </Table>
      <TablePagination
        rowsPerPageOptions={[10, 25, 50]}
        component="div"
        count={addresses.length}
        rowsPerPage={rowsPerPage}
        page={page}
        onPageChange={handleChangePage}
        onRowsPerPageChange={handleChangeRowsPerPage}
      />
    </TableContainer>
  );
}

export default MonitoredSubstates;
