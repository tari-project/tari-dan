# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Committee scenarios

  # FIXME: when spawning VN2 the test is flaky
  @serial
  Scenario: Template registration and invocation in a 2-VN committee
    # Initialize a base node, wallet and miner
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize two validator nodes
    Given a validator node VAL_1 connected to base node BASE and wallet WALLET
    Given a validator node VAL_2 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks
    When wallet WALLET has at least 1000000000 uT

    # VN registration
    When validator node VAL_1 sends a registration transaction
    When validator node VAL_2 sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VAL_1 is listed as registered
    Then the validator node VAL_2 is listed as registered

    # Register the "counter" template
    When validator node VAL_1 registers the template "counter"
    When miner MINER mines 20 new blocks
    Then the template "counter" is listed as registered by the validator node VAL_1
    Then the template "counter" is listed as registered by the validator node VAL_2

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Counter component
    When I create a component COUNTER_1 of template "counter" on VAL_1 using "new"

    # The initial value of the counter must be 0
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "value" with 1 outputs the result is "0"
    When I invoke on VAL_2 on component COUNTER_1/components/Counter the method call "value" with 1 outputs the result is "0"

    # Increase the counter
    When I invoke on VAL_1 on component COUNTER_1/components/Counter the method call "increase" with 1 outputs named "TX1"

    # Check that the counter has been increased in both VNs
    When I invoke on VAL_1 on component TX1/components/Counter the method call "value" with 1 outputs the result is "1"
    When I invoke on VAL_2 on component TX1/components/Counter the method call "value" with 1 outputs the result is "1"

    # Uncomment the following lines to stop execution for manual inspection of the nodes
    # When I print the cucumber world
    # When I wait 5000 seconds



  Scenario: Leader failure
     # Initialize a base node, wallet and miner
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize two validator nodes
    Given a validator node VAL_1 connected to base node BASE and wallet WALLET
    Given a validator node VAL_2 connected to base node BASE and wallet WALLET
    Given a validator node VAL_3 connected to base node BASE and wallet WALLET
    Given a validator node VAL_4 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks
    When wallet WALLET has at least 1000000000 uT

    # VN registration
    When validator node VAL_1 sends a registration transaction
    When validator node VAL_2 sends a registration transaction
    When validator node VAL_3 sends a registration transaction
    When validator node VAL_4 sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VAL_1 is listed as registered
    Then the validator node VAL_2 is listed as registered
    Then the validator node VAL_3 is listed as registered
    Then the validator node VAL_4 is listed as registered


    # stop vn 4
       When I stop validator node VAL_4


    # send transactions. VAL_4 should be the leader in 1 of 4 transactions, so try 10 times, and all should succeed
    When I create an account ACC_1 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_2 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_3 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_4 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_5 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_6 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_7 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_8 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_9 on VAL_1
    Then the transaction succeeds
    When I create an account ACC_10 on VAL_1
    Then the transaction succeeds
