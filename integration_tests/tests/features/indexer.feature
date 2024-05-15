# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@indexer
Feature: Indexer node

  @serial
  Scenario: Indexer is able to connect to validator nodes
    Given fees are disabled
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet daemon WALLET_D

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 2000000000 uT

    # VN registration
    When validator node VN sends a registration transaction to base wallet WALLET

    # Register some templates
    When base wallet WALLET registers the template "counter"
    When base wallet WALLET registers the template "basic_nft"
    When miner MINER mines 10 new blocks
    Then VN has scanned to height 13
    Then the validator node VN is listed as registered
    Then the template "counter" is listed as registered by the validator node VN
    Then the template "basic_nft" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I use an account key named K1

    # Create a new Counter component and increase it to have a version 1
    When I create a component COUNTER_1 of template "counter" on VN using "new"
    When I invoke on VN on component COUNTER_1/components/Counter the method call "increase" named "TX1"

    # Create an account to deposit minted nfts
    When I create an account ACC1 on VN

    # Create a new SparkleNft component and mint an NFT
    When I call function "new" on template "basic_nft" on VN named "NFT"
    When I submit a transaction manifest on VN with inputs "NFT, ACC1" named "TX2" signed with key ACC1
  ```
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
    Then indexer IDX has scanned to height 13

    # Track a component
    When the indexer IDX tracks the address ACC1/components/Account

    # Track a vault
    When the indexer IDX tracks the address TX2/vaults/0

    # Track the NFT resource so the indexer tries to get all individual NFTs
    When the indexer IDX tracks the address NFT/resources/0


    # Get substate of a component (the counter has been increased, so the version is 1)
    Then the indexer IDX returns version 1 for substate COUNTER_1/components/Counter

    # Get substate of a resource (the nft resource has been mutated by the minting, so the version is 1)
    Then the indexer IDX returns version 1 for substate NFT/resources/0

    # Get substate of an nft (newly minted and not mutated, so version is 0)
    Then the indexer IDX returns version 0 for substate TX2/nfts/0

    # List the nfts of a resource
    Then the indexer IDX returns 6 non fungibles for resource NFT/resources/0

    # Scan the network for the event emitted on ACC_1 creation
    When indexer IDX scans the network events for account ACC1 with topics component-created,deposit,deposit,deposit,deposit,deposit,deposit

  # When I print the cucumber world
  # When I wait 5000 seconds


  @serial
  Scenario: Indexer GraphQL requests work
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # Check GraphQL request
    Given IDX indexer GraphQL request works

  @serial
  Scenario: Indexer GraphQL requests events over network substate indexing
    Given fees are disabled
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet daemon WALLET_D

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 2000000000 uT

    # VN registration
    When validator node VN sends a registration transaction to base wallet WALLET

    When miner MINER mines 16 new blocks
    Then VN has scanned to height 19
    Then indexer IDX has scanned to height 19
    Then the validator node VN is listed as registered

    # A file-base CLI account must be created to sign future calls
    When I use an account key named K1

    # Creates a new account
    When I create an account ACC_1 on VN
    When I create an account ACC_2 on VN

    # Scan the network for the event emitted on ACC_1 creation
    When indexer IDX scans the network events for account ACC_1 with topics component-created

    # Scan the network for the event emitted on ACC_2 creation
    When indexer IDX scans the network events for account ACC_2 with topics component-created

  @serial
  Scenario: Indexer GraphQL filtering and pagination of events
    Given fees are disabled
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet daemon WALLET_D

    # Initialize an indexer
    Given an indexer IDX connected to base node BASE

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 6 new blocks
    When wallet WALLET has at least 2000000000 uT

    # VN registration
    When validator node VN sends a registration transaction to base wallet WALLET

    # Register the "faucet" template
    When base wallet WALLET registers the template "faucet"

    # Mine a few block for the VN and template registration
    When miner MINER mines 16 new blocks
    Then VN has scanned to height 19
    Then indexer IDX has scanned to height 19
    Then the validator node VN is listed as registered

    # Initialize the wallet daemon
    Given a wallet daemon WALLET_D connected to indexer IDX

    # A file-base CLI account must be created to sign future calls
    When I use an account key named K1

    # Creates a new account
    When I create an account ACC_1 on VN
    When I create an account ACC_2 on VN

    # Create a new faucet component
    When I call function "mint" on template "faucet" using account ACC_1 to pay fees via wallet daemon WALLET_D with args "10000" named "FAUCET"

    # Generate some events by doing vault operations with the faucet and the acounts
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACC_1, ACC_2" named "TX1"
  ```
  let faucet = global!["FAUCET/components/TestFaucet"];
  let faucet_resource = global!["FAUCET/resources/0"];
  let mut acc1 = global!["ACC_1/components/Account"];
  let mut acc2 = global!["ACC_2/components/Account"];

  // get tokens from the faucet to ACC_1
  let faucet_bucket = faucet.take_free_coins();
  acc1.deposit(faucet_bucket);

  // transfer some tokens from ACC_1 to ACC_2
  let bucket1 = acc1.withdraw(faucet_resource, Amount(50));
  acc2.deposit(bucket1);

  // transfer some tokens back from ACC_2 to ACC_1
  let bucket2 = acc2.withdraw(faucet_resource, Amount(20));
  acc1.deposit(bucket2);
  ```

    # Wait for the scanning and indexing of events
    When I wait 10 seconds

    # Query the events from the network
    When indexer IDX scans the network for events of resource FAUCET/resources/0

