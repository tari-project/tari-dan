import * as React from 'react';
import Box from '@mui/material/Box';
import Collapse from '@mui/material/Collapse';
import IconButton from '@mui/material/IconButton';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import Typography from '@mui/material/Typography';
import Paper from '@mui/material/Paper';
import KeyboardArrowDownIcon from '@mui/icons-material/KeyboardArrowDown';
import KeyboardArrowUpIcon from '@mui/icons-material/KeyboardArrowUp';
import theme from '../theme';
import { DataTableCell } from './StyledComponents';
// import MetaCode from '../assets/demo/codeblock-meta.jpg';

function createData(
  payload: string,
  date: string,
  meta: string,
  instructions: string
) {
  return {
    payload,
    date,
    meta,
    instructions,
  };
}

function Row(props: { row: ReturnType<typeof createData> }) {
  const { row } = props;
  const [open1, setOpen1] = React.useState(false);
  const [open2, setOpen2] = React.useState(false);

  return (
    <>
      <TableRow sx={{ borderBottom: 'none' }}>
        <DataTableCell
          component="th"
          scope="row"
          sx={{
            borderBottom: 'none',
          }}
        >
          {row.payload}
        </DataTableCell>
        <DataTableCell
          sx={{
            borderBottom: 'none',
          }}
        >
          {row.date}
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <IconButton
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen1(!open1);
              setOpen2(false);
            }}
            sx={
              open1
                ? { backgroundColor: '#9330FF', color: '#fff' }
                : { backgroundColor: '#fff', color: '#9330FF' }
            }
          >
            {open1 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </IconButton>
        </DataTableCell>
        <DataTableCell sx={{ borderBottom: 'none', textAlign: 'center' }}>
          <IconButton
            aria-label="expand row"
            size="small"
            onClick={() => {
              setOpen2(!open2);
              setOpen1(false);
            }}
            sx={
              open2
                ? { backgroundColor: '#9330FF', color: '#fff' }
                : { backgroundColor: '#fff', color: '#9330FF' }
            }
          >
            {open2 ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </IconButton>
        </DataTableCell>
      </TableRow>
      <TableRow>
        <DataTableCell
          style={{ paddingBottom: 0, paddingTop: 0, borderBottom: 'none' }}
          colSpan={4}
        >
          <Collapse in={open1} timeout="auto" unmountOnExit>
            <Box sx={{ margin: 1 }}>
              <Box
                component="img"
                sx={{
                  height: 184,
                  width: 1120,
                }}
                src="http://localhost:3000/codeblock-meta.jpg"
              />
            </Box>
          </Collapse>
        </DataTableCell>
      </TableRow>
      <TableRow>
        <DataTableCell style={{ paddingBottom: 0, paddingTop: 0 }} colSpan={4}>
          <Collapse in={open2} timeout="auto" unmountOnExit>
            <Box sx={{ margin: 1 }}>
              <Box
                component="img"
                sx={{
                  height: 216,
                  width: 1120,
                }}
                src="http://localhost:3000/codeblock-instructions.jpg"
              />
            </Box>
          </Collapse>
        </DataTableCell>
      </TableRow>
    </>
  );
}

const rows = [
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
  createData(
    '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    'Thu, 24 Nov 2022 17:28:47 GMT',
    'Meta',
    'Instructions'
  ),
];

export default function CollapsibleTable() {
  return (
    <TableContainer>
      <Table aria-label="collapsible table">
        <TableHead>
          <TableRow>
            <TableCell>Payload ID</TableCell>
            <TableCell>Date</TableCell>
            <TableCell sx={{ textAlign: 'center' }}>Meta</TableCell>
            <TableCell sx={{ textAlign: 'center' }}>Instructions</TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {rows.map((row) => (
            <Row key="1" row={row} />
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
