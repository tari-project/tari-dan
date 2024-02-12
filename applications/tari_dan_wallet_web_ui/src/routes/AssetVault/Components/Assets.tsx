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

import Box from "@mui/material/Box";
import Tab from "@mui/material/Tab";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import Tabs from "@mui/material/Tabs";
import Typography from "@mui/material/Typography";
import { useState } from "react";
import CopyToClipboard from "../../../Components/CopyToClipboard";
import FetchStatusCheck from "../../../Components/FetchStatusCheck";
import { DataTableCell } from "../../../Components/StyledComponents";
import { useAccountNFTsList, useAccountsGetBalances } from "../../../api/hooks/useAccounts";
import useAccountStore from "../../../store/accountStore";
import { shortenString } from "../../../utils/helpers";
import { AccountNftInfo, BalanceEntry } from "@tarilabs/typescript-bindings";

interface TabPanelProps {
  children?: React.ReactNode;
  index: number;
  value: number;
}

interface BalanceRowProps {
  token_symbol: string;
  resource_address: string;
  resource_type: string;
  balance: number;
  confidential_balance: number;
}

function BalanceRow({ token_symbol, resource_address, resource_type, balance, confidential_balance }: BalanceRowProps) {
  const { showBalance } = useAccountStore();
  return (
    <TableRow key={token_symbol || resource_address}>
      <DataTableCell>
        {shortenString(token_symbol || resource_address)}
        <CopyToClipboard copy={token_symbol || resource_address} />
      </DataTableCell>
      <DataTableCell>{resource_type}</DataTableCell>
      <DataTableCell>{showBalance ? balance : "*************"}</DataTableCell>
      <DataTableCell>{showBalance ? confidential_balance : "**************"}</DataTableCell>
    </TableRow>
  );
}

function NftsList({ metadata, is_burned }: AccountNftInfo) {
  return (
    <TableRow key={metadata}>
      <DataTableCell>{metadata}</DataTableCell>
      <DataTableCell>{is_burned}</DataTableCell>
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
          <Typography component="div">{children}</Typography>
        </Box>
      )}
    </div>
  );
}

function tabProps(index: number) {
  return {
    id: `asset-tab-${index}`,
    "aria-controls": `asset-tabpanel-${index}`,
  };
}

function Assets({ accountName }: { accountName: string }) {
  const [value, setValue] = useState(0);
  const { showBalance } = useAccountStore();

  const {
    data: balancesData,
    isError: balancesIsError,
    error: balancesError,
    isFetching: balancesIsFetching,
  } = useAccountsGetBalances(accountName);

  const {
    data: nftsListData,
    isError: nftsListIsError,
    error: nftsListError,
    isFetching: nftsListIsFetching,
  } = useAccountNFTsList(0, 10);

  const handleChange = (event: React.SyntheticEvent, newValue: number) => {
    setValue(newValue);
  };

  return (
    <Box sx={{ width: "100%" }}>
      <Box sx={{ borderBottom: 1, borderColor: "divider" }}>
        <Tabs value={value} onChange={handleChange} aria-label="account assets" variant="standard">
          <Tab label="Tokens" {...tabProps(0)} style={{ width: 150 }} />
          <Tab label="NFTs" {...tabProps(1)} style={{ width: 150 }} />
        </Tabs>
      </Box>
      <TabPanel value={value} index={0}>
        {balancesIsError || balancesIsFetching ? (
          <FetchStatusCheck
            isError={balancesIsError}
            errorMessage={balancesError?.message || "Error fetching data"}
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
                {/* {balancesData?.balances.map((balance: number, index: number) =>
                  BalanceRow(balance)
                )} */}
                {balancesData?.balances.map(
                  ({
                    vault_address,
                    resource_address,
                    balance,
                    resource_type,
                    confidential_balance,
                    token_symbol,
                  }: BalanceEntry) => {
                    return (
                      <BalanceRow
                        key={resource_address}
                        token_symbol={token_symbol || ""}
                        resource_address={resource_address}
                        resource_type={resource_type}
                        balance={balance}
                        confidential_balance={confidential_balance}
                      />
                    );
                  },
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
            errorMessage={nftsListError?.message || "Error fetching data"}
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
                {nftsListData?.nfts.map(({ metadata, is_burned }: AccountNftInfo) => {
                  return <NftsList metadata={metadata} is_burned={is_burned} />;
                })}
              </TableBody>
            </Table>
          </TableContainer>
        )}
      </TabPanel>
    </Box>
  );
}

export default Assets;
