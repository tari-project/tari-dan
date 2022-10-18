Feature: JSON-RPC methods

  @serial
  Scenario: The validator node returns its identity
    Given a base node "bn1"
    Given a wallet "w1" connected to base node "bn1"
    Given a validator node "vn1" connected to base node "bn1" and wallet "w1"
    Then the validator node "vn1" returns a valid identity