import PageHeading from '../../Components/PageHeading';
import Typography from '@mui/material/Typography';
import Grid from '@mui/material/Grid';
import { StyledPaper } from '../../Components/StyledComponents';
import CollapsibleTable from '../../Components/CollapsibleTable';
import RecentTransactions from '../VN/Components/RecentTransactions';

function RecentTransactionsLayout() {
  return (
    <div>
      <Grid container spacing={5}>
        <PageHeading>Recent Transactions</PageHeading>
        <Grid item xs={12}>
          <StyledPaper>
            <RecentTransactions />
          </StyledPaper>
        </Grid>
        <Grid item xs={12}>
          <StyledPaper>
            <CollapsibleTable />
          </StyledPaper>
        </Grid>
      </Grid>
    </div>
  );
}

export default RecentTransactionsLayout;
