//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{collections::HashMap, fs, str::FromStr};

use tari_bor::cbor;
use tari_engine_types::substate::SubstateId;
use tari_ootle_transaction::{
    AllocatableAddressType,
    ComponentReference,
    Instruction,
    args::{InstructionArg, WorkspaceOffsetId},
    call_arg,
    call_args,
};
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    ObjectKey,
    ResourceAddress,
    TemplateAddress,
    constants::TARI_TOKEN,
    crypto::RistrettoPublicKeyBytes,
    hex::bytes_from_hex,
};
use tari_transaction_manifest::{ManifestInstructions, ManifestValue, parse_manifest};

#[test]
#[allow(clippy::too_many_lines)]
fn manifest_smoke_test() {
    let input = fs::read_to_string("tests/examples/picture_seller.rs").unwrap();
    let account_component = ComponentAddress::new([0u8; ObjectKey::LENGTH].into());
    let test_faucet_component = ComponentAddress::new([2u8; ObjectKey::LENGTH].into());
    let picture_seller_template =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let globals = HashMap::from([
        ("account".to_string(), SubstateId::Component(account_component).into()),
        (
            "test_faucet".to_string(),
            SubstateId::Component(test_faucet_component).into(),
        ),
    ]);
    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(&input, globals, Default::default(), Default::default()).unwrap();

    let expected = vec![
        Instruction::CallFunction {
            address: picture_seller_template,
            function: "new".try_into().unwrap(),
            args: call_args![1_000u64],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
        Instruction::CallMethod {
            call: test_faucet_component.into(),
            method: "take_free_coins".try_into().unwrap(),
            args: call_args![1000],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 1 },
        Instruction::CallMethod {
            call: account_component.into(),
            method: "deposit".try_into().unwrap(),
            args: call_args![Workspace(1)],
        },
        Instruction::CallMethod {
            call: account_component.into(),
            method: "set_public_key".try_into().unwrap(),
            args: call_args![
                RistrettoPublicKeyBytes::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                    .unwrap(),
                ComponentAddress::from_str(
                    "component_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                )
                .unwrap(),
                cbor!({"some" => {"data" => [1, 2, 3]}})
            ],
        },
        Instruction::CallMethod {
            call: account_component.into(),
            method: "withdraw".try_into().unwrap(),
            args: call_args![TARI_TOKEN, 1_000],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 2 },
        Instruction::CallMethod {
            call: ComponentReference::Workspace(0),
            method: "buy".try_into().unwrap(),
            args: call_args![Workspace(2)],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 3 },
        Instruction::CallMethod {
            call: account_component.into(),
            method: "deposit".try_into().unwrap(),
            args: call_args![Workspace(3)],
        },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn workspace_component_reference() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let comp = MyTemplate::new();
            let result = comp.do_something();
        }
    "#;

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let expected = vec![
        Instruction::CallFunction {
            address: template_addr,
            function: "new".try_into().unwrap(),
            args: call_args![],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
        Instruction::CallMethod {
            call: ComponentReference::Workspace(0),
            method: "do_something".try_into().unwrap(),
            args: call_args![],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 1 },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn allocate_address_macros() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let component_addr = new_component_addr!();
            let resx_addr = new_resource_addr!();
            let comp = MyTemplate::with_address(component_addr, resx_addr);
        }
    "#;

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let expected = vec![
        Instruction::AllocateAddress {
            allocatable_type: AllocatableAddressType::Component,
            workspace_id: 0,
        },
        Instruction::AllocateAddress {
            allocatable_type: AllocatableAddressType::Resource,
            workspace_id: 1,
        },
        Instruction::CallFunction {
            address: template_addr,
            function: "with_address".try_into().unwrap(),
            args: call_args![Workspace(0), Workspace(1)],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 2 },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn local_function_inlining() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn setup() {
            let comp = MyTemplate::new();
            let result = comp.do_something();
        }

        fn main() {
            setup();
            MyTemplate::final_step();
        }
    "#;

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    // setup() should be inlined: its instructions appear first, then final_step()
    let expected = vec![
        // From setup():
        Instruction::CallFunction {
            address: template_addr,
            function: "new".try_into().unwrap(),
            args: call_args![],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
        Instruction::CallMethod {
            call: ComponentReference::Workspace(0),
            method: "do_something".try_into().unwrap(),
            args: call_args![],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 1 },
        // From main() after setup():
        Instruction::CallFunction {
            address: template_addr,
            function: "final_step".try_into().unwrap(),
            args: call_args![],
        },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn recursive_function_exceeds_call_depth() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn recurse() {
            recurse();
        }

        fn main() {
            recurse();
        }
    "#;

    let result = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default());
    let err = match result {
        Ok(_) => panic!("Expected MaxCallDepthExceeded error, but got Ok"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("Maximum call depth"),
        "Expected MaxCallDepthExceeded error, got: {err}"
    );
}

#[test]
fn exponential_function_expansion_is_capped() {
    // Depth stays under the call-depth limit; the fan-out is what grows.
    let mut manifest = String::from(
        r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn leaf() {
            let comp = MyTemplate::new();
        }
    "#,
    );
    let mut prev = "leaf".to_string();
    for i in 0..14 {
        let name = format!("f{i}");
        manifest.push_str(&format!(
            "fn {name}() {{ {prev}(); {prev}(); {prev}(); {prev}(); }}
"
        ));
        prev = name;
    }
    manifest.push_str(&format!(
        "fn main() {{ {prev}(); }}
"
    ));

    let result = parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default());
    let err = match result {
        Ok(_) => panic!("Expected TooManyInstructions error, but got Ok"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("expands to more than"),
        "Expected TooManyInstructions error, got: {err}"
    );
}

#[test]
fn exponential_expansion_of_empty_bodies_is_capped() {
    let mut manifest = String::from("fn leaf() {}\n");
    let mut prev = "leaf".to_string();
    for i in 0..14 {
        let name = format!("f{i}");
        let calls = format!("{prev}(); ").repeat(8);
        manifest.push_str(&format!("fn {name}() {{ {calls} }}\n"));
        prev = name;
    }
    manifest.push_str(&format!("fn main() {{ {prev}(); }}\n"));

    let err = parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default())
        .err()
        .expect("Expected TooManyInstructions error, but got Ok");
    assert!(
        err.to_string().contains("expands to more than"),
        "Expected TooManyInstructions error, got: {err}"
    );
}

#[test]
fn create_account_simple() {
    let manifest = r#"
        fn main() {
            let owner_pk = var!["owner_pk"];
            let account = create_account!(owner_pk);
        }
    "#;

    let pk_bytes = [42u8; 32];
    let globals = HashMap::from([(
        "owner_pk".to_string(),
        ManifestValue::Value(tari_bor::Value::Bytes(pk_bytes.to_vec())),
    )]);

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let expected = vec![
        Instruction::CreateAccount {
            owner_public_key: RistrettoPublicKeyBytes::from(pk_bytes),
            owner_rule: None,
            access_rules: None,
            bucket_workspace_id: None,
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn create_account_with_bucket() {
    let manifest = r#"
        fn main() {
            let owner_pk = var!["owner_pk"];
            let source = var!["source"];
            let bucket = source.withdraw(TARI, 10);
            let account = create_account!(owner_pk, bucket = bucket);
        }
    "#;

    let pk_bytes = [42u8; 32];
    let source_component = ComponentAddress::new([1u8; ObjectKey::LENGTH].into());
    let globals = HashMap::from([
        (
            "owner_pk".to_string(),
            ManifestValue::Value(tari_bor::Value::Bytes(pk_bytes.to_vec())),
        ),
        ("source".to_string(), SubstateId::Component(source_component).into()),
    ]);

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let expected = vec![
        Instruction::CallMethod {
            call: source_component.into(),
            method: "withdraw".try_into().unwrap(),
            args: call_args![TARI_TOKEN, 10],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
        Instruction::CreateAccount {
            owner_public_key: RistrettoPublicKeyBytes::from(pk_bytes),
            owner_rule: None,
            access_rules: None,
            bucket_workspace_id: Some(WorkspaceOffsetId::new(0)),
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 1 },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn create_account_without_assignment() {
    let manifest = r#"
        fn main() {
            let owner_pk = var!["owner_pk"];
            create_account!(owner_pk);
        }
    "#;

    let pk_bytes = [42u8; 32];
    let globals = HashMap::from([(
        "owner_pk".to_string(),
        ManifestValue::Value(tari_bor::Value::Bytes(pk_bytes.to_vec())),
    )]);

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let expected = vec![Instruction::CreateAccount {
        owner_public_key: RistrettoPublicKeyBytes::from(pk_bytes),
        owner_rule: None,
        access_rules: None,
        bucket_workspace_id: None,
    }];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn none_literal() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            MyTemplate::create("hello", None, 42);
        }
    "#;

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let expected = vec![Instruction::CallFunction {
        address: template_addr,
        function: "create".try_into().unwrap(),
        args: call_args!["hello", Literal(Option::<()>::None), 42],
    }];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn metadata_macro_empty() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            MyTemplate::create(metadata![]);
        }
    "#;

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    use tari_template_lib_types::Metadata;
    let expected = vec![Instruction::CallFunction {
        address: template_addr,
        function: "create".try_into().unwrap(),
        args: call_args![Metadata::new()],
    }];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn metadata_macro_with_values() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            MyTemplate::create(metadata!["key=value"]);
        }
    "#;

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    use tari_template_lib_types::Metadata;
    let expected_metadata: Metadata = "key=value".parse().unwrap();
    let expected = vec![Instruction::CallFunction {
        address: template_addr,
        function: "create".try_into().unwrap(),
        args: call_args![expected_metadata],
    }];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn function_call_literals_name_their_macro() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            MyTemplate::create(Amount(100));
        }
    "#;

    let err = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default())
        .err()
        .expect("a function-call literal must not parse");
    assert!(err.to_string().contains("amount!"), "{err}");
}

#[test]
fn typed_metadata_values() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            MyTemplate::create(metadata!({
                "name": "My NFT",
                "index": -1,
                "resource": address!("resource_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
                "price": amount!(1000),
            }));
        }
    "#;

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();
    let resource =
        ResourceAddress::from_str("resource_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();

    use tari_template_lib_types::{Amount, Metadata};
    let mut expected_metadata = Metadata::new();
    expected_metadata
        .insert("name", "My NFT")
        .insert("index", &-1i64)
        .insert("resource", &resource)
        .insert("price", &Amount::from(1000u64));
    assert_eq!(instructions, vec![Instruction::CallFunction {
        address: template_addr,
        function: "create".try_into().unwrap(),
        args: call_args![expected_metadata],
    }]);
}

#[test]
fn negative_integer_arguments() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            MyTemplate::create(-5, -128i8);
        }
    "#;

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();
    assert_eq!(instructions, vec![Instruction::CallFunction {
        address: template_addr,
        function: "create".try_into().unwrap(),
        args: call_args![-5i64, -128i8],
    }]);

    let out_of_range = manifest.replace("-128i8", "-129i8");
    assert!(parse_manifest(&out_of_range, HashMap::new(), Default::default(), Default::default()).is_err());
}

#[test]
fn tuple_destructuring() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let comp = var!["comp"];
            let (a, b) = comp.redeem(100);
            comp.deposit(a);
            comp.deposit(b);
        }
    "#;

    let comp_address = ComponentAddress::new([5u8; ObjectKey::LENGTH].into());
    let globals = HashMap::from([("comp".to_string(), SubstateId::Component(comp_address).into())]);

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    use tari_ootle_transaction::args::InstructionArg;

    let expected = vec![
        // comp.redeem(100) -> workspace key 0
        Instruction::CallMethod {
            call: comp_address.into(),
            method: "redeem".try_into().unwrap(),
            args: call_args![100],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
        // comp.deposit(a) where a = workspace 0, offset 0
        Instruction::CallMethod {
            call: comp_address.into(),
            method: "deposit".try_into().unwrap(),
            args: vec![InstructionArg::Workspace(WorkspaceOffsetId::new(0).with_offset(0))],
        },
        // comp.deposit(b) where b = workspace 0, offset 1
        Instruction::CallMethod {
            call: comp_address.into(),
            method: "deposit".try_into().unwrap(),
            args: vec![InstructionArg::Workspace(WorkspaceOffsetId::new(0).with_offset(1))],
        },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn tuple_destructuring_template_call() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let (x, y, z) = MyTemplate::split(42);
        }
    "#;

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let ManifestInstructions {
        instructions,
        fee_instructions,
        ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let expected = vec![
        Instruction::CallFunction {
            address: template_addr,
            function: "split".try_into().unwrap(),
            args: call_args![42],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
    ];

    assert_eq!(instructions, expected);
    assert_eq!(fee_instructions, vec![]);
}

/// `blob!(name)` should resolve to `InstructionArg::Blob(idx)` against the supplied blob map,
/// with the `Blobs` output ordered by first reference. Repeated references reuse the same
/// index.
#[test]
fn blob_macro_resolves_to_indexed_arg() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let comp = MyTemplate::new(blob!(payload_a));
            comp.update(blob!("payload_b"), blob!(payload_a));
        }
    "#;

    let template_addr =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let mut blob_inputs = HashMap::new();
    blob_inputs.insert(
        "payload_a".to_string(),
        tari_ootle_transaction::Blob::from(vec![1u8, 2, 3]),
    );
    blob_inputs.insert(
        "payload_b".to_string(),
        tari_ootle_transaction::Blob::from(vec![9u8, 8]),
    );

    let ManifestInstructions {
        instructions,
        fee_instructions,
        blobs,
    } = parse_manifest(manifest, HashMap::new(), Default::default(), blob_inputs).unwrap();

    // payload_a was referenced first so it gets index 0; payload_b is index 1.
    assert_eq!(blobs.len(), 2);
    assert_eq!(blobs.get(0).unwrap().as_bytes(), &[1u8, 2, 3]);
    assert_eq!(blobs.get(1).unwrap().as_bytes(), &[9u8, 8]);

    use tari_ootle_transaction::args::InstructionArg;
    assert_eq!(instructions[0], Instruction::CallFunction {
        address: template_addr,
        function: "new".try_into().unwrap(),
        args: vec![InstructionArg::Blob(0)],
    });
    // The second method call reuses payload_a — same index 0 — and adds payload_b at 1.
    assert_eq!(instructions[2], Instruction::CallMethod {
        call: ComponentReference::Workspace(0),
        method: "update".try_into().unwrap(),
        args: vec![InstructionArg::Blob(1), InstructionArg::Blob(0)],
    });
    assert_eq!(fee_instructions, vec![]);
}

#[test]
fn publish_template_macro_resolves_to_publish_template_instruction() {
    let manifest = r#"
        fn main() {
            publish_template!(wasm);
        }
    "#;

    let mut blob_inputs = HashMap::new();
    blob_inputs.insert(
        "wasm".to_string(),
        tari_ootle_transaction::Blob::from(vec![1u8, 2, 3, 4]),
    );

    let ManifestInstructions {
        instructions, blobs, ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), blob_inputs).unwrap();

    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs.get(0).unwrap().as_bytes(), &[1u8, 2, 3, 4]);
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0], Instruction::PublishTemplate {
        binary: 0,
        metadata_hash: None,
    });
}

#[test]
fn publish_template_resolves_blob_let_binding() {
    let manifest = r#"
        fn main() {
            let template = blob!("wasm");
            publish_template!(template);
        }
    "#;

    let mut blob_inputs = HashMap::new();
    blob_inputs.insert(
        "wasm".to_string(),
        tari_ootle_transaction::Blob::from(vec![1u8, 2, 3, 4]),
    );

    let ManifestInstructions {
        instructions, blobs, ..
    } = parse_manifest(manifest, HashMap::new(), Default::default(), blob_inputs).unwrap();

    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs.get(0).unwrap().as_bytes(), &[1u8, 2, 3, 4]);
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0], Instruction::PublishTemplate {
        binary: 0,
        metadata_hash: None,
    });
}

#[test]
fn publish_template_macro_unknown_blob_errors() {
    let manifest = r#"
        fn main() {
            publish_template!(missing);
        }
    "#;
    let err = match parse_manifest(manifest, HashMap::new(), Default::default(), HashMap::new()) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("blob!('missing')"), "unexpected error: {err}");
}

#[test]
fn blob_macro_unknown_name_errors() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let comp = MyTemplate::new(blob!(missing));
        }
    "#;

    let err = match parse_manifest(manifest, HashMap::new(), Default::default(), HashMap::new()) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("blob!('missing')"), "unexpected error: {err}");
}

#[test]
fn blob_macro_string_that_is_not_an_identifier_errors() {
    let manifests = [
        r#"fn main() { let x = blob!("my blob"); }"#,
        r#"fn main() { let x = blob!("1bad"); }"#,
        r#"fn main() { let x = blob!(""); }"#,
        r#"fn main() { let x = blob!("a-b"); }"#,
        r#"fn main() { let x = blob!("a::b"); }"#,
        r#"fn main() { let x = blob!("r#_"); }"#,
        r#"fn main() { let x = blob!("r#self"); }"#,
        r#"fn main() { let x = blob!("'a"); }"#,
        r#"fn main() { let x = blob!("/*"); }"#,
        r#"fn main() { let x = blob!("\""); }"#,
        r#"fn main() { let x = blob!("\0"); }"#,
        r#"fn main() { let x = blob!(" foo "); }"#,
        r#"fn main() { let x = blob!("foo /* x */"); }"#,
        r#"fn main() { let x = blob!("foo // x"); }"#,
        r#"
            use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;
            fn main() { let comp = MyTemplate::new(blob!("my blob")); }
        "#,
        r#"
            use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;
            fn main() { let comp = MyTemplate::new(blob!("1bad")); }
        "#,
        r#"
            use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;
            fn main() { let comp = MyTemplate::new(blob!("")); }
        "#,
    ];

    for manifest in manifests {
        let err = match parse_manifest(manifest, HashMap::new(), Default::default(), HashMap::new()) {
            Ok(_) => panic!("expected an error for {manifest}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("not a valid identifier"), "unexpected error: {err}");
    }
}

#[test]
fn put_into_bucket_macro() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let a = MyTemplate::make_bucket();
            let b = MyTemplate::make_bucket();
            put_into_bucket!(b, a);
        }
    "#;

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    assert_eq!(instructions[4], Instruction::PutIntoBucket {
        src: WorkspaceOffsetId::new(1),
        dest: WorkspaceOffsetId::new(0),
    });
}

#[test]
fn put_into_bucket_unknown_variable_errors() {
    let manifest = r#"
        use template_c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7 as MyTemplate;

        fn main() {
            let a = MyTemplate::make_bucket();
            put_into_bucket!(missing, a);
        }
    "#;

    let err = match parse_manifest(manifest, HashMap::new(), Default::default(), HashMap::new()) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("missing"), "unexpected error: {err}");
}

const LIST_TEMPLATE: &str = "c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7";
const PK1: &str = "044bccd4d01ceb41816bc9106a836806e6f9412646ecda4c2d726d8372b2c843";
const PK2: &str = "5e8b3e7e6e3aa6d8f9c3a3c4e5d9b1f2a7c6e8d0f1a2b3c4d5e6f708192a3b4c";

fn literal(value: tari_bor::Value) -> InstructionArg {
    InstructionArg::literal(value).unwrap()
}

fn list_globals() -> HashMap<String, ManifestValue> {
    HashMap::from([
        ("k1".to_string(), PK1.parse().unwrap()),
        ("k2".to_string(), PK2.parse().unwrap()),
        (
            "gov".to_string(),
            "component_0104000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        ),
    ])
}

#[test]
fn list_of_literals() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            MyTemplate::create([1u8, 2u8,], [], ["a", -1]);
        }}
    "#
    );

    let ManifestInstructions { instructions, .. } =
        parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    assert_eq!(instructions, vec![Instruction::CallFunction {
        address: TemplateAddress::from_hex(LIST_TEMPLATE).unwrap(),
        function: "create".try_into().unwrap(),
        args: vec![
            literal(cbor!([1, 2])),
            literal(cbor!([])),
            literal(tari_bor::Value::Array(vec![cbor!("a"), cbor!(-1)])),
        ],
    }]);
}

#[test]
fn list_of_input_variables() {
    let manifest = r#"
        fn main() {
            let gov = var!["gov"];
            let k2 = var!["k2"];
            gov.set_council(1u16, [var!["k1"], k2]);
        }
    "#;

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, list_globals(), Default::default(), Default::default()).unwrap();

    let council = vec![
        RistrettoPublicKeyBytes::from_hex(PK1).unwrap(),
        RistrettoPublicKeyBytes::from_hex(PK2).unwrap(),
    ];
    let council_arg = literal(tari_bor::to_value(&council).unwrap());
    // A `Vec<RistrettoPublicKeyBytes>` parameter reads a CBOR array of byte strings.
    assert_eq!(
        council_arg,
        literal(tari_bor::Value::Array(
            council
                .iter()
                .map(|pk| tari_bor::Value::Bytes(pk.as_bytes().to_vec()))
                .collect()
        ))
    );
    assert_eq!(instructions, vec![Instruction::CallMethod {
        call: ComponentReference::Address(
            ComponentAddress::from_hex("0104000000000000000000000000000000000000000000000000000000000000").unwrap()
        ),
        method: "set_council".try_into().unwrap(),
        args: vec![call_arg!(1u16), council_arg],
    }]);
}

#[test]
fn input_variable_macro_as_argument() {
    let manifest = r#"
        fn main() {
            let gov = var!["gov"];
            gov.add(var!["k1"], arg!["k2"]);
        }
    "#;

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, list_globals(), Default::default(), Default::default()).unwrap();

    let Instruction::CallMethod { args, .. } = &instructions[0] else {
        panic!("expected a method call");
    };
    assert_eq!(*args, vec![
        literal(tari_bor::Value::Bytes(bytes_from_hex(PK1).unwrap())),
        literal(tari_bor::Value::Bytes(bytes_from_hex(PK2).unwrap())),
    ]);
}

#[test]
fn nested_lists() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            MyTemplate::create([[1u8], [2u8, amount!(3)], [None, TARI]]);
        }}
    "#
    );

    let ManifestInstructions { instructions, .. } =
        parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let Instruction::CallFunction { args, .. } = &instructions[0] else {
        panic!("expected a function call");
    };
    assert_eq!(*args, vec![literal(tari_bor::Value::Array(vec![
        cbor!([1]),
        tari_bor::Value::Array(vec![cbor!(2), tari_bor::to_value(&Amount::new(3)).unwrap()]),
        tari_bor::Value::Array(vec![tari_bor::Value::Null, tari_bor::to_value(&TARI_TOKEN).unwrap()]),
    ]))]);
}

#[test]
fn list_rejects_workspace_values() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            let bucket = MyTemplate::make();
            MyTemplate::create([1u8, bucket]);
        }}
    "#
    );

    let err = parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default())
        .err()
        .expect("a workspace value cannot be a list element");
    assert!(err.to_string().contains("bucket"), "{err}");
}

#[test]
fn list_rejects_blobs() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            MyTemplate::create([blob!(data)]);
        }}
    "#
    );

    let blobs = HashMap::from([("data".to_string(), tari_ootle_transaction::Blob::from(vec![1u8, 2, 3]))]);
    assert!(parse_manifest(&manifest, HashMap::new(), Default::default(), blobs).is_err());
}

#[test]
fn cbor_literal_rejects_input_variables() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            MyTemplate::create(cbor!([var!["k1"]]));
        }}
    "#
    );

    assert!(parse_manifest(&manifest, list_globals(), Default::default(), Default::default()).is_err());
}

#[test]
fn list_valued_input_variable() {
    let manifest = r#"
        fn main() {
            let gov = var!["gov"];
            gov.set_council(1u16, var!["council"]);
        }
    "#;

    let mut globals = list_globals();
    globals.insert("council".to_string(), format!("[{PK1}, {PK2}]").parse().unwrap());

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let council = vec![
        RistrettoPublicKeyBytes::from_hex(PK1).unwrap(),
        RistrettoPublicKeyBytes::from_hex(PK2).unwrap(),
    ];
    let Instruction::CallMethod { args, .. } = &instructions[0] else {
        panic!("expected a method call");
    };
    assert_eq!(args[1], literal(tari_bor::to_value(&council).unwrap()));
}

#[test]
fn list_depth_counts_nested_cbor() {
    let cbor_depth = tari_bor::MAX_DECODE_DEPTH;
    let list_depth = 1;
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            MyTemplate::create({open}cbor!({cbor_open}{cbor_close}){close});
        }}
    "#,
        open = "[".repeat(list_depth),
        close = "]".repeat(list_depth),
        cbor_open = "[".repeat(cbor_depth),
        cbor_close = "]".repeat(cbor_depth),
    );

    let err = parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default())
        .err()
        .expect("a list nested past the decode depth is rejected");
    assert!(err.to_string().contains("nests deeper"), "{err}");
}

#[test]
fn list_passes_a_tuple() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            MyTemplate::create([123, "hello"]);
        }}
    "#
    );

    let ManifestInstructions { instructions, .. } =
        parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default()).unwrap();

    let Instruction::CallFunction { args, .. } = &instructions[0] else {
        panic!("expected a function call");
    };
    let bytes = args[0].as_literal_bytes().unwrap();
    assert_eq!(bytes, tari_bor::encode(&(123u64, "hello".to_string())).unwrap());
    let decoded: (u64, String) = tari_bor::decode(bytes).unwrap();
    assert_eq!(decoded, (123, "hello".to_string()));
}

fn caller_component() -> ManifestValue {
    SubstateId::Component(ComponentAddress::new([9u8; ObjectKey::LENGTH].into())).into()
}

#[test]
fn manifest_binding_shadows_input_variable_in_every_position() {
    let manifest = r#"
        fn main() {
            let acc = new_component_addr!();
            acc.withdraw(TARI, acc);
        }
    "#;
    let globals = HashMap::from([("acc".to_string(), caller_component())]);

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let Instruction::CallMethod { call, args, .. } = &instructions[1] else {
        panic!("expected a method call");
    };
    assert_eq!(*call, ComponentReference::Workspace(0));
    assert_eq!(args[1], InstructionArg::Workspace(WorkspaceOffsetId::new(0)));
}

#[test]
fn output_binding_shadows_input_variable() {
    let manifest = r#"
        fn main() {
            let src = var!["src"];
            let bucket = src.withdraw(TARI, 10);
            src.deposit(bucket);
        }
    "#;
    let globals = HashMap::from([
        ("src".to_string(), caller_component()),
        ("bucket".to_string(), "1".parse().unwrap()),
    ]);

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let Instruction::CallMethod { args, .. } = &instructions[2] else {
        panic!("expected a method call");
    };
    assert_eq!(*args, vec![InstructionArg::Workspace(WorkspaceOffsetId::new(0))]);
}

#[test]
fn later_binding_shadows_earlier_input_alias() {
    let manifest = r#"
        fn main() {
            let acc = var!["acc"];
            let acc = new_component_addr!();
            acc.withdraw(TARI, acc);
        }
    "#;
    let globals = HashMap::from([("acc".to_string(), caller_component())]);

    let ManifestInstructions { instructions, .. } =
        parse_manifest(manifest, globals, Default::default(), Default::default()).unwrap();

    let Instruction::CallMethod { call, args, .. } = &instructions[1] else {
        panic!("expected a method call");
    };
    assert_eq!(*call, ComponentReference::Workspace(0));
    assert_eq!(args[1], InstructionArg::Workspace(WorkspaceOffsetId::new(0)));
}

#[test]
fn list_rejects_workspace_value_shadowing_input_variable() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as MyTemplate;

        fn main() {{
            let bucket = MyTemplate::make();
            MyTemplate::create([1u8, bucket]);
        }}
    "#
    );
    let globals = HashMap::from([("bucket".to_string(), "1".parse().unwrap())]);

    let err = parse_manifest(&manifest, globals, Default::default(), Default::default())
        .err()
        .expect("a workspace value cannot be a list element");
    assert!(err.to_string().contains("bucket"), "{err}");
}

#[test]
fn template_alias_cannot_be_redefined() {
    let manifest = format!(
        r#"
        use template_{LIST_TEMPLATE} as Account;

        fn main() {{
            Account::create();
        }}
    "#
    );

    let err = parse_manifest(&manifest, HashMap::new(), Default::default(), Default::default())
        .err()
        .expect("the built-in Account alias cannot be redefined");
    assert!(err.to_string().contains("Account"), "{err}");
}
