//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// TODO: This does not work currently

fn fee_main() {
    let owner = arg!["owner"];
    let free_coins = create_free_coins!(Amount(1000), None);
    let account = allocate_component_address!();
    Account::create_advanced(owner, free_coins, account);
    account.pay_fee(Amount(1000));
}

fn main() {}
