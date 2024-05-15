# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause
@claim_fees
Feature: Claim Fees
  @serial @fixed
  Scenario: Claim validator fees
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
    Given a seed validator node VN connected to base node BASE and wallet daemon WALLET_D using claim fee key K1
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 5000 T
    When validator node VN sends a registration transaction to base wallet WALLET
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17
    And indexer IDX has scanned to height 17
    Then the validator node VN is listed as registered

    When indexer IDX connects to all other validators

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins using key K1
    When I create an account ACC3 via the wallet daemon WALLET_D with 10000 free coins

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D

    When I check the balance of ACC2 on wallet daemon WALLET_D the amount is at least 10200

  @serial @fixed
  Scenario: Prevent double claim of validator fees
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
    Given a seed validator node VN connected to base node BASE and wallet daemon WALLET_D using claim fee key K1
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 10000 T
    When validator node VN sends a registration transaction to base wallet WALLET
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17
    And indexer IDX has scanned to height 17
    Then the validator node VN is listed as registered

    When indexer IDX connects to all other validators

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins using key K1
    When I create an account ACC3 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC4 via the wallet daemon WALLET_D with 10000 free coins

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27

    # Can't claim fees with difference account
    When I claim fees for validator VN and epoch 1 into account ACC1 using the wallet daemon WALLET_D, it fails

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D
    When I check the balance of ACC2 on wallet daemon WALLET_D the amount is at least 10300

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D, it fails
