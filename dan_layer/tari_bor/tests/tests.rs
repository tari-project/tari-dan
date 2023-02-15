//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use borsh::{
    schema::{Declaration, Definition},
    BorshSchema,
};
use tari_bor::{decode, decode_from, encode_into, Decode, Encode};

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct Testme {
    a: u32,
    b: Vec<u32>,
}

impl borsh::schema::BorshSchema for Testme {
    fn add_definitions_recursively(definitions: &mut HashMap<Declaration, Definition>) {
        // Vec::<u32>::add_definitions_recursively(definitions);
    }

    fn declaration() -> Declaration {
        "Testme".to_string()
    }
}

#[test]
fn it_works() {
    let mut buf = Vec::new();
    encode_into(&Testme { a: 1, b: vec![1, 2, 3] }, &mut buf).unwrap();
    encode_into(&Testme { a: 2, b: vec![1, 2, 3] }, &mut buf).unwrap();
    let mut b = buf.as_slice();
    let a = decode_from::<Testme>(&mut b).unwrap();
    let b = decode_from::<Testme>(&mut b).unwrap();
    assert_eq!(a, Testme { a: 1, b: vec![1, 2, 3] });
    assert_eq!(b, Testme { a: 2, b: vec![1, 2, 3] });

    let testme = Testme { a: 1, b: vec![1, 2, 3] };
    let a = Testme::schema_container();
    eprintln!("{:?}", a);
    let a = borsh::schema_helpers::try_to_vec_with_schema(&testme).unwrap();
    let decoded = borsh::schema_helpers::try_from_slice_with_schema(&a).unwrap();
    // let encoded = testme.try_to_vec().unwrap();
    // let decoded = Testme::try_from_slice(&a).unwrap();
    assert_eq!(testme, decoded);
}
