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

import React from 'react';
import ReactDOM from 'react-dom/client';
import './index.css';
import reportWebVitals from './reportWebVitals';
import { createBrowserRouter, RouterProvider } from 'react-router-dom';
import ValidatorNode from './routes/VN/ValidatorNode';
import Transaction, {
  transactionLoader,
} from './routes/Transaction/Transaction';
import App from './App';
import Committees from './routes/Committees/Committees';
import Connections from './routes/Connections/Connections';
import Mempool from './routes/Mempool/Mempool';
import RecentTransactions from './routes/RecentTransactions/RecentTransactions';
import Templates from './routes/Templates/Templates';
import ValidatorNodes from './routes/ValidatorNodes/ValidatorNodes';
import ErrorPage from './routes/ErrorPage';
import TemplateFunctions from './routes/VN/Components/TemplateFunctions';

const router = createBrowserRouter([
  {
    path: '*',
    element: <App />,
    errorElement: <ErrorPage />,
    children: [
      {
        path: 'connections',
        element: <Connections />,
      },
      {
        path: 'transactions',
        element: <RecentTransactions />,
      },
      {
        path: 'templates',
        element: <Templates />,
      },
      {
        path: 'vns',
        element: <ValidatorNodes />,
      },
      {
        path: 'app',
        element: <App />,
      },
      {
        path: 'mempool',
        element: <Mempool />,
      },
      {
        path: 'committees',
        element: <Committees />,
      },
      {
        path: 'transaction/:payloadId',
        element: <Transaction />,
        loader: transactionLoader,
      },
      {
        path: 'template/:address',
        element: <TemplateFunctions />,
      },
    ],
  },
]);

const root = ReactDOM.createRoot(
  document.getElementById('root') as HTMLElement
);
root.render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>
);

// If you want to start measuring performance in your app, pass a function
// to log results (for example: reportWebVitals(console.log))
// or send to an analytics endpoint. Learn more: https://bit.ly/CRA-vitals
reportWebVitals();
