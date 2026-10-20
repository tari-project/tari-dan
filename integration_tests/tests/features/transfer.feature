# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@concurrent
@transfer
Feature: Account transfers

  Scenario: Transfer tokens to account that does not previously exist
    Given a network with registered validator VN and wallet daemon WALLET_D

    # Publish the "faucet" template
    When I create an account ACC via the wallet daemon WALLET_D with 10000 XTR
    When wallet daemon WALLET_D publishes the template "faucet" using account ACC

    # Create the sender account
    When I create an account ACCOUNT via the wallet daemon WALLET_D with 10000 XTR

    # Create a new faucet component
    When I call function "mint" on template "faucet" using account ACCOUNT to pay fees via wallet daemon WALLET_D with args "amount_10000" named "FAUCET"

    # Burn some tari in the base layer to have funds for fees in the sender account
    When I burn 10T on wallet MINOTARI_WALLET to proof BURN_PROOF for account ACCOUNT using wallet daemon WALLET_D
    When miner MINER mines 13 new blocks
    Then VN has scanned to at least height 40
    Then indexer INDEXER has scanned to at least height 40

    When I wait for proof BURN_PROOF to confirm on wallet MINOTARI_WALLET
    When miner MINER mines to the next epoch
    Then VN is past the epoch burn proof BURN_PROOF was mined in

    When I claim burn BURN_PROOF and spend it into account ACCOUNT using wallet daemon WALLET_D

    # Wait for the wallet daemon account monitor to update the sender account information

    # Fund the sender account with faucet tokens
    When I print the cucumber world
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACCOUNT" named "TX1"
  """
  let faucet = global!["FAUCET/components/faucet"];
  let mut acc = global!["ACCOUNT/accounts/ACCOUNT"];

  // get tokens from the faucet
  let faucet_bucket = faucet.take_free_coins();
  acc.deposit(faucet_bucket);
  """

    # Wait for the wallet daemon account monitor to update the sender account information

    When I check the balance of ACCOUNT on wallet daemon WALLET_D the amount is at least 10000
    # Do the transfer from ACCOUNT to the second account (which does not exist yet in the network)
    When I create an account ACC_2 via the wallet daemon WALLET_D
    When I print the cucumber world
    When I transfer 50 tokens of resource FAUCET/resources/FAUCET from account ACCOUNT to account ACC_2 via the wallet daemon WALLET_D named TRANSFER

    When I print the cucumber world
    # Check that ACC_2 component was created and has funds
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, TRANSFER" named "TX2"
  """
  let mut acc2 = global!["TRANSFER/accounts/ACC_2"];
  let faucet_resource = global!["FAUCET/resources/FAUCET"];
  acc2.balance(faucet_resource);
  """
    When I print the cucumber world

  Scenario: Transfer tokens to existing account
    Given a network with registered validator VN and wallet daemon WALLET_D

    # Publish the "faucet" template
    When I create an account ACC via the wallet daemon WALLET_D with 10000 XTR
    When wallet daemon WALLET_D publishes the template "faucet" using account ACC

    # Create the sender account with some tokens
    When I create an account ACCOUNT_1 via the wallet daemon WALLET_D with 10000 XTR
    When I create an account ACCOUNT_2 via the wallet daemon WALLET_D

    # Create a new faucet component
    When I call function "mint" on template "faucet" using account ACCOUNT_1 to pay fees via wallet daemon WALLET_D with args "amount_10000" named "FAUCET"

    # Burn some tari in the base layer to have funds for fees in the sender account
    When I burn 10T on wallet MINOTARI_WALLET to proof BURN_PROOF for account ACCOUNT_1 using wallet daemon WALLET_D
    When miner MINER mines 13 new blocks
    Then VN has scanned to at least height 40
    Then indexer INDEXER has scanned to at least height 40

    When I wait for proof BURN_PROOF to confirm on wallet MINOTARI_WALLET
    When miner MINER mines to the next epoch
    Then VN is past the epoch burn proof BURN_PROOF was mined in

    When I claim burn BURN_PROOF and spend it into account ACCOUNT_1 using wallet daemon WALLET_D

    # Fund the sender account with faucet tokens
    When I print the cucumber world
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACCOUNT_1" named "TX1"
  """
  let faucet = global!["FAUCET/components/faucet"];
  let mut acc1 = global!["ACCOUNT_1/accounts/ACCOUNT_1"];

  // get tokens from the faucet
  let faucet_bucket = faucet.take_free_coins();
  acc1.deposit(faucet_bucket);
  """

    # Wait for the wallet daemon account monitor to update the sender account information
    When I check the balance of ACCOUNT_1 on wallet daemon WALLET_D the amount is at least 10000

    # Do the transfer from ACCOUNT_1 to another existing account
    When I transfer 50 tokens of resource FAUCET/resources/FAUCET from account ACCOUNT_1 to account ACCOUNT_2 via the wallet daemon WALLET_D named TRANSFER

    # Check that ACCOUNT_2 component now has funds
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, TRANSFER" named "TX2"
  """
  let mut acc2 = global!["TRANSFER/accounts/ACCOUNT_2"];
  let faucet_resource = global!["FAUCET/resources/FAUCET"];
  acc2.balance(faucet_resource);
  """
    When I print the cucumber world

  Scenario: Stealth transfer to account that does not previously exist
    Given a network with registered validator VN and wallet daemon WALLET_D

    Then VN has scanned to at least height 27
    Then indexer INDEXER has scanned to at least height 27

    # Create the sender account with initial funds
    When I create an account ACC_1 via the wallet daemon WALLET_D with 10000 XTR
    # Create the receiver account (which will only be created in the wallet DB and does not exist on the network)
    When I create an account ACC_2 via the wallet daemon WALLET_D

    When I check the balance of ACC_1 on wallet daemon WALLET_D the amount is at least 9000
    # Do the transfer from ACC_1 to the second account (which does not exist yet in the network)
    When I do a stealth transfer of 50 from account ACC_1 to account ACC_2 via the wallet daemon WALLET_D named TRANSFER

    When I check the balance of ACC_2 on wallet daemon WALLET_D the amount is exactly 50

    Then I check that outputs TRANSFER contain 1 unspent outputs

    When I print the cucumber world
