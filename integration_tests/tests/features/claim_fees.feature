# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Claim Fees
  @serial
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
    Given a seed validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 5000 T
    When validator node VN sends a registration transaction allowing fee claims from wallet WALLET_D using key K1
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17 within 10 seconds
    And indexer IDX has scanned to height 17 within 10 seconds
    Then the validator node VN is listed as registered

    When indexer IDX connects to all other validators

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins using key K1
    When I create an account ACC3 via the wallet daemon WALLET_D with 10000 free coins

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27 within 10 seconds

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D

    When I check the balance of ACC2 on wallet daemon WALLET_D the amount is at least 10500

  @serial
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
    Given a seed validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 10000 T
    When validator node VN sends a registration transaction allowing fee claims from wallet WALLET_D using key K1
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17 within 10 seconds
    And indexer IDX has scanned to height 17 within 10 seconds
    Then the validator node VN is listed as registered

    When indexer IDX connects to all other validators

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins using key K1
    When I create an account ACC3 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC4 via the wallet daemon WALLET_D with 10000 free coins

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27 within 20 seconds

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D
    When I check the balance of ACC2 on wallet daemon WALLET_D the amount is at least 10500

    # Claim fees into ACC2
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET_D, it fails

  @serial
  Scenario: Prevent validator fees claim for unauthorized wallet
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize wallet daemons
    Given a wallet daemon WALLET1 connected to indexer IDX
    When I create a key named K1 for WALLET1
    Given a wallet daemon WALLET2 connected to indexer IDX
    When I create a key named K2 for WALLET2

    # Initialize a VN
    Given a seed validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 10000 T
    When validator node VN sends a registration transaction allowing fee claims from wallet WALLET1 using key K1
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17 within 10 seconds
    And indexer IDX has scanned to height 17 within 10 seconds
    Then the validator node VN is listed as registered

    When indexer IDX connects to all other validators

    # Run some transactions to generate fees
    When I create an account ACC1 via the wallet daemon WALLET1 with 10000 free coins using key K1
    When I create an account ACC2 via the wallet daemon WALLET2 with 10000 free coins using key K2
    # Run up some fees
    When I create an account UNUSED1 via the wallet daemon WALLET2
    When I create an account UNUSED2 via the wallet daemon WALLET2
    When I create an account UNUSED3 via the wallet daemon WALLET2
    When I create an account UNUSED4 via the wallet daemon WALLET2

    # Progress to the next epoch
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 27 within 10 seconds

    # Claim fees using unauthorized wallet
    When I claim fees for validator VN and epoch 1 into account ACC2 using the wallet daemon WALLET2, it fails

    # Claim fees using authorized wallet
    When I claim fees for validator VN and epoch 1 into account ACC1 using the wallet daemon WALLET1
    When I check the balance of ACC1 on wallet daemon WALLET1 the amount is at least 10500

