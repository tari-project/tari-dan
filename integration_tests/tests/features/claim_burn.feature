# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Claim Burn

  @serial
  Scenario: Claim base layer burn funds with wallet daemon
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet_daemon WALLET_D
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 5000 T
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17
    Then the validator node VN is listed as registered

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX

    # When I create a component SECOND_LAYER_TARI of template "fees" on VN using "new"
    When I create an account ACC via the wallet daemon WALLET_D with 10000 free coins

    When I burn 10T on wallet WALLET with wallet daemon WALLET_D into commitment COMMITMENT with proof PROOF for ACC, range proof RANGEPROOF and claim public key CLAIM_PUBKEY

    # unfortunately have to wait for this to get into the mempool....
    Then there is 1 transaction in the mempool of BASE within 10 seconds
    When miner MINER mines 13 new blocks
    Then VN has scanned to height 30

    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS

    When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACC via the wallet daemon WALLET_D
  # Then account ACC has one confidential bucket in it

  @serial
  Scenario: Double Claim base layer burn funds with wallet daemon. should fail
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet daemon WALLET_D
    When miner MINER mines 4 new blocks
    When wallet WALLET has at least 10000 T
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 17
    Then the validator node VN is listed as registered

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX

    # When I create a component SECOND_LAYER_TARI of template "fees" on VN using "new"
    When I create an account ACC via the wallet daemon WALLET_D with 10000 free coins

    When I burn 10T on wallet WALLET with wallet daemon WALLET_D into commitment COMMITMENT with proof PROOF for ACC, range proof RANGEPROOF and claim public key CLAIM_PUBKEY

    # unfortunately have to wait for this to get into the mempool....
    Then there is 1 transaction in the mempool of BASE within 10 seconds
    When miner MINER mines 13 new blocks
    Then VN has scanned to height 30

    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS

    When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACC via the wallet daemon WALLET_D
    When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACC via the wallet daemon WALLET_D, it fails
# Then account ACC has one confidential bucket in it
