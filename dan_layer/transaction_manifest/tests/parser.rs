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

use std::{collections::HashMap, fs};

use syn::{parse::Parser, Lit};
use tari_engine_types::{instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress, LayerOneCommitmentAddress, ResourceAddress, TemplateAddress},
    Hash,
};
use tari_transaction_manifest::{parse_manifest, ManifestValue};

#[test]
#[allow(clippy::too_many_lines)]
fn manifest_smoke_test() {
    let input = fs::read_to_string("tests/examples/picture_seller.rs").unwrap();
    let account_component = ComponentAddress::from([0u8; 32]);
    let picture_seller_component = ComponentAddress::from([1u8; 32]);
    let test_faucet_component = ComponentAddress::from([2u8; 32]);
    let xtr_resource = ResourceAddress::from([3u8; 32]);
    let picture_seller_template =
        TemplateAddress::from_hex("c2b621869ec2929d3b9503ea41054f01b468ce99e50254b58e460f608ae377f7").unwrap();

    let globals = HashMap::from([
        (
            "account".to_string(),
            SubstateAddress::Component(account_component).into(),
        ),
        (
            "picture_seller_addr".to_string(),
            SubstateAddress::Component(picture_seller_component).into(),
        ),
        (
            "test_faucet".to_string(),
            SubstateAddress::Component(test_faucet_component).into(),
        ),
        (
            "xtr_resource".to_string(),
            SubstateAddress::Resource(xtr_resource).into(),
        ),
    ]);
    let instructions = parse_manifest(&input, globals).unwrap();

    let expected = vec![
        Instruction::CallFunction {
            template_address: picture_seller_template,
            function: "new".to_string(),
            args: args![1_000u64],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"picture_seller".to_vec(),
        },
        Instruction::CallMethod {
            component_address: test_faucet_component,
            method: "take_free_coins".to_string(),
            args: args![Amount(1_000)],
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: b"funds".to_vec() },
        Instruction::CallMethod {
            component_address: account_component,
            method: "deposit".to_string(),
            args: args![Variable("funds")],
        },
        Instruction::CallMethod {
            component_address: account_component,
            method: "withdraw".to_string(),
            args: args![xtr_resource, Amount(1_000)],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        },
        Instruction::CallMethod {
            component_address: picture_seller_component,
            method: "buy".to_string(),
            args: args![Variable("bucket")],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"picture".to_vec(),
        },
        Instruction::CallMethod {
            component_address: account_component,
            method: "deposit".to_string(),
            args: args![Variable("picture")],
        },
    ];

    assert_eq!(instructions, expected);
}

#[test]
fn test_manifest_burn_macro() {
    let input = r#"
       let confidential_bucket = burnt_to_bucket!["COMMITMENT", "PROOF", "RANGEPROOF"];

       //let mut acc1 = global!["ACC_1/components/Account"];

    // get tokens from the faucet
    //let faucet_bucket = faucet.take_free_coins();
    // acc1.deposit(confidential_bucket);
    "#;

    let commitment = LayerOneCommitmentAddress::new(Hash::from([0u8; 32]));

    let globals = HashMap::from([
        ("COMMITMENT".to_string(), commitment.to_string().parse().unwrap()),
        ("PROOF".to_string(), "b\"111111111\"".parse().unwrap()),
        ("RANGEPROOF".to_string(), "b\"111111111\"".parse().unwrap()),
    ]);
    let instructions = parse_manifest(&input, globals).unwrap();

    let expected = vec![Instruction::ClaimBurn {
        commitment_address: [0u8; 32].to_vec(),
        range_proof: [1u8; 96].to_vec(),
        proof_of_knowledge: [2u8; 800].to_vec(),
    }];
    assert_eq!(instructions, expected);
}
