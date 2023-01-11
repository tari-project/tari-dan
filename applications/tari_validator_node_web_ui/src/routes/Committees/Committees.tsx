import PageHeading from '../../Components/PageHeading';
import Typography from '@mui/material/Typography';
import Grid from '@mui/material/Grid';
import { StyledPaper } from '../../Components/StyledComponents';

function Committees() {
  return (
    <div>
      <Grid container spacing={5}>
        <PageHeading>Committees</PageHeading>
        <Grid item xs={12} md={12} lg={12}>
          <StyledPaper>
            <Typography>Info goes in here</Typography>
          </StyledPaper>
        </Grid>
      </Grid>
    </div>
  );
}

export default Committees;
