Feature: JSON-RPC methods

  @serial
  Scenario: The validator node returns its identity
    Given a validator node "vn1"
    Then the validator node "vn1" returns a valid identity