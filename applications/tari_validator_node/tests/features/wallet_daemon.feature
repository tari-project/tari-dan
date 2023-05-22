# Copyright 2022 The Tari Project
# SPDX-License-Identifier: BSD-3-Clause

Feature: Wallet Daemon

    @serial
    Scenario: Create account and transfer faucets via wallet daemon
        # Initialize a base node, wallet, miner and VN
        Given a base node BASE
        Given a wallet WALLET connected to base node BASE
        Given a miner MINER connected to base node BASE and wallet WALLET

        # Initialize a VN
        Given a validator node VAL_1 connected to base node BASE and wallet WALLET

        # The wallet must have some funds before the VN sends transactions
        When miner MINER mines 6 new blocks
        When wallet WALLET has at least 20000000 uT

        # VN registration
        When validator node VAL_1 sends a registration transaction
        When miner MINER mines 16 new blocks
        Then the validator node VAL_1 is listed as registered

        # Initialize an indexer
        Given an indexer IDX connected to base node BASE

        # Initialize the wallet daemon
        Given a wallet daemon WALLET_D connected to indexer IDX

        # Register the "faucet" template
        When validator node VAL_1 registers the template "faucet"

        # Mine some blocks until the UTXOs are scanned
        When miner MINER mines 5 new blocks
        Then the template "faucet" is listed as registered by the validator node VAL_1

        # Create two accounts to test sending the tokens
        When I create an account ACC_1 via the wallet daemon WALLET_D with 1000 free coins
        When I create an account ACC_2 via the wallet daemon WALLET_D
        When I check the balance of ACC_2 on wallet daemon WALLET_D the amount is exactly 0

        # Create a new Faucet component
        When I call function "mint" on template "faucet" using account ACC_1 to pay fees via wallet daemon WALLET_D with args "10000" and 3 outputs named "FAUCET"

        # Submit a transaction manifest
        When I print the cucumber world
        When I submit a transaction manifest via wallet daemon WALLET_D with inputs "FAUCET, ACC_1" and 3 outputs named "TX1"
        ```
        let faucet = global!["FAUCET/components/TestFaucet"];
        let mut acc1 = global!["ACC_1/components/Account"];

        // get tokens from the faucet
        let faucet_bucket = faucet.take_free_coins();
        acc1.deposit(faucet_bucket);
        ```
        When I print the cucumber world

        # Submit a transaction manifest
        When I submit a transaction manifest via wallet daemon WALLET_D signed by the key of ACC_1 with inputs "FAUCET, TX1, ACC_2" and 1 output named "TX2"
        ```
        let mut acc1 = global!["TX1/components/Account"];
        let mut acc2 = global!["ACC_2/components/Account"];
        let faucet_resource = global!["FAUCET/resources/0"];

        // Withdraw 50 of the tokens and send them to acc2
        let tokens = acc1.withdraw(faucet_resource, Amount(50));
        acc2.deposit(tokens);
        acc2.balance(faucet_resource);
        acc1.balance(faucet_resource);
        ```
        # Check balances
        # Notice that `take_free_coins` extracts precisely 1000 faucet tokens
        When I check the balance of ACC_1 on wallet daemon WALLET_D the amount is at least 1000
        When I wait for ACC_2 on wallet daemon WALLET_D to have balance eq 50

    @serial
    Scenario: Claim and transfer confidential assets via wallet daemon
        # Initialize a base node, wallet, miner and VN
        Given a base node BASE
        Given a wallet WALLET connected to base node BASE
        Given a miner MINER connected to base node BASE and wallet WALLET

        # Initialize a VN
        Given a validator node VN connected to base node BASE and wallet WALLET
        When miner MINER mines 4 new blocks
        When validator node VN sends a registration transaction
        When miner MINER mines 16 new blocks
        Then the validator node VN is listed as registered

        # Initialize an indexer
        Given an indexer IDX connected to base node BASE

        # Initialize the wallet daemon
        Given a wallet daemon WALLET_D connected to indexer IDX

        # When I create a component SECOND_LAYER_TARI of template "fees" on VN using "new"
        When I create an account ACCOUNT_1 via the wallet daemon WALLET_D with 1000 free coins
        When I create an account ACCOUNT_2 via the wallet daemon WALLET_D

        When I burn 1000T on wallet WALLET with wallet daemon WALLET_D into commitment COMMITMENT with proof PROOF for ACCOUNT_1, range proof RANGEPROOF and claim public key CLAIM_PUBKEY

        # unfortunately have to wait for this to get into the mempool....
        Then there is 1 transaction in the mempool of BASE within 10 seconds
        When miner MINER mines 13 new blocks
        Then VN has scanned to height 30 within 10 seconds

        When I convert commitment COMMITMENT into COMM_ADDRESS address
        Then validator node VN has state at COMM_ADDRESS

        When I claim burn COMMITMENT with PROOF, RANGEPROOF and CLAIM_PUBKEY and spend it into account ACCOUNT_1 via the wallet daemon WALLET_D
        When I print the cucumber world
        When I check the confidential balance of ACCOUNT_1 on wallet daemon WALLET_D the amount is at least 1000
        # When account ACCOUNT_1 reveals 100 burned tokens via wallet daemon WALLET_D
        Then I make a confidential transfer with amount 5 from ACCOUNT_1 to ACCOUNT_2 creating output OUTPUT_TX1 via the wallet_daemon WALLET_D
