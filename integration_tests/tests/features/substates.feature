# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@substates
Feature: Substates

  @dev
  @serial
  Scenario: Transactions with DOWN local substates are rejected
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VAL_1 connected to base node BASE and wallet daemon WALLET_D

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 20 T

    # VN registration
    When validator node VAL_1 sends a registration transaction to base wallet WALLET

    # Register the "counter" template
    When base wallet WALLET registers the template "counter"
    When miner MINER mines 13 new blocks
    Then VAL_1 has scanned to height 16
    Then the validator node VAL_1 is listed as registered
    Then the template "counter" is listed as registered by the validator node VAL_1

    # Initialize indexer and connect wallet daemon
    Given an indexer IDX connected to base node BASE
    Given a wallet daemon WALLET_D connected to indexer IDX

    # A file-base CLI account must be created to sign future calls
    When I create an account ACC via the wallet daemon WALLET_D with 10000 free coins

    # Create a new Counter component
    When I call function "new" on template "counter" using account ACC to pay fees via wallet daemon WALLET_D named "COUNTER_1"
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER_1/components/Counter the method call "value" the result is "0"

    # Increase the counter and check the value
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER_1/components/Counter the method call "increase" named "TX1"
    When I invoke on wallet daemon WALLET_D on account ACC on component TX1/components/Counter the method call "value" the result is "1"

    # We should get an error if we se as inputs the same component version thas has already been downed from previous transactions
    # We can achieve this by reusing inputs from COUNTER_1 instead of the most recent TX1
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER_1/components/Counter the method call "increase" named "TX2", I expect it to fail with "Substate .*? is DOWN"

    # Check that the counter has NOT been increased by the previous erroneous transaction
    When I invoke on wallet daemon WALLET_D on account ACC on component TX1/components/Counter the method call "value" the result is "1"


