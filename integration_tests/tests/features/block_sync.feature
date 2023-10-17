# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Block Sync

  @serial
  Scenario: New validator node registers and syncs
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX
    When I create a key named K1 for WALLET_D

    # Initialize a VN
    Given a seed validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 5000 T
    When validator node VN sends a registration transaction allowing fee claims from wallet WALLET_D using key K1
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17 within 10 seconds
    And indexer IDX has scanned to height 17 within 10 seconds
    Then the validator node VN is listed as registered

    When indexer IDX connects to all other validators

    # Submit a few transactions
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account UNUSED1 via the wallet daemon WALLET_D
    When I create an account UNUSED2 via the wallet daemon WALLET_D
    When I create an account UNUSED3 via the wallet daemon WALLET_D

    When I wait for validator VN has leaf block height of at least 20

    # Start a new VN that needs to sync
    Given a validator node VN2 connected to base node BASE and wallet WALLET
    Given validator VN2 nodes connect to all other validators
    When indexer IDX connects to all other validators

    When validator node VN2 sends a registration transaction allowing fee claims from wallet WALLET_D using key K1
    When miner MINER mines 20 new blocks
    Then VN has scanned to height 37 within 20 seconds
    Then VN2 has scanned to height 37 within 10 seconds
    Then the validator node VN2 is listed as registered

    When I wait for validator VN2 has leaf block height of at least 20

    When I create an account UNUSED4 via the wallet daemon WALLET_D
    When I create an account UNUSED5 via the wallet daemon WALLET_D

    When I wait for validator VN has leaf block height of at least 26
    When I wait for validator VN2 has leaf block height of at least 26

