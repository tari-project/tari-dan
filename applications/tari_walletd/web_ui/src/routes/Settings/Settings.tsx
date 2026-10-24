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

import { useAuthMethod } from "@api/hooks/useAuth";
import PageHeading from "@components/PageHeading";
import { StyledPaper } from "@components/StyledComponents";
import Grid from "@mui/material/Grid";
import AdvancedFeatures from "@routes/Settings/Components/AdvancedFeatures";
import DecryptUtxoBalance from "@routes/Settings/Components/DecryptUtxoBalance";
import GeneralSettings from "@routes/Settings/Components/GeneralSettings";
import Passkeys from "@routes/Settings/Components/Passkeys";
import SettingsTabs from "@routes/Settings/Components/SettingsTabs";
import ViewVaultBalance from "@routes/Settings/Components/ViewVaultBalance";
import AccessTokens from "@routes/Wallet/Components/AccessTokens";
import Accounts from "@routes/Wallet/Components/Accounts";
import Keys from "@routes/Wallet/Components/Keys";

export interface ISettingsMenu {
  label: string;
  title: string;
  content: React.ReactNode;
}

function SettingsPage() {
  const { data: authMethod } = useAuthMethod();
  const isWebAuthn = authMethod?.method === "webauthn";

  const menuItems: ISettingsMenu[] = [
    {
      label: "General",
      title: "General Settings",
      content: <GeneralSettings />,
    },
    {
      label: "Accounts",
      title: "Manage Accounts",
      content: <Accounts />,
    },
    {
      label: "Keys",
      title: "Manage Keys",
      content: <Keys />,
    },
    {
      label: "Access Tokens",
      title: "Manage Access Tokens",
      content: <AccessTokens />,
    },
    ...(isWebAuthn
      ? [
          {
            label: "Passkeys",
            title: "Manage Passkeys",
            content: <Passkeys />,
          },
        ]
      : []),
    {
      label: "View Vault Balance",
      title: "View Vault Balance",
      content: <ViewVaultBalance />,
    },
    {
      label: "Decrypt UTXO",
      title: "Decrypt UTXO Balance",
      content: <DecryptUtxoBalance />,
    },
    {
      label: "Advanced",
      title: "Advanced Features",
      content: <AdvancedFeatures />,
    },
  ];

  return (
    <>
      <Grid size={12}>
        <PageHeading>Settings</PageHeading>
      </Grid>
      <Grid size={12}>
        <StyledPaper>
          <SettingsTabs menuItems={menuItems} />
        </StyledPaper>
      </Grid>
    </>
  );
}

export default SettingsPage;
