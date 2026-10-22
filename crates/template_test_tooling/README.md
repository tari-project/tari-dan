# tari_template_test_tooling

[![Crates.io](https://img.shields.io/crates/v/tari_template_test_tooling.svg)](https://crates.io/crates/tari_template_test_tooling)
[![Documentation](https://docs.rs/tari_template_test_tooling/badge.svg)](https://docs.rs/tari_template_test_tooling)

Test harness for Tari smart-contract templates. Compiles template crates,
initialises an in-process engine with built-in faucet state, and exposes a
simple API for building and executing transactions — no running network
required.

## Example

```rust
use tari_ootle_transaction::{Transaction, args};
use tari_template_lib_types::constants::TARI_TOKEN;
use tari_template_test_tooling::TemplateTest;

#[test]
fn transfer_between_accounts() {
    // Compiles and loads the template from the current crate (i.e. template located at ./src, test located at ./tests)
    let mut test = TemplateTest::my_crate();

    // Create two funded accounts (each receives TARI from the built-in faucet)
    let (sender, sender_proof, sender_secret) = test.create_funded_account();
    let (receiver, _, _) = test.create_empty_account();

    test.execute_expect_success(
        test.transaction()
            .call_method(sender, "withdraw", args![TARI_TOKEN, 100])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(receiver, "deposit", args![Workspace("bucket")])
            .finish()
            .seal(&sender_secret),
        vec![sender_proof],
    );
}
```

To test a template crate at any path, pass the crate path instead:

```rust
// Compiles and loads the template from the given crate directory
let mut test = TemplateTest::new(env!("CARGO_MANIFEST_DIR"), vec!["MyTemplate"]);
```

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
