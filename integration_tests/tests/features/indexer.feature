# Copyright 2024 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@concurrent
@indexer
Feature: Indexer node

  Scenario: Wallet daemon is able to connect to indexer
    Given a network with registered validator VN and wallet daemon WALLET_D

    # Create the sender account
    When I create an account ACC via the wallet daemon WALLET_D with 10 XTR

    # Publish some templates
    When wallet daemon WALLET_D publishes the template "counter" using account ACC
    When wallet daemon WALLET_D publishes the template "basic_nft" using account ACC

    ##### Scenario
    # Create a new counter component and increase it to have version 1
    When I call function "new" on template "counter" using account ACC to pay fees via wallet daemon WALLET_D named "COUNTER"
    When I invoke on wallet daemon WALLET_D on account ACC on component COUNTER/components/counter the method call "increase"

    # Create a new SparkleNft component and mint an NFT
    When I call function "new" on template "basic_nft" using account ACC to pay fees via wallet daemon WALLET_D named "NFT"
    When I submit a transaction manifest via wallet daemon WALLET_D signed by the key of ACC with inputs "NFT, ACC" named "TX1"
  """
  let sparkle_nft = global!["NFT/components/basic_nft"];
  let mut acc = global!["ACC/accounts/ACC"];

  // mint a couple of nfts with random ids
  let nft_bucket_1 = sparkle_nft.mint("Astronaut (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149622024.jpg");
  acc.deposit(nft_bucket_1);
  let nft_bucket_2 = sparkle_nft.mint("Baby (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149629576.jpg");
  acc.deposit(nft_bucket_2);
  let nft_bucket_3 = sparkle_nft.mint("Cool (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149622021.jpg");
  acc.deposit(nft_bucket_3);
  let nft_bucket_4 = sparkle_nft.mint("Metaverse (Image by Freepik.com)", "https://img.freepik.com/premium-vector/hand-drawn-monkey-ape-vr-box-virtual-nft-style_361671-246.jpg");
  acc.deposit(nft_bucket_4);
  let nft_bucket_5 = sparkle_nft.mint("Suit (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149629594.jpg");
  acc.deposit(nft_bucket_5);
  let nft_bucket_6 = sparkle_nft.mint("Cook  (Image by Freepik.com)", "https://img.freepik.com/free-vector/hand-drawn-nft-style-ape-illustration_23-2149629582.jpg");
  acc.deposit(nft_bucket_6);
  """

    # Get substate of a component (the counter has been increased, so the version is 1)
    Then the indexer INDEXER returns version 1 for substate COUNTER/components/counter

    # Get substate of a resource (the nft resource has been mutated by the minting, so the version is 1)
    Then the indexer INDEXER returns version 1 for substate NFT/resources/SPKL

    # Get substate of an nft (newly minted and not mutated, so version is 0)
    Then the indexer INDEXER returns version 0 for substate TX1/nfts/0

    Then I wait for the indexer INDEXER to sync with the network

    # Scan the network for the event emitted on ACC creation
    When indexer INDEXER scans the network events for account ACC with topics std.component.created,std.component.updated

  Scenario: Indexer serves events over network substate indexing
    Given a network with registered validator VN and wallet daemon WALLET_D

    When I create an account ACC_1 via the wallet daemon WALLET_D with 1 XTR
    When I create an account ACC_2 via the wallet daemon WALLET_D with 1 XTR

    Then I wait for the indexer INDEXER to sync with the network
    ##### Scenario
    # Scan the network for the event emitted on ACC_1 creation
    When indexer INDEXER scans the network events for account ACC_1 with topics std.component.created

    # Scan the network for the event emitted on ACC_2 creation
    When indexer INDEXER scans the network events for account ACC_2 with topics std.component.created

  Scenario: Indexer filtering and pagination of events
    Given a network with registered validator VN and wallet daemon WALLET_D

    # publish template
    When I create an account ACC via the wallet daemon WALLET_D with 10000 XTR
    When wallet daemon WALLET_D publishes the template "faucet" using account ACC

    When I create an account ACC_1 via the wallet daemon WALLET_D with 10000 XTR
    When I create an account ACC_2 via the wallet daemon WALLET_D with 10000 XTR

    ##### Scenario
    # Create a new faucet component
    When I call function "mint" on template "faucet" using account ACC_1 to pay fees via wallet daemon WALLET_D with args "10000" named "FAUCET"

    # Generate some events by doing vault operations with the faucet and the acounts
    When I submit a transaction manifest via wallet daemon WALLET_D signed by the key of ACC_1 with inputs "FAUCET, ACC_1, ACC_2" named "TX1"
  """
  let faucet = global!["FAUCET/components/faucet"];
  let faucet_resource = global!["FAUCET/resources/FAUCET"];
  let mut acc1 = global!["ACC_1/accounts/ACC_1"];
  let mut acc2 = global!["ACC_2/accounts/ACC_2"];

  // get tokens from the faucet to ACC_1
  let faucet_bucket = faucet.take_free_coins();
  acc1.deposit(faucet_bucket);

  // transfer some tokens from ACC_1 to ACC_2
  let bucket1 = acc1.withdraw(faucet_resource, amount!(50));
  acc2.deposit(bucket1);

  // transfer some tokens back from ACC_2 to ACC_1
  let bucket2 = acc2.withdraw(faucet_resource, amount!(20));
  acc1.deposit(bucket2);
  """

    # Wait for the scanning and indexing of events
    Then I wait for the indexer INDEXER to sync with the network

    # Query the events from the network
    When indexer INDEXER scans the network for events of resource FAUCET/resources/FAUCET

  @catalogue
  Scenario: Indexer template catalogue is populated after template publication
    Given a network with registered validator VN and wallet daemon WALLET_D

    # Create account and publish two templates
    When I create an account ACC via the wallet daemon WALLET_D with 10 XTR
    When wallet daemon WALLET_D publishes the template "counter" using account ACC
    When wallet daemon WALLET_D publishes the template "basic_nft" using account ACC

    # Wait for the indexer to sync template metadata from validators
    Then I wait for the indexer INDEXER to sync with the network

    # Both templates must appear in the catalogue
    Then the indexer INDEXER has at least 2 templates in the catalogue
    Then the indexer INDEXER catalogue contains template counter
    Then the indexer INDEXER catalogue contains template basic_nft

    # Name filter should narrow results
    Then the indexer INDEXER catalogue name filter counter returns 1 result
    Then the indexer INDEXER catalogue name filter nonexistent_xyz returns 0 results

    # Non-existent template address should return 404
    Then the indexer INDEXER catalogue entry for address "template_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" is not found

  @catalogue_pagination
  Scenario: Indexer template catalogue pagination works
    Given a network with registered validator VN and wallet daemon WALLET_D

    # Create account and publish three templates to exercise pagination
    When I create an account ACC via the wallet daemon WALLET_D with 10 XTR
    When wallet daemon WALLET_D publishes the template "counter" using account ACC
    When wallet daemon WALLET_D publishes the template "basic_nft" using account ACC
    When wallet daemon WALLET_D publishes the template "faucet" using account ACC

    # Wait for the indexer to sync template metadata from validators
    Then I wait for the indexer INDEXER to sync with the network
    Then the indexer INDEXER has at least 3 templates in the catalogue

    # Pagination: one entry per page
    Then the indexer INDEXER catalogue with limit 1 returns 1 entries
    # All entries in one page
    Then the indexer INDEXER catalogue with limit 100 returns at least 3 entries
