# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: NFTs

  @serial
  Scenario: Mint, mutate and burn non fungible tokens
    # Initialize a base node, wallet, miner and VN
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET

    # Initialize a VN
    Given a validator node VN connected to base node BASE and wallet WALLET

    # The wallet must have some funds before the VN sends transactions
    When miner MINER mines 12 new blocks
    When wallet WALLET has at least 1000000000 uT

    # VN registration
    When validator node VN sends a registration transaction
    When miner MINER mines 20 new blocks
    Then the validator node VN is listed as registered

    # Register the "basic_nft" template
    When validator node VN registers the template "basic_nft"
    When miner MINER mines 20 new blocks
    Then the template "basic_nft" is listed as registered by the validator node VN

    # A file-base CLI account must be created to sign future calls
    When I create a DAN wallet

    # Create a new BasicNft component
    When I call function "new" on template "basic_nft" on VN with 3 outputs named "NFT"

    # Create an account to deposit the minted nfts
    When I create an account ACC1 on VN
    When I create an account ACC2 on VN

    # Submit a transaction with NFT operations
    When I submit a transaction manifest on VN with inputs "NFT, ACC1, ACC2" and 3 outputs named "TX1"
        ```
            // $mint NFT/resources/0 1
            // $mint_specific NFT/resources/0 str:SpecialNft
            // $mint_specific NFT/resources/0 str:Burn!
            let sparkle_nft = global!["NFT/components/SparkleNft"];
            let sparkle_res = global!["NFT/resources/0"];
            let mut acc1 = global!["ACC1/components/Account"];
            let mut acc2 = global!["ACC2/components/Account"];

            // mint a new nft with random id
            let nft_bucket = sparkle_nft.mint();
            acc1.deposit(nft_bucket);

            // mint a new nft with specific id
            let nft_bucket = sparkle_nft.mint_specific(NonFungibleId("SpecialNft"));
            acc1.deposit(nft_bucket);

            // transfer nft between accounts
            let acc_bucket = acc1.withdraw_non_fungible(sparkle_res, NonFungibleId("SpecialNft"));
            acc2.deposit(acc_bucket);

            // mutate a nft
            sparkle_nft.inc_brightness(NonFungibleId("SpecialNft"), 10u32);

            // burn a nft
            let nft_bucket = sparkle_nft.mint_specific(NonFungibleId("Burn!"));
            acc1.deposit(nft_bucket);
            let acc_bucket = acc1.withdraw_non_fungible(sparkle_res, NonFungibleId("Burn!"));
            sparkle_nft.burn(acc_bucket);
        ```
    When I print the cucumber world

