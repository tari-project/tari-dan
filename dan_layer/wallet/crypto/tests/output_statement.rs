//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_crypto::create_withdraw_proof;
use tari_template_lib::models::Amount;

#[test]
fn it_create_a_valid_revealed_only_proof() {
    let proof = create_withdraw_proof(&[], Amount(123), None, Amount(123), None, Amount(0)).unwrap();

    assert!(proof.is_revealed_only());
}
