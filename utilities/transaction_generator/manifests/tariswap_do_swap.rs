//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

fn fee_main() {
    let in_account = arg!["in_account"];
    in_account.pay_fee(Amount(1000));
}

fn main() {
    let swap_component = arg!["swap_component"];
    let in_account = arg!["in_account"];
    let out_account = arg!["out_account"];
    let amt = arg!["amt"];

    let token_a = arg!["token_a"];
    let token_b = arg!["token_b"];

    let in_bucket = in_account.withdraw(token_a, amt);
    let out_bucket = swap_component.swap(in_bucket, token_b);
    out_account.deposit(out_bucket);
}
