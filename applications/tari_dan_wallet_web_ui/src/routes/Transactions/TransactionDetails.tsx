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
import { useLocation } from 'react-router-dom';
import { transactionsGet } from '../../utils/json_rpc';
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
} from '../../Components/Accordion';
import {
  Grid,
  Table,
  TableContainer,
  TableBody,
  TableRow,
  TableCell,
  Button,
  Fade,
  Alert,
} from '@mui/material';
import Typography from '@mui/material/Typography';
import { DataTableCell, StyledPaper } from '../../Components/StyledComponents';
import PageHeading from '../../Components/PageHeading';
import Events from './Events';
import Logs from './Logs';
import FeeInstructions from './FeeInstructions';
import Instructions from './Instructions';
import Substates from './Substates';
import StatusChip from '../../Components/StatusChip';
import KeyboardArrowDownIcon from '@mui/icons-material/KeyboardArrowDown';
import KeyboardArrowUpIcon from '@mui/icons-material/KeyboardArrowUp';
import Loading from '../../Components/Loading';

export default function TransactionDetails() {
  const [state, setState] = useState<any>([]);
  const [expandedPanels, setExpandedPanels] = useState<string[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<String>();
  const { hash, status, result, transaction, transaction_failure } = state;
  const location = useLocation();

  const getTransactionByHash = () => {
    setLoading(true);
    const path = location.pathname.split('/')[2];
    transactionsGet(path)
      .then((response) => {
        console.log('transaction details: ', response);
        setState(response);
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
    getTransactionByHash();
  }, []);

  console.log('state: ', state);

  const handleChange =
    (panel: string) => (event: React.SyntheticEvent, isExpanded: boolean) => {
      setExpandedPanels((prevExpandedPanels) => {
        if (isExpanded) {
          return [...prevExpandedPanels, panel];
        } else {
          return prevExpandedPanels.filter((p) => p !== panel);
        }
      });
    };

  const expandAll = () => {
    setExpandedPanels(['panel1', 'panel2', 'panel3', 'panel4', 'panel5']);
  };

  const collapseAll = () => {
    setExpandedPanels([]);
  };

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <PageHeading>Transaction Details</PageHeading>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          {loading ? (
            <Loading />
          ) : (
            <Fade in={!loading}>
              <div>
                {error ? (
                  <Alert severity="error">{error}</Alert>
                ) : (
                  <>
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
                            <TableCell>Total Fees</TableCell>
                            <DataTableCell>
                              {/* {result &&
                                result.cost_breakdown.total_fees_charged} */}
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
                    <div
                      style={{
                        display: 'flex',
                        justifyContent: 'space-between',
                        alignItems: 'center',
                        padding: '2rem 1rem 0.5rem 1rem',
                      }}
                      // className="flex-container"
                    >
                      <Typography variant="h5">More Info</Typography>
                      <div
                        style={{
                          display: 'flex',
                          justifyContent: 'flex-end',
                          gap: '1rem',
                        }}
                      >
                        <Button
                          onClick={expandAll}
                          style={{
                            fontSize: '0.85rem',
                          }}
                          startIcon={<KeyboardArrowDownIcon />}
                        >
                          Expand All
                        </Button>
                        <Button
                          onClick={collapseAll}
                          style={{
                            fontSize: '0.85rem',
                          }}
                          startIcon={<KeyboardArrowUpIcon />}
                          disabled={expandedPanels.length === 0 ? true : false}
                        >
                          Collapse All
                        </Button>
                      </div>
                    </div>
                  </>
                )}
                {transaction?.fee_instructions && (
                  <Accordion
                    expanded={expandedPanels.includes('panel1')}
                    onChange={handleChange('panel1')}
                  >
                    <AccordionSummary
                      aria-controls="panel1bh-content"
                      id="panel1bh-header"
                    >
                      <Typography>Fee Instructions</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <FeeInstructions data={transaction.fee_instructions} />
                    </AccordionDetails>
                  </Accordion>
                )}
                {transaction?.instructions && (
                  <Accordion
                    expanded={expandedPanels.includes('panel2')}
                    onChange={handleChange('panel2')}
                  >
                    <AccordionSummary
                      aria-controls="panel2bh-content"
                      id="panel1bh-header"
                    >
                      <Typography>Instructions</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Instructions data={transaction.instructions} />
                    </AccordionDetails>
                  </Accordion>
                )}
                {result && (
                  <Accordion
                    expanded={expandedPanels.includes('panel3')}
                    onChange={handleChange('panel3')}
                  >
                    <AccordionSummary
                      aria-controls="panel3bh-content"
                      id="panel1bh-header"
                    >
                      <Typography>Events</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Events data={result.events} />
                    </AccordionDetails>
                  </Accordion>
                )}
                {result && (
                  <Accordion
                    expanded={expandedPanels.includes('panel4')}
                    onChange={handleChange('panel4')}
                  >
                    <AccordionSummary
                      aria-controls="panel4bh-content"
                      id="panel1bh-header"
                    >
                      <Typography>Logs</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Logs data={result.logs} />
                    </AccordionDetails>
                  </Accordion>
                )}
                {transaction_failure === null && (
                  <Accordion
                    expanded={expandedPanels.includes('panel5')}
                    onChange={handleChange('panel5')}
                  >
                    <AccordionSummary
                      aria-controls="panel5bh-content"
                      id="panel1bh-header"
                    >
                      <Typography>Substates</Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Substates data={result.result} />
                    </AccordionDetails>
                  </Accordion>
                )}
              </div>
            </Fade>
          )}
        </StyledPaper>
      </Grid>
    </>
  );
}
