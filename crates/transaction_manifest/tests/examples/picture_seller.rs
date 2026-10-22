//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

// These mappings can be provided to the parser context or defined inline in the manifest
use template_687b0d5b3bee2e987a72c0f8b0b9286968803eba9040ed67e3a85b8465ad294a as TestFaucet;
use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as PictureSeller;

fn main() {
    // initialize the component
    let picture_seller = PictureSeller::new(1_000u64);

    let faucet = var!["test_faucet"];
    // TODO: Implement sugar for the account component
    // e.g.  let account = default_account!();
    let mut account = var!["account"];

    // initialize a user account with some faucet funds
    let funds = faucet.take_free_coins(1_000);
    account.deposit(funds);
    account.set_public_key(
        public_key!("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        address!("component_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        cbor!({"some": {"data": [1, 2, 3]}}),
    );

    // buy a picture
    let bucket = account.withdraw(TARI, 1_000);
    let picture = picture_seller.buy(bucket);

    // store our brand new picture in our account
    account.deposit(picture);
}
