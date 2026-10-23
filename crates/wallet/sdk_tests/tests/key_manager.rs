//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::{PublicKey as _, SecretKey as _},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::hex::Hex,
};
use tari_ootle_address::OotleAddress;
use tari_ootle_wallet_sdk::{
    WalletSdk,
    models::{EpochBirthday, KeyId, KeyType},
    storage::{WalletStoreWriter, WriteableWalletStore},
};

use crate::support::{PanicNetworkInterface, Test, TestSdkSpec};

mod support;

#[test]
fn it_generates_new_account_addresses() {
    let test = Test::new();
    let account_addr = test.sdk().key_manager_api().next_account_address().unwrap();
    let addr = account_addr.address.to_byte_type().to_bech32_string();

    let decoded = OotleAddress::decode_bech32(&addr).unwrap();

    assert_eq!(decoded.network(), test.sdk().network());
    assert_eq!(
        *decoded.account_public_key(),
        account_addr.address.account_key.to_byte_type()
    );
    assert_eq!(
        *decoded.view_only_key(),
        account_addr.address.view_only_key().to_byte_type()
    );

    let account_addr2 = test.sdk().key_manager_api().next_account_address().unwrap();
    assert_ne!(account_addr.address, account_addr2.address);
}

#[test]
fn imported_keys_are_decrypted_once_and_updated_on_import() {
    let test = Test::new();
    let key_manager = test.sdk().key_manager_api();
    let first = RistrettoSecretKey::random(&mut rand::rng());
    let first_public = RistrettoPublicKey::from_secret_key(&first);
    let KeyId::Imported { local_key_id: first_id } =
        key_manager.import_key("first", &first, KeyType::ViewOnly).unwrap()
    else {
        panic!("import_key returns an imported key id");
    };

    // Replace the stored ciphertext so that only a cached key can be returned
    test.store()
        .with_write_tx(|tx| {
            tx.key_manager_insert_imported_key("first", &first_public.to_hex(), b"not a ciphertext", KeyType::ViewOnly)
        })
        .unwrap();
    for _ in 0..2 {
        let key = test.sdk().key_manager_api().get_imported_key(first_id).unwrap();
        assert_eq!(key.key, first);
    }
    let uncached = WalletSdk::<TestSdkSpec>::initialize_with_local_key_store(
        test.store().clone(),
        PanicNetworkInterface,
        test.sdk().sdk_config().clone(),
        EpochBirthday::far_future(),
    )
    .unwrap();
    assert!(uncached.key_manager_api().get_imported_key(first_id).is_err());

    // Importing the key again stores and caches it under the same id
    let reimported = test
        .sdk()
        .key_manager_api()
        .import_key("first", &first, KeyType::ViewOnly)
        .unwrap();
    assert_eq!(reimported, KeyId::imported(first_id));
    assert_eq!(
        uncached.key_manager_api().get_imported_key(first_id).unwrap().key,
        first
    );

    let second = RistrettoSecretKey::random(&mut rand::rng());
    let second_id = test
        .sdk()
        .key_manager_api()
        .import_key("second", &second, KeyType::ViewOnly)
        .unwrap();
    assert_ne!(second_id, KeyId::imported(first_id));
    assert_eq!(test.sdk().key_manager_api().get_key(second_id).unwrap().secret, second);
}
