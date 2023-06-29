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

import { Routes, Route } from 'react-router-dom';
import Accounts from './routes/Accounts/Accounts';
import AccountDetails from './routes/AccountDetails/AccountDetails';
import Keys from './routes/Keys/Keys';
import ErrorPage from './routes/ErrorPage';
import Wallet from './routes/Wallet/Wallet';
import Layout from './theme/LayoutMain';
import TransactionsLayout from './routes/Transactions/TransactionsLayout';
import TransactionDetails from './routes/Transactions/TransactionDetails';

export const breadcrumbRoutes = [
  {
    label: 'Home',
    path: '/',
    dynamic: false,
  },
  {
    label: 'Accounts',
    path: '/accounts',
    dynamic: false,
  },
  {
    label: 'Keys',
    path: '/keys',
    dynamic: false,
  },
  {
    label: 'Transactions',
    path: '/transactions',
    dynamic: false,
  },
  {
    label: 'Account Details',
    path: '/accounts/:name',
    dynamic: true,
  },
  {
    label: 'Transaction Details',
    path: '/transactions/:id',
    dynamic: true,
  },
];

function App() {
  return (
    <div>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<Wallet />} />
          <Route path="accounts" element={<Accounts />} />
          <Route path="accounts/:name" element={<AccountDetails />} />
          <Route path="keys" element={<Keys />} />
          <Route path="transactions" element={<TransactionsLayout />} />
          <Route
            path="transactions/:id"
            element={<TransactionDetails />}
            // loader={transactionLoader}
          />
          <Route path="*" element={<ErrorPage />} />
        </Route>
      </Routes>
    </div>
  );
}

export default App;
