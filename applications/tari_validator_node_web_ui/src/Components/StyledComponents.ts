import Paper from '@mui/material/Paper';
import TableCell from '@mui/material/TableCell';
import { styled } from '@mui/material/styles';
import Box from '@mui/material/Box';

export const StyledPaper = styled(Paper)(({ theme }) => ({
  padding: theme.spacing(3),
  boxShadow: '10px 14px 28px rgba(35, 11, 73, 0.05)',
}));

export const DataTableCell = styled(TableCell)(({ theme }) => ({
  fontFamily: "'Courier New', Courier, monospace",
}));

export const CodeBlock = styled(Box)(({ theme }) => ({
  backgroundColor: '#F5F5F7',
  borderRadius: theme.shape.borderRadius,
  padding: theme.spacing(3),
}));
