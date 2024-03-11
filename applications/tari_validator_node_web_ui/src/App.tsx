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

import { Routes, Route } from "react-router-dom";
import Mempool from "./routes/Mempool/Mempool";
import Committees from "./routes/Committees/CommitteesLayout";
import ValidatorNode from "./routes/VN/ValidatorNode";
import Connections from "./routes/Connections/Connections";
import Fees from "./routes/Fees/Fees";
import Blocks from "./routes/Blocks/Blocks";
import Templates from "./routes/Templates/Templates";
import ValidatorNodes from "./routes/ValidatorNodes/ValidatorNodes";
import ErrorPage from "./routes/ErrorPage";
import TemplateFunctions from "./routes/VN/Components/TemplateFunctions";
import Layout from "./theme/LayoutMain";
import CommitteeMembers from "./routes/Committees/CommitteeMembers";
import { createContext, useState, useEffect } from "react";
import { getEpochManagerStats, getIdentity, getShardKey } from "./utils/json_rpc";
import TransactionDetails from "./routes/Transactions/TransactionDetails";
import BlockDetails from "./routes/Blocks/BlockDetails";
import type {
  GetEpochManagerStatsResponse,
  GetIdentityResponse,
} from "@tariproject/typescript-bindings/validator-node-client";

interface IContext {
  epoch?: GetEpochManagerStatsResponse;
  identity?: GetIdentityResponse;
  shardKey?: string | null;
  error?: string;
}

export const VNContext = createContext<IContext>({
  epoch: undefined,
  identity: undefined,
  shardKey: undefined,
  error: "",
});

export const breadcrumbRoutes = [
  {
    label: "Home",
    path: "/",
    dynamic: false,
  },
  {
    label: "Committees",
    path: "/committees",
    dynamic: false,
  },
  {
    label: "Connections",
    path: "/connections",
    dynamic: false,
  },
  {
    label: "Fees",
    path: "/fees",
    dynamic: false,
  },
  {
    label: "Transactions",
    path: "/transactions",
    dynamic: false,
  },
  {
    label: "Blocks",
    path: "/blocks",
    dynamic: false,
  },
  {
    label: "Blocks",
    path: "/blocks/:blockId",
    dynamic: true,
  },
  {
    label: "Templates",
    path: "/templates",
    dynamic: false,
  },
  {
    label: "Validator Nodes",
    path: "/vns",
    dynamic: false,
  },
  {
    label: "Mempool",
    path: "/mempool",
    dynamic: false,
  },
  {
    label: "Transactions",
    path: "/transactions/:payloadId",
    dynamic: true,
  },
  {
    label: "Template",
    path: "/templates/:address",
    dynamic: true,
  },
  {
    label: "Committee",
    path: "/committees/:address",
    dynamic: true,
  },
];

export default function App() {
  const [epoch, setEpoch] = useState<GetEpochManagerStatsResponse | undefined>(undefined);
  const [identity, setIdentity] = useState<GetIdentityResponse | undefined>(undefined);
  const [shardKey, setShardKey] = useState<string | null>(null);
  const [error, setError] = useState("");

  // Refresh every 2 minutes
  const refreshEpoch = (epoch: GetEpochManagerStatsResponse | undefined) => {
    getEpochManagerStats()
      .then((response) => {
        console.log("getEpochManagerStats", response);
        if (response.current_epoch !== epoch?.current_epoch) {
          setEpoch(response);
        }
      })
      .catch((reason) => {
        console.error(reason);
        setError("Json RPC error, please check console");
      });
  };
  useEffect(() => {
    const id = window.setInterval(() => refreshEpoch(epoch), 2 * 60 * 1000);
    return () => {
      window.clearInterval(id);
    };
  }, [epoch]);
  // Initial fetch
  useEffect(() => {
    refreshEpoch(undefined);
    getIdentity()
      .then((response) => {
        setIdentity(response);
      })
      .catch((reason) => {
        console.log(reason);
        setError("Json RPC error, please check console");
      });
  }, []);
  // Get shard key.
  useEffect(() => {
    if (epoch !== undefined && identity !== undefined) {
      // The *10 is from the hardcoded constant in VN.
      getShardKey({ height: epoch.current_epoch * 10, public_key: identity.public_key }).then((response) => {
        setShardKey(response.shard_key);
      });
    }
  }, [epoch, identity]);

  return (
    <>
      <VNContext.Provider value={{ epoch, identity, shardKey, error }}>
        <Routes>
          <Route path="/" element={<Layout />} errorElement={<ErrorPage />}>
            <Route index element={<ValidatorNode />} />
            <Route path="committees" element={<Committees />} />
            <Route path="connections" element={<Connections />} />
            <Route path="fees" element={<Fees />} />
            <Route path="blocks" element={<Blocks />} />
            <Route path="templates" element={<Templates />} />
            <Route path="vns" element={<ValidatorNodes />} />
            <Route path="mempool" element={<Mempool />} />
            <Route path="transactions/:transactionHash" element={<TransactionDetails />} />
            <Route path="blocks/:blockId" element={<BlockDetails />} />
            <Route path="templates/:address" element={<TemplateFunctions />} />
            <Route path="committees/:address" element={<CommitteeMembers />} />
            <Route path="*" element={<ErrorPage />} />
          </Route>
        </Routes>
      </VNContext.Provider>
    </>
  );
}
