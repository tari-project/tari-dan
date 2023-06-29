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
import { useLoaderData, useLocation } from 'react-router-dom';
import { transactionsGet } from '../../utils/json_rpc';
import {
  Grid,
  Table,
  TableContainer,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
  Button,
  Chip,
  Avatar,
  Box,
} from '@mui/material';
import Accordion from '@mui/material/Accordion';
import AccordionSummary from '@mui/material/AccordionSummary';
import AccordionDetails from '@mui/material/AccordionDetails';
import Typography from '@mui/material/Typography';
import ExpandMoreIcon from '@mui/icons-material/ExpandMore';
import { DataTableCell, StyledPaper } from '../../Components/StyledComponents';
import PageHeading from '../../Components/PageHeading';
import { demoTransactionDetail } from '../../assets/transactionDetail';
import Events from './Events';
import Logs from './Logs';
import FeeInstructions from './FeeInstructions';
import Instructions from './Instructions';
import Substates from './Substates';
import { IoCheckmarkOutline } from 'react-icons/io5';
import SecondaryHeading from '../../Components/SecondaryHeading';
import StatusChip from '../../Components/StatusChip';

// type loaderData = any;

// export async function transactionLoader({ params }: any) {
//   const { id } = params;
//   const result = await transactionsGet(id);
//   console.log('result', result);
//   return result;
// }

export default function TransactionDetails() {
  const [state, setState] = useState<any>([]);
  const [error, setError] = useState<String>();
  const [expanded, setExpanded] = useState<boolean>(true);
  const { hash, status, result, transaction, transaction_failure } = state;
  const location = useLocation();
  //   const loaderData = useLoaderData() as loaderData;
  //   console.log('loaderData', loaderData);

  const getTransactionByHash = async () => {
    const path = location.pathname.split('/')[2];
    const result = await transactionsGet(path);
    console.log('result', result);
    setState(result);
  };

  useEffect(() => {
    getTransactionByHash();
    // console.log(demoTransactionDetail);
    // setState(demoTransactionDetail);
  }, []);

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Transaction Details</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <TableContainer>
            <Table>
              <TableBody>
                <TableRow>
                  <TableCell>Transaction Hash</TableCell>
                  <DataTableCell>{hash}</DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Timestamp</TableCell>
                  <DataTableCell>Timestamp</DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Amount</TableCell>
                  <DataTableCell>Amount</DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Total Fees</TableCell>
                  <DataTableCell>
                    {result && result.cost_breakdown.total_fees_charged}
                  </DataTableCell>
                </TableRow>
                <TableRow>
                  <TableCell>Status</TableCell>
                  <DataTableCell>
                    <StatusChip status={status} />
                  </DataTableCell>
                </TableRow>
              </TableBody>
            </Table>
          </TableContainer>
        </StyledPaper>
      </Grid>
      {/* <Grid item xs={12} md={12} lg={12}>
        <Box
          style={{
            display: 'flex',
            width: '100%',
            justifyContent: 'space-between',
            alignItems: 'center',
            padding: '0 1rem',
          }}
        >
          <Typography variant="h4">More Info</Typography>
          <Button onClick={() => setExpanded(!expanded)}>
            {!expanded ? 'Expand All' : 'Collapse All'}
          </Button>
        </Box>
        {transaction?.fee_instructions && (
          <Accordion expanded={expanded} square>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              aria-controls="panel1a-content"
              id="panel1a-header"
            >
              <Typography>Fee Instructions</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <FeeInstructions data={transaction.fee_instructions} />
            </AccordionDetails>
          </Accordion>
        )}
        {transaction?.instructions && (
          <Accordion expanded={expanded} square>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              aria-controls="panel1a-content"
              id="panel1a-header"
            >
              <Typography>Instructions</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <Instructions data={transaction.instructions} />
            </AccordionDetails>
          </Accordion>
        )}
        {result && (
          <Accordion expanded={expanded} square>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              aria-controls="panel1a-content"
              id="panel1a-header"
            >
              <Typography>Events</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <Events data={result.events} />
            </AccordionDetails>
          </Accordion>
        )}
        {result && (
          <Accordion expanded={expanded} square>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              aria-controls="panel2a-content"
              id="panel2a-header"
            >
              <Typography>Logs</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <Logs data={result.logs} />
            </AccordionDetails>
          </Accordion>
        )}
        {result && (
          <Accordion expanded={expanded} square>
            <AccordionSummary
              expandIcon={<ExpandMoreIcon />}
              aria-controls="panel1a-content"
              id="panel1a-header"
            >
              <Typography>Substates</Typography>
            </AccordionSummary>
            <AccordionDetails>
              <Substates data={result.result} />
            </AccordionDetails>
          </Accordion>
        )}
      </Grid> */}
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Fee Instructions</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          {transaction?.fee_instructions && (
            <FeeInstructions data={transaction.fee_instructions} />
          )}
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Instructions</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          {transaction?.instructions && (
            <Instructions data={transaction.instructions} />
          )}
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Events</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>{result && <Events data={result.events} />}</StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Logs</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>{result && <Logs data={result.logs} />}</StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <SecondaryHeading>Substates</SecondaryHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          {result && <Substates data={result.result} />}
        </StyledPaper>
      </Grid>
    </>
  );
}
