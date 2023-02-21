# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Counter template

  @serial
  Scenario: Counter template registration and invocation
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VAL_1 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 20000000 uT

    # VN registration
    When validator node VAL_1 sends a registration transaction

    # Register the "counter" template
    When validator node VAL_1 registers the template "counter"
    When miner MINER mines 13 new blocks
    Then the validator node VAL_1 is listed as registered
    Then the template "counter" is listed as registered by the validator node VAL_1

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Counter component
    When I create a component COUNTER_1 of template "counter" on VAL_1 using "new"
    When I print the cucumber world

    # The initial value of the counter must be 0
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "value" with 1 outputs the result is "0"
    When I print the cucumber world

    # Increase the counter
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "increase" with 1 outputs named "TX1"
    When I print the cucumber world

    # Check that the counter has been increased
    When I invoke on VAL_1 on component TX1/components/Counter the method call "value" with 1 outputs the result is "1"
    When I print the cucumber world

    # Uncomment the following lines to stop execution for manual inspection of the nodes
    # When I print the cucumber world
    # When I wait 5000 seconds
    

