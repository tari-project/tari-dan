# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Fungible tokens

  @serial
  Scenario: Mint fungible tokens
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

    # Register the "faucet" template
    When validator node VAL_1 registers the template "faucet"
    When miner MINER mines 20 new blocks
    Then the template "faucet" is listed as registered by the validator node VAL_1

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Faucet component
    When I create a component FAUCET of template "faucet" on VAL_1 using "mint" with inputs "10000"
    

