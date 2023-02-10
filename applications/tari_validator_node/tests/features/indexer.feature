# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Indexer node

  @current
  @serial
  Scenario: Indexer is able to connect to validator nodes
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VAL_1 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks
    When wallet WALLET has at least 1000000000 uT

    # VN registration
    When validator node VAL_1 sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VAL_1 is listed as registered

    # Register the "counter" template
    When validator node VAL_1 registers the template "counter"
    When miner MINER mines 20 new blocks
    Then the template "counter" is listed as registered by the validator node VAL_1

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Counter component
    When I create a component COUNTER_1 of template "counter" on VAL_1 using "new"

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE watching "COUNTER_1/components/Counter"
    Then the indexer IDX is connected
    When I print the cucumber world
    
    When I wait 5000 seconds
