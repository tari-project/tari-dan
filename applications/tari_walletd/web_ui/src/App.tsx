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

import ConfidentialOutputListPage from "@/routes/ConfidentialOutputList/ConfidentialOutputListPage";
import StealthUtxoListPage from "@/routes/StealthUtxoList/StealthUtxoListPage";
import { useAuthMethod } from "@api/hooks/useAuth";
import { AuthDialog } from "@components/auth";
import Loading from "@components/Loading";
import AccessTokensLayout from "@routes/AccessTokens/AccessTokens";
import AccountDetails from "@routes/AccountDetails/AccountDetails";
import Accounts from "@routes/Accounts/Accounts";
import AddressBookPage from "@routes/AddressBook/AddressBookPage";
import ApiKeysLayout from "@routes/ApiKeys/ApiKeys";
import MyAssets from "@routes/AssetVault/Components/MyAssets";
import ErrorPage from "@routes/ErrorPage";
import Keys from "@routes/Keys/Keys";
import Manifest from "@routes/Manifest/Manifest";
import Onboarding from "@routes/Onboarding/Onboarding";
import SettingsPage from "@routes/Settings/Settings";
import SigningRequests from "@routes/SigningRequests/SigningRequests";
import Templates from "@routes/Templates/Templates";
import TransactionRequests from "@routes/TransactionRequests/TransactionRequests";
import TransactionDetails from "@routes/Transactions/TransactionDetails";
import Transactions from "@routes/Transactions/TransactionsLayout";
import ValidatorSetPage from "@routes/ValidatorSet/ValidatorSetPage";
import Wallet from "@routes/Wallet/Wallet";
import useAuthStore from "@store/authStore";
import useSettingsStore from "@store/settingsStore";
import Layout from "@theme/LayoutMain";
import { getClientInstance, isValidJwt, settingsGet } from "@utils/json_rpc";
import { useEffect, useState } from "react";
import { Route, Routes } from "react-router-dom";
import { ErrorNotificationProvider } from "./contexts/ErrorNotificationContext";

export const breadcrumbRoutes = [
  {
    label: "Home",
    path: "/",
    dynamic: false,
  },
  {
    label: "Authentication",
    path: "/auth",
    dynamic: false,
  },
  {
    label: "Webauthn",
    path: "/auth/webauthn",
    dynamic: false,
  },
  {
    label: "Accounts",
    path: "/accounts",
    dynamic: false,
  },
  {
    label: "Keys",
    path: "/keys",
    dynamic: false,
  },
  {
    label: "Access Tokens",
    path: "/access-tokens",
    dynamic: false,
  },
  {
    label: "Get access token",
    path: "/access-token",
    dynamic: false,
  },
  {
    label: "Account Details",
    path: "/accounts/:id",
    dynamic: true,
  },
  {
    label: "Transactions",
    path: "/transactions",
    dynamic: false,
  },
  {
    label: "Transaction Requests",
    path: "/transaction-requests",
    dynamic: false,
  },
  {
    label: "Signing Requests",
    path: "/signing-requests",
    dynamic: false,
  },
  {
    label: "Transaction Details",
    path: "/transactions/:id",
    dynamic: true,
  },
  {
    label: "Wallet",
    path: "/wallet",
    dynamic: false,
  },
  {
    label: "Settings",
    path: "/settings",
    dynamic: false,
  },
  {
    label: "Templates",
    path: "/templates",
    dynamic: false,
  },
  {
    label: "Manifest",
    path: "/manifest",
    dynamic: false,
  },
  {
    label: "Address Book",
    path: "/address-book",
    dynamic: false,
  },
  {
    label: "Validator Set",
    path: "/validator-set",
    dynamic: false,
  },
  // {
  //   label: "Flow Editor",
  //   path: "/flow-editor",
  //   dynamic: false,
  // },
  {
    label: "Stealth UTXOs",
    path: "/stealth-utxos/:resource_address",
    dynamic: true,
  },
  {
    label: "Confidential Outputs",
    path: "/confidential-outputs/:resource_address",
    dynamic: true,
  },
];

interface GuardedRouteProps {
  component: React.ComponentType<any>;
  redirect?: string;

  [key: string]: any;
}

// @ts-ignore
const GuardedRoute = ({ component: Component, redirect = "/", ...rest }: GuardedRouteProps) => {
  const { loggedIn, setLoggedIn, needsReauth, setNeedsReauth } = useAuthStore();
  const { data: authMethod, isError: authMethodsIsError, error: authMethodsError, isLoading } = useAuthMethod();
  const [hasToken, setHasToken] = useState<boolean | null>(null);
  const setAdvancedUiFeatures = useSettingsStore((s) => s.setAdvancedUiFeatures);

  useEffect(() => {
    async function initAuth() {
      const client = await getClientInstance();
      const token = client.getToken();

      if (isValidJwt(token)) {
        setHasToken(true);
        setLoggedIn(true);
        setNeedsReauth(false);
        return;
      }

      // We only want to reauth after page refresh, or if a reauth is needed (we had a previous auth 401 error)
      if (!needsReauth && hasToken !== null) {
        return;
      }
      // No valid JWT in memory (e.g. page reload). Try a silent refresh using
      // the server-side refresh token cookie before falling back to the dialog.
      try {
        const response = await client.authRefresh();
        if (response.token) {
          console.log("AUTH: refresh succeeded");
          client.setToken(response.token);
          setHasToken(true);
          setLoggedIn(true);
        } else {
          setHasToken(false);
        }
      } catch (e) {
        console.log("AUTH: refresh failed", e);
        setHasToken(false);
      } finally {
        setNeedsReauth(false);
      }
    }

    initAuth();
  }, [hasToken, needsReauth]);

  useEffect(() => {
    if (hasToken && loggedIn) {
      settingsGet().then((res) => {
        setAdvancedUiFeatures(res.advanced_ui_features);
      });
    }
  }, [hasToken, loggedIn, setAdvancedUiFeatures]);

  const handleOnAuthenticated = () => {
    setHasToken(true);
    setLoggedIn(true);
    setNeedsReauth(false);
  };

  if (isLoading || !authMethod || hasToken === null) {
    return <Loading />;
  }

  if (authMethodsIsError) {
    console.error("Error fetching auth method:", authMethodsError);
    return <div>Error fetching authentication method: {authMethodsError?.message || "Unknown error"}</div>;
  }

  if (!hasToken || !loggedIn) {
    console.log(`AUTH: User not authenticated, showing auth dialog for ${authMethod.method}`);
    return <AuthDialog open={true} authMethod={authMethod.method} onAuthenticated={handleOnAuthenticated} />;
  }

  return <Component {...rest} />;
};

function App() {
  return (
    <ErrorNotificationProvider>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<GuardedRoute component={MyAssets} />} />
          <Route path="register" element={<GuardedRoute component={MyAssets} />} />
          <Route path="onboarding" element={<GuardedRoute component={Onboarding} />} />
          <Route path="accounts" element={<GuardedRoute redirect="/accounts" component={Accounts} />} />
          <Route path="accounts/:id" element={<GuardedRoute redirect="/accounts" component={AccountDetails} />} />
          <Route path="keys" element={<GuardedRoute redirect="/keys" component={Keys} />} />
          <Route
            path="access-tokens"
            element={<GuardedRoute redirect="/access-tokens" component={AccessTokensLayout} />}
          />
          <Route path="api-keys" element={<GuardedRoute redirect="/api-keys" component={ApiKeysLayout} />} />
          <Route path="transactions" element={<GuardedRoute redirect="/transactions" component={Transactions} />} />
          <Route
            path="transaction-requests"
            element={<GuardedRoute redirect="/transaction-requests" component={TransactionRequests} />}
          />
          <Route
            path="signing-requests"
            element={<GuardedRoute redirect="/signing-requests" component={SigningRequests} />}
          />
          <Route path="wallet" element={<GuardedRoute redirect="/wallet" component={Wallet} />} />
          <Route
            path="transactions/:id"
            element={<GuardedRoute redirect="/transactions" component={TransactionDetails} />}
          />
          <Route path="settings" element={<GuardedRoute redirect="/settings" component={SettingsPage} />} />
          <Route path="templates" element={<GuardedRoute redirect="/templates" component={Templates} />} />
          <Route path="manifest" element={<GuardedRoute redirect="/manifest" component={Manifest} />} />
          <Route path="address-book" element={<GuardedRoute redirect="/address-book" component={AddressBookPage} />} />
          <Route
            path="validator-set"
            element={<GuardedRoute redirect="/validator-set" component={ValidatorSetPage} />}
          />
          {/*<Route path="flow-editor" element={<GuardedRoute redirect="/flow-editor" component={FlowEditor} />} />*/}
          <Route
            path="stealth-utxos/:resource_address"
            element={<GuardedRoute redirect="/stealth-utxos" component={StealthUtxoListPage} />}
          />
          <Route
            path="confidential-outputs/:resource_address"
            element={<GuardedRoute component={ConfidentialOutputListPage} />}
          />
          <Route path="*" element={<ErrorPage />} />
        </Route>
      </Routes>
    </ErrorNotificationProvider>
  );
}

export default App;
