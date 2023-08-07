# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Claim Fees
  @serial
  Scenario: Claim validator fees
    # TODO: Update all cucumbers to use fees. For now we'll enable just for this test.
    Given fees are enabled

    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17 within 10 seconds
    Then the validator node VN is listed as registered

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC3 via the wallet daemon WALLET_D with 10000 free coins

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27 within 10 seconds

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D

    When I check the balance of ACC2 on wallet daemon WALLET_D the amount is at least 10500

  @serial
  Scenario: Prevent double claim of validator fees
    Given fees are enabled

    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17 within 10 seconds
    Then the validator node VN is listed as registered

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC3 via the wallet daemon WALLET_D with 10000 free coins

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27 within 10 seconds

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D
    When I check the balance of ACC2 on wallet daemon WALLET_D the amount is at least 10500

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D, it fails

