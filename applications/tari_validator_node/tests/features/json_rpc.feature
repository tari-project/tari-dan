# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: JSON-RPC methods

  #@serial
  #Scenario: The validator node returns a valid identity
    #Given a base node BASE
    #Given a wallet WALLET connected to base node BASE
    #Given a miner MINER connected to base node BASE and wallet WALLET
    #Given a validator node VAL connected to base node BASE and wallet WALLET
    #Then the validator node VAL returns a valid identity

  #@serial
  #Scenario: The validator node is able to register
    #Given a base node BASE
    #Given a wallet WALLET connected to base node BASE
    #Given a miner MINER connected to base node BASE and wallet WALLET
    #Given a validator node VAL connected to base node BASE and wallet WALLET

    ## The wallet must have some funds before the VN sends the registration transaction
    #When miner MINER mines 12 new blocks

    #When validator node VAL sends a registration transaction

    ## The registration transactions must be mined to be included in a block
    #When miner MINER mines 16 new blocks
    ## FIXME: the base node does not list the VN as registered, so the test fails
    #Then the validator node VAL is listed as registered

  @serial
  Scenario: The validator node registers a template
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    Given a validator node VAL_1 connected to base node BASE and wallet WALLET
    Given a validator node VAL_2 connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks

    When validator node VAL_1 sends a registration transaction
    When validator node VAL_2 sends a registration transaction
    When miner MINER mines 20 new blocks
    # Then the validator node VAL is listed as registered

    When validator node VAL_1 registers the template "counter"
    When miner MINER mines 20 new blocks

    Then the template "counter" is listed as registered by the validator node VAL_1
    Then the template "counter" is listed as registered by the validator node VAL_2
    Then the validator node VAL_1 calls the function "new" on the template "counter" and gets a valid response

