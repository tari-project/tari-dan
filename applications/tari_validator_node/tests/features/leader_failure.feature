# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Leader failure scenarios

  Scenario: Leader failure
    # Initialize a base node, wallet and miner
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize four validator nodes
    Given a seed validator node SEED_VN connected to base node BASE and wallet WALLET
    Given 4 validator nodes connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VNs send transactions
    When miner MINER mines 24 new blocks
    When wallet WALLET has at least 2000000000 uT

    # VNs registration
    When all validator nodes send registration transactions
    When miner MINER mines 20 new blocks
    Then all validator nodes are listed as registered

    # Stop VN 4
    # When I stop validator node VAL_4

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Need to wait a few seconds, so that all VNs get properly
    # registered
    When I wait 10 seconds

    # Send transactions, VAL_4 is offline, but should be the leader in 1 of 4
    # transactions, so we send 10 transactions. All should succeed
    When I create an account ACC_1 on VAL_1
    When I create an account ACC_2 on VAL_1
    When I create an account ACC_3 on VAL_1
    When I create an account ACC_4 on VAL_1
    When I create an account ACC_5 on VAL_1
    When I create an account ACC_6 on VAL_1
    When I create an account ACC_7 on VAL_1
    When I create an account ACC_8 on VAL_1
    When I create an account ACC_9 on VAL_1
    When I create an account ACC_10 on VAL_1

    # Wait a few seconds and then verify that all transactions have succeeded
    When I wait 3 seconds
    Then the transaction succeeds

    # Uncomment the following lines to stop execution for manual inspection of the nodes
    # When I print the cucumber world
    # When I wait 5000 seconds
