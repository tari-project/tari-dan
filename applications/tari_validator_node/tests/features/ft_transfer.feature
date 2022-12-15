Feature: Fungible token transfer

  @serial
  Scenario: Fungible token transfer
    Given a base node BASE
    Given a wallet WALLET connected to a base node BASE
    Given a miner MINER connected to a base node BASE and wallet WALLET
    Given a validator node NODE connected to a base node BASE and wallet WALLET
    When miner MINER mines 12 new blocks
    When validator node NODE sends a registration transaction
    When miner MINER mines 20 new blocks
    Then validator node NODE is listed as registered by the validator node NODE
    When validator node NODE registers the template "faucet"
    When miner MINER mines 20 new blocks
    Then the template "faucet" is listed as registered by the validator node NODE
    When the validator node NODE calls the function "new" with 1 outputs on the template "account"
    When the validator node NODE calls the function "new" with 1 outputs on the template "account"
    When the validator node NODE calls the function "mint" with 1000000 amount input and 1 outputs on the template "faucet"
