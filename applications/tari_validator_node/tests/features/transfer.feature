# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Account transfers

  @serial
  Scenario: Transfer tokens to unexisting account
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

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet
    When I wait 3 seconds

    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    # Mine some blocks until the UTXOs are scanned
    When miner MINER mines 15 new blocks
    Then the validator node VN is listed as registered
    Then the template "faucet" is listed as registered by the validator node VN

    # Create the sender account
    When I create an account ACCOUNT via the wallet daemon WALLET_D

    # Create a new Faucet component
    # When I call function "mint" on template "faucet" on VN with args "amount_10000" and 3 outputs named "FAUCET" with new resource "test"
    When I call function "mint" on template "faucet" using account ACCOUNT to pay fees via wallet daemon WALLET_D with args "amount_10000" and 3 outputs named "FAUCET"

    # Burn some tari in the base layer to have funds for fees in the sender account
    When I burn 10T on wallet WALLET with wallet daemon WALLET_D into commitment COMMITMENT with proof PROOF for ACCOUNT, range proof RANGEPROOF and claim public key CLAIM_PUBKEY
    When miner MINER mines 13 new blocks
    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS
    When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACCOUNT via the wallet daemon WALLET_D

    # Wait for the wallet daemon account monitor to update the sender account information
    When I wait 10 seconds

    # Fund the sender account with faucet tokens
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACCOUNT" and 5 outputs named "TX1"
    ```
    let faucet = global!["FAUCET/components/TestFaucet"];
    let mut acc1 = global!["ACCOUNT/components/Account"];

    // get tokens from the faucet
    let faucet_bucket = faucet.take_free_coins();
    acc1.deposit(faucet_bucket);
    ```
    # Wait for the wallet daemon account monitor to update the sender account information
    When I wait 10 seconds

    # Do the transfer from ACCOUNT to the second account (which does not exist yet in the network)
    When I create a new key pair KEY_ACC_2
    When I transfer 50 tokens of resource FAUCET/resources/0 from account ACCOUNT to public key KEY_ACC_2 via the wallet daemon WALLET_D named TRANSFER

    # Check that ACC_2 component was created and has funds
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, TRANSFER" and 1 output named "TX2"
    ```
    let mut acc2 = global!["TRANSFER/components/Account"];
    let faucet_resource = global!["FAUCET/resources/0"];
    acc2.balance(faucet_resource);
    ```
    When I print the cucumber world
