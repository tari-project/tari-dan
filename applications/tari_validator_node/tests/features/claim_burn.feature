# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Claim Burn

  @serial @current
  Scenario: Claim base layer burn token
      # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET
    When miner MINER mines 4 new blocks
    When validator node VN sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VN is listed as registered
    When I burn 10T on wallet WALLET
    When miner MINER mines 10 new blocks

    When validator node VN registers the template "fees"
    When miner MINER mines 5 new blocks

        # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet
    When I create a component SECOND_LAYER_TARI of template "fees" on VN using "new"
    When I print the cucumber world
