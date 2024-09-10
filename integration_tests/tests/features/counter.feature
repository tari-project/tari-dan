# Copyright 2024 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@counter
Feature: Counter template

  @serial
  Scenario: Counter template registration and invocation

    # Initialize a base node, wallet, miner and VN
    Given fees are disabled
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VAL connected to base node BASE and wallet daemon WALLET_D

    # Fund wallet to send VN registration tx
    When miner MINER mines 10 new blocks
    When wallet WALLET has at least 2000 T
    When validator node VAL sends a registration transaction to base wallet WALLET
    When miner MINER mines 16 new blocks
    Then the validator node VAL is listed as registered

    # Initialize indexer and connect wallet daemon
    Given an indexer IDX connected to base node BASE
    Given a wallet daemon WALLET_D connected to indexer IDX

    # Register the "counter" template
    When base wallet WALLET registers the template "counter"
    When miner MINER mines 20 new blocks
    Then VAL has scanned to height 43

    # Create the sender account
    When I create an account ACC via the wallet daemon WALLET_D with 10000 free coins

    # The initial value of the counter must be 0
    When I call function "new" on template "counter" using account ACC to pay fees via wallet daemon WALLET_D named "COUNTER"
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/Counter the method call "value" the result is "0"

    # Increase the counter
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/Counter the method call "increase"

    # Check that the counter has been increased
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/Counter the method call "value" the result is "1"

# Uncomment the following lines to stop execution for manual inspection of the nodes
# When I print the cucumber world
# When I wait 5000 seconds
