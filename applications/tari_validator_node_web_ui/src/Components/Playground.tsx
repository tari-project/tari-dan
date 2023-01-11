import Typography from '@mui/material/Typography';
import Grid from '@mui/material/Grid';
import Paper from '@mui/material/Paper';
import { Button } from '@mui/material';
import CustomizedAccordions from './Accordion';
import GridInfo from './GridInfo';
import PageHeading from './PageHeading';
import { StyledPaper } from './StyledComponents';
import CollapsibleTable from './CollapsibleTable';
// import ReactFlowDemo from './ReactFlow/ReactFlowDemo';
// import './ReactFlow/styles.css';

function DashboardContent() {
  return (
    <div>
      {/* <Container maxWidth="lg" sx={{ mt: 4, mb: 4 }}> */}
      <Grid container spacing={5}>
        <PageHeading>Playground</PageHeading>
        <Grid item xs={12} md={8} lg={9}>
          <StyledPaper>
            <p>Box no 1</p>
          </StyledPaper>
        </Grid>
        <Grid item xs={12} md={4} lg={3}>
          <StyledPaper>
            <Typography>Box no 3</Typography>
            <Typography>This is some text</Typography>
            <Button>Click me!</Button>
          </StyledPaper>
        </Grid>
        <Grid item xs={12}>
          <StyledPaper>
            <CustomizedAccordions />
          </StyledPaper>
        </Grid>
        <Grid item xs={12}>
          <StyledPaper>
            <CollapsibleTable />
          </StyledPaper>
        </Grid>
        <Grid item xs={12}>
          <StyledPaper>
            <GridInfo />
          </StyledPaper>
        </Grid>
      </Grid>
      {/* </Container> */}
    </div>
  );
}

export default function Dashboard() {
  return <DashboardContent />;
}
