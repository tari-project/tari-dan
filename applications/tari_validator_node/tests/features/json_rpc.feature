Feature: JSON-RPC methods

  @serial
  Scenario: The validator node returns its identity
    Given a base node BASE
    Given a wallet WALLET connected to base node BASE
    Given a miner MINER connected to base node BASE and wallet WALLET
    When miner MINER mines 8 new blocks
    Given a validator node VAL connected to base node BASE and wallet WALLET
    Then the validator node VAL returns a valid identity