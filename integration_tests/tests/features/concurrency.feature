# Copyright 2024 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Concurrency

  @serial
  Scenario: Concurrent calls to the Counter template
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
    When I print the cucumber world

    # Send multiple concurrent transactions to increase the counter
    # TODO: when concurrency is fully working, call it with "2 times" or higher
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "increase" concurrently 1 times
    When I print the cucumber world

    # Check that the counter has been increased
    # TODO: uncomment when concurrency is fully working
    # When I invoke on VAL_1 on component TX1/components/Counter the method call "value" the result is "2"  