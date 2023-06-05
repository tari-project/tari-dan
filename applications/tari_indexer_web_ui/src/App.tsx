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
import ValidatorNode from './routes/VN/ValidatorNode';
import Connections from './routes/Connections/Connections';
import RecentTransactions from './routes/RecentTransactions/RecentTransactions';
import MonitoredSubstates from './routes/MonitoredSubstates/MonitoredSubstates';
import MonitoredNftCollections from './routes/MonitoredNftCollections/MonitoredNftCollections';
import NftGallery from './routes/NftGallery/NftGallery';
import ErrorPage from './routes/ErrorPage';
import Layout from './theme/LayoutMain';

export const breadcrumbRoutes = [
  {
    label: 'Home',
    path: '/',
    dynamic: false,
  },
  {
    label: 'Connections',
    path: '/connections',
    dynamic: false,
  },
  {
    label: 'Transactions',
    path: '/transactions',
    dynamic: false,
  },
  {
    label: 'Monitored Substates',
    path: '/monitored_substates',
    dynamic: false,
  },
  {
    label: 'Monitored NFTs',
    path: '/nfts',
    dynamic: false,
  },
  {
    label: 'NFT Gallery',
    path: '/nfts/:resourceAddress',
    dynamic: true,
  },
  {
    label: 'Error',
    path: '*',
    dynamic: false,
  },
];
export default function App() {
  return (
    <div>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<ValidatorNode />} />
          <Route path="connections" element={<Connections />} />
          <Route path="monitored_substates" element={<MonitoredSubstates />} />
          <Route path="nfts" element={<MonitoredNftCollections />} />
          <Route path="nfts/:resourceAddress" element={<NftGallery />} />
          <Route path="transactions" element={<RecentTransactions />} />
          <Route path="*" element={<ErrorPage />} />
        </Route>
      </Routes>
    </div>
  );
}
