# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Fungible tokens

  @serial
  Scenario: Mint fungible tokens
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks
    When wallet WALLET has at least 1000000000 uT

    # VN registration
    When validator node VN sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VN is listed as registered

    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    When miner MINER mines 20 new blocks
    Then the template "faucet" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Faucet component
    When I call function "mint" on template "faucet" on VN with args "10000" and 3 outputs named "FAUCET"

    # Create two accounts to test sending the tokens
    When I create an account ACC_1 on VN
    When I create an account ACC_2 on VN

    # Submit a transaction manifest
    When I print the cucumber world
    When I submit a transaction manifest on VN with inputs "FAUCET, ACC_1" and 3 outputs named "TX1"
        ```
        use template_faucet as TestFaucet;

        fn main() {
            let faucet = global!["FAUCET.components[0]"];
            let mut acc1 = global!["ACC_1.components[0]"];

            // get tokens from the faucet
            let faucet_bucket = faucet.take_free_coins();
            acc1.deposit(faucet_bucket);
        }
        ```
    When I print the cucumber world
    # Submit a transaction manifest
    When I submit a transaction manifest on VN with inputs "FAUCET, TX1, ACC_2" and 1 output named "TX2"
      ```
      use template_faucet as TestFaucet;

      fn main() {
        let mut acc1 = global!["TX1.components[0]"];
        let mut acc2 = global!["ACC_2.components[0]"];
        let faucet_resource = global!["FAUCET.resources[0]"];

        // Withdraw half of the tokens and send them to acc2
        let tokens = acc1.withdraw(faucet_resource, 500);
        acc2.deposit(tokens);
        acc2.balance(faucet_resource);
        acc1.balance(faucet_resource);
      }
      ```
