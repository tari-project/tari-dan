# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Indexer node

  @serial
  Scenario: Indexer is able to connect to validator nodes
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 2000000000 uT

    # VN registration
    When validator node VN sends a registration transaction

    # Register some templates
    When validator node VN registers the template "counter"
    When validator node VN registers the template "basic_nft"
    When miner MINER mines 10 new blocks
    Then the validator node VN is listed as registered
    Then the template "counter" is listed as registered by the validator node VN
    Then the template "basic_nft" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new Counter component and increase it to have a version 1
    When I create a component COUNTER_1 of template "counter" on VN using "new"
    When I invoke on VN on component COUNTER_1/components/Counter the method call "increase" with 1 outputs named "TX1"

    # Create an account to deposit minted nfts
    When I create an account ACC1 on VN

    # Create a new SparkleNft component and mint an NFT
    When I call function "new" on template "basic_nft" on VN with 3 outputs named "NFT"
    When I submit a transaction manifest on VN with inputs "NFT, ACC1" and 4 outputs named "TX2"
        ```
            // $mint NFT/resources/0 1
            let sparkle_nft = global!["NFT/components/SparkleNft"];
            let mut acc1 = global!["ACC1/components/Account"];

            // mint a new nft with random id
            let nft_bucket = sparkle_nft.mint();
            acc1.deposit(nft_bucket);
        ```

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Get substate of a component (the counter has been increased, so the version is 1)
    Then the indexer IDX returns version 1 for substate COUNTER_1/components/Counter

    # Get substate of a resource (the nft resource has been mutated by the minting, so the version is 1)
    Then the indexer IDX returns version 1 for substate NFT/resources/0

    # Get substate of a nft (newly minted and not mutated, so version is 0)
    Then the indexer IDX returns version 0 for substate TX2/nfts/0
    
    # When I print the cucumber world
    # When I wait 5000 seconds
