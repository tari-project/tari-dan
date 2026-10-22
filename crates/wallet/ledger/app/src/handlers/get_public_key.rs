//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_ledger_common::{
    OotleStatusWord,
    arg_types::{GetPublicKeyRequest, GetPublicKeyResponse, KeyType},
};
use zeroize::Zeroizing;

use crate::{
    crypto::public_key_from_scalar,
    hashing::derive_stealth_secret,
    key_derive::derive_from_bip32_key,
    state::State,
    status::AppStatus,
};

pub fn get_public_key(_state_mut: &mut State, request: GetPublicKeyRequest) -> Result<GetPublicKeyResponse, AppStatus> {
    let k = Zeroizing::new(derive_from_bip32_key(request.account, request.index, request.key_type)?);
    let secret = match request.stealth {
        // Stealth one-time keys are only defined over the account branch: a stealth output pays
        // `c·G + K_account`, so tweaking any other branch derives a key that owns nothing.
        Some(_) if !matches!(request.key_type, KeyType::Account) => {
            return Err(AppStatus::OotleStatusWord(OotleStatusWord::BadRequest));
        },
        Some(tweak) => {
            let v = Zeroizing::new(derive_from_bip32_key(
                request.account,
                request.index,
                KeyType::ViewOnlyKey,
            )?);
            Zeroizing::new(derive_stealth_secret(tweak.network, &k, &v, &tweak.public_nonce)?)
        },
        None => k,
    };
    let pk = public_key_from_scalar(&secret);

    Ok(GetPublicKeyResponse {
        public_key: pk.compress().0,
    })
}
