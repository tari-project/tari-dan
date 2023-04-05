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
    Then VN has scanned to height 13 within 10 seconds
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
    When I submit a transaction manifest on VN with inputs "NFT, ACC1" and 12 outputs named "TX2"
        ```
            // $mint NFT/resources/0 1
            // $mint NFT/resources/0 2
            // $mint NFT/resources/0 3
            // $mint NFT/resources/0 4
            // $mint NFT/resources/0 5
            // $mint NFT/resources/0 6
            // $nft_index NFT/resources/0 0
            // $nft_index NFT/resources/0 1
            // $nft_index NFT/resources/0 2
            // $nft_index NFT/resources/0 3
            // $nft_index NFT/resources/0 4
            // $nft_index NFT/resources/0 5
            let sparkle_nft = global!["NFT/components/SparkleNft"];
            let mut acc1 = global!["ACC1/components/Account"];

            // mint a couple of nfts with random ids
            let nft_bucket_1 = sparkle_nft.mint("Astronaut (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149622024.jpg");
            acc1.deposit(nft_bucket_1);
            let nft_bucket_2 = sparkle_nft.mint("Baby (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149629576.jpg");
            acc1.deposit(nft_bucket_2);
            let nft_bucket_3 = sparkle_nft.mint("Cool (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149622021.jpg");
            acc1.deposit(nft_bucket_3);
            let nft_bucket_4 = sparkle_nft.mint("Metaverse (Image by Freepik.com)", "https://img.freepik.com/premium-vector/hand-drawn-monkey-ape-vr-box-virtual-nft-style_361671-246.jpg");
            acc1.deposit(nft_bucket_4);
            let nft_bucket_5 = sparkle_nft.mint("Suit (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149629594.jpg");
            acc1.deposit(nft_bucket_5);
            let nft_bucket_6 = sparkle_nft.mint("Cook  (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149629582.jpg");
            acc1.deposit(nft_bucket_6);
        ```

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Track a component
    When the indexer IDX tracks the address ACC1/components/Account

    # Track a vault
    When the indexer IDX tracks the address TX2/vaults/0

    # Track the NFT resource so the indexer tries to get all individual NFTs
    When the indexer IDX tracks the address NFT/resources/0

    # The indexer needs a bit of time to track and store all NFTs of a resource
    When I wait 5 seconds

    # Get substate of a component (the counter has been increased, so the version is 1)
    Then the indexer IDX returns version 1 for substate COUNTER_1/components/Counter

    # Get substate of a resource (the nft resource has been mutated by the minting, so the version is 1)
    Then the indexer IDX returns version 1 for substate NFT/resources/0

    # Get substate of an nft (newly minted and not mutated, so version is 0)
    Then the indexer IDX returns version 0 for substate TX2/nfts/0

    # List the nfts of a resource
    Then the indexer IDX returns 6 non fungibles for resource NFT/resources/0
    
    # When I print the cucumber world
    # When I wait 5000 seconds
