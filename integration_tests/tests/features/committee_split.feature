# Copyright 2026 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@serial
@committee_split
Feature: Committee split

  Scenario: Indexer keeps serving when an epoch change splits the committee
    Given a network with spec
    """
    consensus_constants:
      # One committee per four validators, so an eighth registration splits the network rather than
      # needing the fourteen the devnet default would ask for. Validators are placed in the split
      # shard space by their base-layer shard key, which no one here chooses, so the count is kept
      # high enough that the draw is unlikely to leave either half of the shard space empty.
      committee_size_per_shard_group: 4
    validators:
      - name: VN1
      - name: VN2
      - name: VN3
      - name: VN4
      - name: VN5
      - name: VN6
      - name: VN7
    walletds:
      - name: WALLET_D
    """

    Then the network has 1 shard group according to indexer INDEXER

    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 XTR
    Then I wait for the indexer INDEXER to sync with the network

    # An eighth registration takes the validator count past the committee size, so the next epoch
    # splits the shard space in two and every validator stops storing half of what it stored before.
    Given a validator node VN8 connected to base node BASE_NODE
    Given validator VN8 nodes connect to all other validators
    When indexer INDEXER connects to all other validators
    When validator node VN8 sends a registration transaction to base wallet MINOTARI_WALLET
    Then miner MINER mines to the next epoch
    Then the validator node VN8 is listed as registered
    When all validator nodes have started epoch 4
    Then the network has 2 shard groups according to indexer INDEXER

    # Reads have to survive the split. Each validator now answers for half the shard space, so an
    # indexer still asking one of them for the whole of it is refused, and must re-plan onto the new
    # shard groups rather than treating the refusal as an empty round.
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 XTR
    Then I wait for the indexer INDEXER to sync with the network
