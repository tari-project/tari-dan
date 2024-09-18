# Copyright 2024 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@fungible
Feature: Fungible tokens

  @serial
  Scenario: Mint fungible tokens

    ##### Setup
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a validator node
    Given a validator node VN connected to base node BASE and wallet daemon WALLET_D

    # Fund wallet to send VN registration tx
    When miner MINER mines 10 new blocks
    When wallet WALLET has at least 2000 T
    When validator node VN sends a registration transaction to base wallet WALLET
    When miner MINER mines 16 new blocks
    Then the validator node VN is listed as registered

    # Initialize indexer and connect wallet daemon
    Given an indexer IDX connected to base node BASE
    Given a wallet daemon WALLET_D connected to indexer IDX

    # Register the "faucet" template
    When base wallet WALLET registers the template "faucet"
    When miner MINER mines 20 new blocks
    Then VN has scanned to height 43
    Then the template "faucet" is listed as registered by the validator node VN

    ##### Scenario
    # Create two accounts to test deposit the tokens
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins

    # Create a new Faucet component
    When I call function "mint" on template "faucet" using account ACC1 to pay fees via wallet daemon WALLET_D with args "amount_10000" named "FAUCET"

    # Deposit tokens in first account
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACC1" named "TX1"
  ```
  let faucet = global!["FAUCET/components/TestFaucet"];
  let mut acc1 = global!["ACC1/components/Account"];

  // get tokens from the faucet
  let faucet_bucket = faucet.take_free_coins();
  acc1.deposit(faucet_bucket);
  ```

    # Move tokens from first to second account
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, TX1, ACC2" named "TX2"
  ```
  let mut acc1 = global!["TX1/components/Account"];
  let mut acc2 = global!["ACC2/components/Account"];
  let faucet_resource = global!["FAUCET/resources/0"];

  // Withdraw 50 of the tokens and send them to acc2
  let tokens = acc1.withdraw(faucet_resource, Amount(50));
  acc2.deposit(tokens);
  acc2.balance(faucet_resource);
  acc1.balance(faucet_resource);
  ```
