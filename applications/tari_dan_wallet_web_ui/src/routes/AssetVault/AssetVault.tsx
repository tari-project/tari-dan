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
import Grid from '@mui/material/Grid';
import { StyledPaper, InnerHeading } from '../../Components/StyledComponents';
import TableContainer from '@mui/material/TableContainer';
import Table from '@mui/material/Table';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import TableCell from '@mui/material/TableCell';
import TableBody from '@mui/material/TableBody';
import Tabs from '@mui/material/Tabs';
import Tab from '@mui/material/Tab';
import Typography from '@mui/material/Typography';
import Box from '@mui/material/Box';
import Fade from '@mui/material/Fade';
import Button from '@mui/material/Button';
import {
  useAccountsGetBalances,
  useAccountsGet,
  useAccountNFTsList,
  useAccountsList,
} from '../../api/hooks/useAccounts';
import useAccountStore from '../../store/accountStore';
import Transactions from '../Transactions/Transactions';
import { removeTagged, shortenString } from '../../utils/helpers';
import { DataTableCell } from '../../Components/StyledComponents';
import CopyToClipboard from '../../Components/CopyToClipboard';
import FetchStatusCheck from '../../Components/FetchStatusCheck';
import { IoEyeOffOutline, IoEyeOutline } from 'react-icons/io5';
import IconButton from '@mui/material/IconButton';
import Divider from '@mui/material/Divider';
import { useTheme } from '@mui/material/styles';
import SelectAccount from './Components/SelectAccount';
import { useAccountsCreateFreeTestCoins } from '../../api/hooks/useAccounts';
import ClaimBurn from './Components/ClaimBurn';
import TariGem from '../../assets/TariGem';
import SendMoney from './Components/SendMoney';

interface TabPanelProps {
  children?: React.ReactNode;
  index: number;
  value: number;
}

function BalanceRow(props: any) {
  return (
    <TableRow key={props.index}>
      <DataTableCell>
        {shortenString(props.token_symbol || props.resource_address)}
        <CopyToClipboard copy={props.token_symbol || props.resource_address} />
      </DataTableCell>
      <DataTableCell>{props.resource_type}</DataTableCell>
      <DataTableCell>{removeTagged(props.balance)}</DataTableCell>
      <DataTableCell>{removeTagged(props.confidential_balance)}</DataTableCell>
    </TableRow>
  );
}

function NftsList(props: any) {
  return (
    <TableRow key={props.index}>
      <DataTableCell>{props.token_symbol}</DataTableCell>
      <DataTableCell>{props.metadata}</DataTableCell>
      <DataTableCell>{props.is_burned}</DataTableCell>
    </TableRow>
  );
}

function TabPanel(props: TabPanelProps) {
  const { children, value, index, ...other } = props;

  return (
    <div
      role="tabpanel"
      hidden={value !== index}
      id={`simple-tabpanel-${index}`}
      aria-labelledby={`simple-tab-${index}`}
      {...other}
    >
      {value === index && (
        <Box sx={{ p: 3 }}>
          <Typography>{children}</Typography>
        </Box>
      )}
    </div>
  );
}

function tabProps(index: number) {
  return {
    id: `asset-tab-${index}`,
    'aria-controls': `asset-tabpanel-${index}`,
  };
}

function AssetVault() {
  const { showBalance, setShowBalance, accountName, setAccountName } =
    useAccountStore();

    const { data: dataAccountsList } = useAccountsList(0, 10);
    // Set to the first account if we haven't selected an account
    if (!accountName && dataAccountsList?.accounts?.length > 0) {
        setAccountName(dataAccountsList.accounts[0].account.name);
    }

  const {
    data: balancesData,
    isError: balancesIsError,
    error: balancesError,
    refetch: balancesRefetch,
    isFetching: balancesIsFetching,
  } = useAccountsGetBalances(accountName);

  const {
    data: nftsListData,
    isError: nftsListIsError,
    error: nftsListError,
    refetch: nftsListRefetch,
    isFetching: nftsListIsFetching,
  } = useAccountNFTsList(0, 10);

  const {
    data: accountsData,
    isError: accountsIsError,
    error: accountsError,
    refetch: accountsRefetch,
    isFetching: accountsIsFetching,
  } = useAccountsGet(accountName);


  const { mutateAsync } = useAccountsCreateFreeTestCoins();

  const theme = useTheme();

  useEffect(() => {
    accountsRefetch();
    balancesRefetch();
    nftsListRefetch();
  }, [accountName]);

  const onClaimFreeCoins = async () => {
    await mutateAsync({
      accountName: 'TestAccount',
      amount: 100000,
      fee: 1000,
    });
  };

  function AccountBalance() {
    const formattedBalance =
      balancesData?.balances[0]?.confidential_balance.toLocaleString('en-US', {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      });

    return (
      <>
        <FetchStatusCheck
          isError={balancesIsError}
          errorMessage={balancesError?.message || 'Error fetching data'}
          isLoading={balancesIsFetching}
        />
        <Fade in={!balancesIsFetching && !balancesIsError} timeout={100}>
          <Box>
            <Box>
              <Typography variant="body2" gutterBottom={false}>
                Confidential Balance
              </Typography>
            </Box>
            <Box
              style={{
                display: 'flex',
                alignItems: 'flex-start',
                justifyContent: 'space-between',
                gap: theme.spacing(1),
                minWidth: '230px',
              }}
            >
              <Typography variant="h2">
                {showBalance
                  ? (
                      <>
                        <TariGem fill={theme.palette.text.primary} />{' '}
                        {formattedBalance}
                      </>
                    ) || (
                      <>
                        <TariGem fill={theme.palette.text.primary} /> 0
                      </>
                    )
                  : '************'}
              </Typography>
              <IconButton onClick={() => setShowBalance(!showBalance)}>
                {showBalance ? (
                  <IoEyeOffOutline color={theme.palette.primary.main} />
                ) : (
                  <IoEyeOutline color={theme.palette.primary.main} />
                )}
              </IconButton>
            </Box>
          </Box>
        </Fade>
      </>
    );
  }

  function AccountDetails() {
    return (
      <>
        {accountsIsError || accountsIsFetching ? (
          <FetchStatusCheck
            isError={accountsIsError}
            errorMessage={accountsError?.message || 'Error fetching data'}
            isLoading={accountsIsFetching}
          />
        ) : (
          <TableContainer>
            <Table>
              <TableHead>
                <TableRow>
                  <TableCell>Name</TableCell>
                  <TableCell>Address</TableCell>
                  <TableCell>Public key</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {accountsData && (
                  <TableRow>
                    <DataTableCell>{accountsData.account.name}</DataTableCell>
                    <DataTableCell>
                      {shortenString(accountsData.account.address.Component)}
                      <CopyToClipboard
                        copy={accountsData.account.address.Component}
                      />
                    </DataTableCell>
                    <DataTableCell>
                      {shortenString(accountsData.public_key)}
                      <CopyToClipboard copy={accountsData.public_key} />
                    </DataTableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </TableContainer>
        )}
      </>
    );
  }

  function AssetTabs() {
    const [value, setValue] = useState(0);

    const handleChange = (event: React.SyntheticEvent, newValue: number) => {
      setValue(newValue);
    };

    return (
      <Box sx={{ width: '100%' }}>
        <Box sx={{ borderBottom: 1, borderColor: 'divider' }}>
          <Tabs
            value={value}
            onChange={handleChange}
            aria-label="account assets"
            variant="standard"
          >
            <Tab label="Tokens" {...tabProps(0)} style={{ width: 150 }} />
            <Tab label="NFTs" {...tabProps(1)} style={{ width: 150 }} />
          </Tabs>
        </Box>
        <TabPanel value={value} index={0}>
          {balancesIsError || balancesIsFetching ? (
            <FetchStatusCheck
              isError={balancesIsError}
              errorMessage={balancesError?.message || 'Error fetching data'}
              isLoading={balancesIsFetching}
            />
          ) : (
            <TableContainer>
              <Table>
                <TableHead>
                  <TableRow>
                    <TableCell>Resource</TableCell>
                    <TableCell>Resource Type</TableCell>
                    <TableCell>Revealed Balance</TableCell>
                    <TableCell>Confidential Balance</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {balancesData?.balances.map(
                    (balance: number, index: number) => BalanceRow(balance)
                  )}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </TabPanel>
        <TabPanel value={value} index={1}>
          {nftsListIsError || nftsListIsFetching ? (
            <FetchStatusCheck
              isError={nftsListIsError}
              errorMessage={nftsListError?.message || 'Error fetching data'}
              isLoading={nftsListIsFetching}
            />
          ) : (
            <TableContainer>
              <Table>
                <TableHead>
                  <TableRow>
                    <TableCell>Token Symbol</TableCell>
                    <TableCell>Resource Type</TableCell>
                    <TableCell>Is Burned</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {nftsListData?.nfts.map((nft: any, index: number) =>
                    NftsList(nft)
                  )}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </TabPanel>
      </Box>
    );
  }

  return (
    <>
      <Grid item xs={12} md={12} lg={12}>
        <Box
          className="flex-container"
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            width: '100%',
          }}
        >
          <Typography
            variant="h4"
            style={{
              paddingBottom: theme.spacing(2),
            }}
          >
            My Assets
          </Typography>
          <Box
            style={{
              display: 'flex',
              gap: theme.spacing(1),
              marginBottom: theme.spacing(2),
            }}
          >
            <SendMoney />
            <Button variant="outlined" onClick={onClaimFreeCoins}>
              Claim Free Testnet Coins
            </Button>
            <ClaimBurn />
          </Box>
        </Box>
        <Divider />
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <Box
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}
        >
          <AccountBalance />
          <SelectAccount
            accountName={accountName}
            setAccountName={setAccountName}
            dataAccountsList={dataAccountsList}
          />
        </Box>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <InnerHeading>Account Details</InnerHeading>
          <AccountDetails />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <InnerHeading>Assets</InnerHeading>
          <AssetTabs />
        </StyledPaper>
      </Grid>
      <Grid item xs={12} md={12} lg={12}>
        <StyledPaper>
          <InnerHeading>Transactions</InnerHeading>
          <Transactions />
        </StyledPaper>
      </Grid>
    </>
  );
}

export default AssetVault;
