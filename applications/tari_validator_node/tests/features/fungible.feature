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
    Given a validator node VN connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks
    When wallet WALLET has at least 1000000000 uT

    # VN registration
    When validator node VN sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VN is listed as registered

    # Register the "faucet" template
    When validator node VN registers the template "faucet"
    When miner MINER mines 20 new blocks
    Then the template "faucet" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Faucet component
    When I create a component FAUCET of template "faucet" on VN using "mint" with inputs "10000"

    # Create two accounts to test sending the tokens
    When I create an account ACC_1 on VN
    When I create an account ACC_2 on VN
    

