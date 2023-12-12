# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Substates

  @serial
  Scenario: Transactions with DOWN local substates are rejected
    Given fees are disabled
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VAL_1 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 20 T

    # VN registration
    When validator node VAL_1 sends a registration transaction

    # Register the "counter" template
    When validator node VAL_1 registers the template "counter"
    When miner MINER mines 13 new blocks
    Then VAL_1 has scanned to height 16
    Then the validator node VAL_1 is listed as registered
    Then the template "counter" is listed as registered by the validator node VAL_1

    # A file-base CLI account must be created to sign future calls
    When I use an account key named K1

    # Create a new Counter component
    When I create a component COUNTER_1 of template "counter" on VAL_1 using "new"

    # Increase the counter an check the value
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "increase" named "TX1"
    When I invoke on VAL_1 on component TX1/components/Counter the method call "value" the result is "1"

    # We should get an error if we se as inputs the same component version thas has already been downed from previous transactions
    # We can achieve this by reusing inputs from COUNTER_1 instead of the most recent TX1
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "increase" named "TX2" the result is error "Shard was rejected"

    # Check that the counter has NOT been increased by the previous erroneous transaction
    When I invoke on VAL_1 on component TX1/components/Counter the method call "value" the result is "1"


