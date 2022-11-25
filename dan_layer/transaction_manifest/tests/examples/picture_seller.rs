//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

// These mappings can be provided to the parser context or defined inline in the manifest
// TODO: Account builtin should not have to be explicitly defined
use template_687b0d5b3bee2e987a72c0f8b0b9286968803eba9040ed67e3a85b8465ad294a as TestFaucet;
use template_bc617551fc331bb5ebc12943c1456dbf8fbd0ae4e950b2ffc28a6b89ce040382 as Account;
use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as PictureSeller;

fn main() {
    // initialize the component
    // TODO: This creates a new component but does not use it as a component input for call method
    //       because this is not currently supported.
    let picture_seller = PictureSeller::new(1_000);

    let picture_seller = global!["picture_seller_addr"];
    let faucet = global!["test_faucet"];
    // TODO: Implement sugar for the account component
    // e.g.  let account = default_account!();
    let mut account = global!["account"];

    // initialize a user account with some faucet funds
    let funds = faucet.take_free_coins(1_000);
    account.deposit(funds);

    // TODO: XTR builtin
    let XTR = global!["xtr_resource"];

    // buy a picture
    let bucket = account.withdraw(XTR, 1_000);
    let picture = picture_seller.buy(bucket);

    // store our brand new picture in our account
    account.deposit(picture);
}
