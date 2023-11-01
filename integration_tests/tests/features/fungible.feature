# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Fungible tokens

  @serial
  Scenario: Mint fungible tokens
    Given fees are disabled
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 10000 T
    When validator node VN sends a registration transaction
    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    # Mine some blocks until the UTXOs are scanned
    When miner MINER mines 14 new blocks
    Then VN has scanned to height 17
    Then the validator node VN is listed as registered
    Then the template "faucet" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I use an account key named K1

    # Create a new Faucet component
    When I call function "mint" on template "faucet" on VN with args "amount_10000" named "FAUCET"

    # Create two accounts to test sending the tokens
    When I create an account ACC_1 on VN
    When I create an account ACC_2 on VN

    # Submit a transaction manifest
    #    When I print the cucumber world
    When I submit a transaction manifest on VN with inputs "FAUCET, ACC_1" named "TX1" signed with key ACC_1
    ```
    let faucet = global!["FAUCET/components/TestFaucet"];
    let mut acc1 = global!["ACC_1/components/Account"];

    // get tokens from the faucet
    let faucet_bucket = faucet.take_free_coins();
    acc1.deposit(faucet_bucket);
    ```
    #    When I print the cucumber world
    # Submit a transaction manifest
    When I submit a transaction manifest on VN with inputs "FAUCET, TX1, ACC_2" named "TX2" signed with key ACC_1
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
