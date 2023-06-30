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

    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    # Mine some blocks until the UTXOs are scanned
    When miner MINER mines 15 new blocks
    Then the validator node VN is listed as registered
    Then the template "faucet" is listed as registered by the validator node VN

    # Create the sender account
    When I create an account ACCOUNT via the wallet daemon WALLET_D with 1000 free coins

    # Create a new Faucet component
    When I call function "mint" on template "faucet" using account ACCOUNT to pay fees via wallet daemon WALLET_D with args "amount_10000" and 3 outputs named "FAUCET"

    # Burn some tari in the base layer to have funds for fees in the sender account
    When I burn 10T on wallet WALLET with wallet daemon WALLET_D into commitment COMMITMENT with proof PROOF for ACCOUNT, range proof RANGEPROOF and claim public key CLAIM_PUBKEY
    When miner MINER mines 13 new blocks
    Then VN has scanned to height 45 within 10 seconds

    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS
    When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACCOUNT via the wallet daemon WALLET_D

    # Wait for the wallet daemon account monitor to update the sender account information

    # Fund the sender account with faucet tokens
    When I print the cucumber world
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACCOUNT" and 5 outputs named "TX1"
    ```
    let faucet = global!["FAUCET/components/TestFaucet"];
    let mut acc1 = global!["ACCOUNT/components/Account"];

    // get tokens from the faucet
    let faucet_bucket = faucet.take_free_coins();
    acc1.deposit(faucet_bucket);
    ```

    # Wait for the wallet daemon account monitor to update the sender account information

    When I check the balance of ACCOUNT on wallet daemon WALLET_D the amount is at least 1000
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

  @serial
  Scenario: Transfer tokens to existing account
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

    # Initialize different wallet daemons to simulate different users
    Given a wallet daemon WALLET_D connected to indexer IDX

    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    # Mine some blocks until the UTXOs are scanned
    When miner MINER mines 15 new blocks
    Then the validator node VN is listed as registered
    Then the template "faucet" is listed as registered by the validator node VN

    # Create the sender account with some tokens
    When I create an account ACCOUNT_1 via the wallet daemon WALLET_D with 1000 free coins
    When I create an account ACCOUNT_2 via the wallet daemon WALLET_D

    # Create a new Faucet component
    When I call function "mint" on template "faucet" using account ACCOUNT_1 to pay fees via wallet daemon WALLET_D with args "amount_10000" and 3 outputs named "FAUCET"

    # Burn some tari in the base layer to have funds for fees in the sender account
    When I burn 10T on wallet WALLET with wallet daemon WALLET_D into commitment COMMITMENT with proof PROOF for ACCOUNT_1, range proof RANGEPROOF and claim public key CLAIM_PUBKEY
    When miner MINER mines 13 new blocks
    Then VN has scanned to height 45 within 10 seconds

    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS
    When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACCOUNT_1 via the wallet daemon WALLET_D

    # Wait for the wallet daemon account monitor to update the sender account information

    # Fund the sender account with faucet tokens
    When I print the cucumber world
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACCOUNT_1" and 5 outputs named "TX1"
    ```
    let faucet = global!["FAUCET/components/TestFaucet"];
    let mut acc1 = global!["ACCOUNT_1/components/Account"];

    // get tokens from the faucet
    let faucet_bucket = faucet.take_free_coins();
    acc1.deposit(faucet_bucket);
    ```

    When I wait 3 seconds

    # Do the transfer from ACCOUNT_1 to another existing account
    When I transfer 50 tokens of resource FAUCET/resources/0 from account ACCOUNT_1 to public key ACCOUNT_2 via the wallet daemon WALLET_D named TRANSFER

    # Check that ACCOUNT_2 component now has funds
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACCOUNT_2" and 1 output named "TX2"
    ```
    let mut acc2 = global!["ACCOUNT_2/components/Account"];
    let faucet_resource = global!["FAUCET/resources/0"];
    acc2.balance(faucet_resource);
    ```
    When I print the cucumber world

  @serial
  Scenario: Confidential transfer to unexisting account
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

    # Create the sender account
    When I create an account ACC_1 via the wallet daemon WALLET_D with 1000 free coins

    When I check the balance of ACC_1 on wallet daemon WALLET_D the amount is at least 1000
    # Do the transfer from ACC_1 to the second account (which does not exist yet in the network)
    When I create a new key pair KEY_ACC_2
    When I do a confidential transfer of 50 from account ACC_1 to public key KEY_ACC_2 via the wallet daemon WALLET_D named TRANSFER

    When I print the cucumber world
