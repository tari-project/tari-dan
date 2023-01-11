import { DataGrid, GridRowsProp, GridColDef } from '@mui/x-data-grid';
import { Button } from '@mui/material';
import KeyboardArrowDownOutlinedIcon from '@mui/icons-material/KeyboardArrowDownOutlined';

const rows: GridRowsProp = [
  {
    id: 1,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 24 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 2,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 25 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 3,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 4,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 5,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 6,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 7,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 8,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 9,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
  {
    id: 10,
    col1: '3c6220bed856b2a9e09cf0a40431965a27fae496cdc35d2464aec09c58978310',
    col2: 'Thu, 26 Nov 2022 17:28:47 GMT',
    col3: 'Meta',
    col4: 'Instructions',
  },
];

const columns: GridColDef[] = [
  { field: 'col1', headerName: 'Payload ID', width: 580 },
  { field: 'col2', headerName: 'Date', width: 250 },
  {
    field: 'col3',
    headerName: 'Meta',
    width: 120,
    renderCell: (params) => (
      <strong>
        <Button
          variant="outlined"
          size="small"
          onClick={() => console.log('clicked')}
          endIcon={<KeyboardArrowDownOutlinedIcon />}
        >
          {params.row.col3}
        </Button>
      </strong>
    ),
  },
  {
    field: 'col4',
    headerName: 'Instructions',
    width: 180,
    renderCell: (params) => (
      <strong>
        <Button
          variant="outlined"
          size="small"
          onClick={() => console.log('clicked')}
          endIcon={<KeyboardArrowDownOutlinedIcon />}
        >
          {params.row.col4}
        </Button>
      </strong>
    ),
  },
];

export default function GridInfo() {
  return (
    <div style={{ height: 650, width: '100%' }}>
      <DataGrid rows={rows} columns={columns} />
    </div>
  );
}
