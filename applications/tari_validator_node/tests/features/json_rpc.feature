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
    #When miner MINER mines 8 new blocks
    #Then the validator node VAL is listed as registered
