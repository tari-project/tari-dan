# Copyright 2024 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@concurrency
@doit
Feature: Concurrency

  @ignore
  Scenario: Concurrent calls to the Counter template

    ##### Setup
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a validator node
    Given a validator node VN connected to base node BASE and wallet daemon WALLET_D

    # Fund wallet to send VN registration tx
    When miner MINER mines 10 new blocks
    When wallet WALLET has at least 2000 T
    When validator node VN sends a registration transaction to base wallet WALLET
    When miner MINER mines 16 new blocks
    Then the validator node VN is listed as registered

    # Initialize indexer and connect wallet daemon
    Given an indexer IDX connected to base node BASE
    Given a wallet daemon WALLET_D connected to indexer IDX

    # Register the "counter" template
    When base wallet WALLET registers the template "counter"
    When miner MINER mines 20 new blocks
    Then VN has scanned to height 43

    # Create the sender account
    When I create an account ACC via the wallet daemon WALLET_D with 10000 free coins

    ##### Scenario
    # The initial value of the counter must be 0
    When I call function "new" on template "counter" using account ACC to pay fees via wallet daemon WALLET_D named "COUNTER"
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/Counter the method call "value" the result is "0" 

    # Send multiple concurrent transactions to increase the counter
    # Currently there is a lock bug where the subsequent transactions executed are being rejected, should be tested later after engine changes:
    # Reject(FailedToLockInputs("Failed to Write lock substate component_459d...4443c:1 due to conflict with existing Write lock"))
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/Counter the method call "increase" concurrently 2 times

    # Check that the counter has been increased
    # Note: this is currently not working together with the previous test case when times > 1, only the first transaction is being executed properly
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/Counter the method call "value" the result is "2"
