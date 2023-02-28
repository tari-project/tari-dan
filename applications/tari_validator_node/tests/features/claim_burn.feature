# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Claim Burn

  @serial
  Scenario: Claim base layer burn token
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then the validator node VN is listed as registered

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet
    # When I create a component SECOND_LAYER_TARI of template "fees" on VN using "new"
    When I create an account ACC_1 on VN

    When I burn 10T on wallet WALLET into commitment COMMITMENT with proof PROOF for ACC_1 and range proof RANGEPROOF

    # unfortunately have to wait for this to get into the mempool....
    Then there is 1 transaction in the mempool of BASE within 10 seconds
    When miner MINER mines 6 new blocks
    Then VN is on epoch 2 within 10 seconds

    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS

    When validator node VN registers the template "fees"
    When miner MINER mines 5 new blocks

    When I save the state database of VN
    When I claim burn COMMITMENT with PROOF and RANGEPROOF and spend it into account ACC_1 on VN
    # Then account ACC_1 has one confidential bucket in it

  @serial
  Scenario: Double claim burn funds is invalid
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When validator node VN sends a registration transaction
    When miner MINER mines 16 new blocks
    Then the validator node VN is listed as registered

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet
    # When I create a component SECOND_LAYER_TARI of template "fees" on VN using "new"
    When I create an account ACC_1 on VN

    When I burn 10T on wallet WALLET into commitment COMMITMENT with proof PROOF for ACC_1 and range proof RANGEPROOF

    # unfortunately have to wait for this to get into the mempool....
    Then there is 1 transaction in the mempool of BASE within 10 seconds
    # TODO: reduce the number of blocks mined by checking the VN's scanned height instead of epochs
    When miner MINER mines 13 new blocks
    Then VN is on epoch 3 within 10 seconds

    When I convert commitment COMMITMENT into COMM_ADDRESS address
    Then validator node VN has state at COMM_ADDRESS

    When validator node VN registers the template "fees"
    When miner MINER mines 5 new blocks

    When I save the state database of VN
    When I claim burn COMMITMENT with PROOF and RANGEPROOF and spend it into account ACC_1 on VN
    When I claim burn COMMITMENT with PROOF and RANGEPROOF and spend it into account ACC_1 on VN a second time, it fails
