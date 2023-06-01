# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Leader failure scenarios

  @serial
  Scenario: Leader failure with single committee
    # Initialize a base node, wallet and miner
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize four validator nodes
    Given a seed validator node SEED_VN connected to base node BASE and wallet WALLET
    Given 4 validator nodes connected to base node BASE and wallet WALLET
    # TODO: Something isn't right here. All VNs should connect to the seed and peer sync.
    Given validator VAL_1 nodes connect to all other validators
    Given validator VAL_2 nodes connect to all other validators
    Given validator VAL_3 nodes connect to all other validators
    Given validator VAL_4 nodes connect to all other validators

    # The wallet must have some funds before the VNs send transactions
    When miner MINER mines 8 new blocks
    When wallet WALLET has at least 40000 T

    # VNs registration
    When all validator nodes send registration transactions
    When miner MINER mines 13 new blocks
    Then VAL_1 has scanned to height 18 within 10 seconds
    Then VAL_2 has scanned to height 18 within 10 seconds
    Then VAL_3 has scanned to height 18 within 10 seconds
    Then VAL_4 has scanned to height 18 within 10 seconds
    Then all validator nodes are listed as registered

    # Stop VN 4
    When I stop validator node VAL_4

    # A file-base CLI account must be created to sign future calls
    When I use an account key named K1

    # Send transactions, VAL_4 is offline, but should be the leader in 1 of 4
    # transactions, so we send 10 transactions. All should succeed
    When I create 10 accounts on VAL_1

    # Wait a few seconds and then verify that all transactions have succeeded
    Then all transactions succeed on all validator nodes

  # Uncomment the following lines to stop execution for manual inspection of the nodes
  # When I print the cucumber world
  # When I wait 5000 seconds

  @serial @ignore
  Scenario: Leader failure with multiple committees
    # Initialize a base node, wallet and miner
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize four validator nodes
    Given a seed validator node SEED_1 connected to base node BASE and wallet WALLET
    Given a seed validator node SEED_2 connected to base node BASE and wallet WALLET
    Given 10 validator nodes connected to base node BASE and wallet WALLET
    Given all validator nodes are connected to each other

    # The wallet must have some funds before the VNs send transactions
    When miner MINER mines 13 new blocks
    When wallet WALLET has at least 45000 T

    # VNs registration
    When all validator nodes send registration transactions
    When miner MINER mines 13 new blocks
    Then VAL_1 has scanned to height 23 within 10 seconds
    Then all validator nodes are listed as registered

    # Stop VN 4
    When I stop validator node VAL_4

    # Need to wait a few seconds, so that all VNs get properly
    # registered
    When I wait 5 seconds

    # Send transactions, VAL_4 is offline, but should be the leader in 1 of 4
    # transactions, so we send 10 transactions. All should succeed
    When I create 15 accounts on VAL_1

    # Wait a few seconds and then verify that all transactions have succeeded
    #    When I wait 3 seconds
    Then all transactions succeed on all validator nodes

# Uncomment the following lines to stop execution for manual inspection of the nodes
# When I print the cucumber world
# When I wait 5000 seconds