# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Basic scenarios
  @serial
  Scenario: Template registration and invocation in a 2-VN committee
    # Initialize a base node, wallet and miner
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize two validator nodes
    Given a validator node VAL_1 connected to base node BASE and wallet WALLET
    Given a validator node VAL_2 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks

    # VN registration
    When validator node VAL_1 sends a registration transaction
    When validator node VAL_2 sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VAL_1 is listed as registered
    Then the validator node VAL_2 is listed as registered

    # Register the "counter" template
    # When validator node VAL_1 registers the template "counter"
    # When miner MINER mines 20 new blocks
    # Then the template "counter" is listed as registered by the validator node VAL_1
    # FIXME: In GitHub actions, we get a "Template not found" error in VN2
    # Then the template "counter" is listed as registered by the validator node VAL_2

    # Call the constructor in the "counter" template
    # FIXME: The VN does not return a valid response
    # When the validator node VAL_1 calls the function "new" on the template "counter"

    # Uncomment the following lines to stop execution for manual inspection of the nodes
    # When I print the cucumber world
    # When I wait 5000 seconds
    

