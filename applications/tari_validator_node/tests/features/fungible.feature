# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Fungible tokens

  @serial @current
  Scenario: Mint fungible tokens
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then the validator node VN is listed as registered

    # VN registration
    When validator node VN sends a registration transaction

    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    # Mine some blocks until the UTXOs are scanned
    When miner MINER mines 15 new blocks
    Then VN has scanned to height 18 within 10 seconds
    Then the validator node VN is listed as registered
    Then the template "faucet" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Faucet component
    When I call function "mint" on template "faucet" on VN with args "amount_10000" and 3 outputs named "FAUCET" with new resource "test"

    # Create two accounts to test sending the tokens
    When I create an account ACC_1 on VN
    When I create an account ACC_2 on VN

    # Submit a transaction manifest
    When I print the cucumber world
    When I submit a transaction manifest on VN with inputs "FAUCET, ACC_1" and 3 outputs named "TX1"
        ```
            let faucet = global!["FAUCET/components/TestFaucet"];
            let mut acc1 = global!["ACC_1/components/Account"];

            // get tokens from the faucet
            let faucet_bucket = faucet.take_free_coins();
            acc1.deposit(faucet_bucket);
        ```
    When I print the cucumber world
    # Submit a transaction manifest
    When I submit a transaction manifest on VN with inputs "FAUCET, TX1, ACC_2" and 1 output named "TX2"
      ```
        let mut acc1 = global!["TX1/components/Account"];
        let mut acc2 = global!["ACC_2/components/Account"];
        let faucet_resource = global!["FAUCET/resources/0"];

        // Withdraw 50 of the tokens and send them to acc2
        let tokens = acc1.withdraw(faucet_resource, Amount(50));
        acc2.deposit(tokens);
        acc2.balance(faucet_resource);
        acc1.balance(faucet_resource);
      ```
