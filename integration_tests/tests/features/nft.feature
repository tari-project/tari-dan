# Copyright 2024 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

@concurrent
@nft
Feature: NFTs

  Scenario: Mint, mutate and burn non fungible tokens

    ###### Setup
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

    # Register the "basic_nft" template
    When base wallet WALLET registers the template "basic_nft"
    When miner MINER mines 20 new blocks
    Then VN has scanned to height 43
    Then the template "basic_nft" is listed as registered by the validator node VN

    ###### Scenario
    # Create two accounts to deposit the minted NFTs
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins
    When I create an account ACC2 via the wallet daemon WALLET_D with 10000 free coins

    # Mint a basic NFT
    When I mint a new non fungible token NFT_X on ACC1 using wallet daemon WALLET_D

    # Check that a new NFT_X has been minted for ACC1
    # TODO: investigate flaky test
    #When I list all non fungible tokens on ACC1 using wallet daemon WALLET_D the amount is 1

    # Create instance of the basic NFT template
    When I call function "new" on template "basic_nft" using account ACC1 to pay fees via wallet daemon WALLET_D named "NFT"

    # Submit a transaction with NFT operations
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "NFT, ACC1, ACC2" named "TX1"
  ```
  let sparkle_nft = global!["NFT/components/SparkleNft"];
  let sparkle_res = global!["NFT/resources/0"];
  let mut acc1 = global!["ACC1/components/Account"];
  let mut acc2 = global!["ACC2/components/Account"];

  // mint a new nft with random id
  let nft_bucket = sparkle_nft.mint("NFT1", "http://example.com");
  acc1.deposit(nft_bucket);

  // mint a new nft with specific id
  let nft_bucket = sparkle_nft.mint_specific(NonFungibleId("SpecialNft"), "NFT2", "http://example.com");
  acc1.deposit(nft_bucket);

  // transfer nft between accounts
  let acc_bucket = acc1.withdraw_non_fungible(sparkle_res, NonFungibleId("SpecialNft"));
  acc2.deposit(acc_bucket);

  // mutate a nft
  sparkle_nft.inc_brightness(NonFungibleId("SpecialNft"), 10u32);

  // burn a nft
  let nft_bucket = sparkle_nft.mint_specific(NonFungibleId("Burn!"), "NFT3", "http://example.com");
  acc1.deposit(nft_bucket);
  let acc_bucket = acc1.withdraw_non_fungible(sparkle_res, NonFungibleId("Burn!"));
  sparkle_nft.burn(acc_bucket);
  ```

  Scenario: Create resource and mint in one transaction

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


    # Register the "basic_nft" template
    When base wallet WALLET registers the template "basic_nft"
    When miner MINER mines 20 new blocks
    Then VN has scanned to height 43
    Then the template "basic_nft" is listed as registered by the validator node VN


    ###### Scenario
    # Create an account to deposit the minted NFT
    When I create an account ACC1 via the wallet daemon WALLET_D with 10000 free coins

    # Create a new BasicNft component and mint in the same transaction.
    # Note the updated NFT address format or parsing the manifest will fail.
    When I call function "new_with_initial_nft" on template "basic_nft" using account ACC1 to pay fees via wallet daemon WALLET_D with args "nft_str_1000" named "NFT"

    # Check that the initial NFT was actually minted by trying to deposit it into an account
    When I submit a transaction manifest via wallet daemon WALLET_D with inputs "NFT, ACC1" named "TX1"
  ```
  let sparkle_nft = global!["NFT/components/SparkleNft"];
  let mut acc1 = global!["ACC1/components/Account"];

  // get the initailly NFT from the component's vault
  let nft_bucket = sparkle_nft.take_initial_nft();
  acc1.deposit(nft_bucket);
  ```
