//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use serde::{Deserialize, Deserializer};
use serde_json as json;
use tari_bor::{cbor, encode, to_value};
use tari_template_lib::{
    arg,
    args::Arg,
    models::{Amount, Metadata},
};

use crate::{substate::SubstateId, template::parse_template_address, TemplateAddress};

pub fn json_deserialize<'de, D>(d: D) -> Result<Vec<Arg>, D::Error>
where D: Deserializer<'de> {
    if d.is_human_readable() {
        // human_readable !== json. This is why the function name is json_deserialize
        let value = json::Value::deserialize(d)?;
        match value {
            json::Value::Array(args) => args
                .into_iter()
                .map(|arg| convert_value_to_arg(arg).map_err(serde::de::Error::custom))
                .collect(),
            // Vec<Arg> should always be a json::Value::Array
            v => Err(serde::de::Error::custom(format!(
                "Unexpected value: {}. Expected JSON array.",
                v
            ))),
        }
    } else {
        Vec::<Arg>::deserialize(d)
    }
}

fn convert_value_to_arg(arg: json::Value) -> Result<Arg, ArgParseError> {
    if let Some(s) = arg.as_str() {
        parse_arg(s)
    } else if is_arg_json(&arg) {
        let parsed = json::from_value(arg)?;
        Ok(parsed)
    } else {
        let value = convert_to_cbor(arg);
        let arg = Arg::literal(value)?;
        Ok(arg)
    }
}

/// Checks if the value provided is in the form {"Literal": \[bytes...]} or {"Workspace": \[bytes...]}
fn is_arg_json(arg: &json::Value) -> bool {
    let Some(obj) = arg.as_object() else {
        return false;
    };

    if !obj.contains_key("Literal") && !obj.contains_key("Workspace") {
        return false;
    }

    let v = obj
        .get("Literal")
        .or_else(|| obj.get("Workspace"))
        .expect("Already checked");
    v.is_array()
}

/// Parses a custom string syntax that represents common argument types.
///
/// e.g. Amount(123) becomes an Amount type
/// component_xxxx.. becomes a ComponentAddress type etc
pub fn parse_arg(s: &str) -> Result<Arg, ArgParseError> {
    let ty = try_parse_special_string_arg(s)?;
    Ok(ty.into())
}

fn try_parse_special_string_arg(s: &str) -> Result<ParsedArg<'_>, ArgParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(ParsedArg::String(""));
    }

    if s.chars().all(|c| c.is_ascii_digit() || c == '-') {
        if let Ok(ty) = s
            .parse()
            .map(ParsedArg::UnsignedInteger)
            .or_else(|_| s.parse().map(ParsedArg::SignedInteger))
        {
            return Ok(ty);
        }
    }

    if let Some(contents) = strip_cast_func(s, "Amount") {
        let amt = contents
            .parse()
            .map(Amount)
            .map_err(|_| ArgParseError::ExpectedAmount {
                got: contents.to_string(),
            })?;
        return Ok(ParsedArg::Amount(amt));
    }

    if let Some(contents) = strip_cast_func(s, "Workspace") {
        return Ok(ParsedArg::Workspace(contents.as_bytes().to_vec()));
    }

    if let Ok(address) = SubstateId::from_str(s) {
        return Ok(ParsedArg::SubstateId(address));
    }

    if let Some(address) = parse_template_address(s) {
        return Ok(ParsedArg::TemplateAddress(address));
    }

    if let Ok(metadata) = Metadata::from_str(s) {
        return Ok(ParsedArg::Metadata(metadata));
    }

    match s {
        "true" => return Ok(ParsedArg::Bool(true)),
        "false" => return Ok(ParsedArg::Bool(false)),
        _ => (),
    }

    if let Ok(bytes) = hex::decode(s) {
        return Ok(ParsedArg::Bytes(bytes));
    }

    Ok(ParsedArg::String(s))
}

/// Strips off "casting" syntax and returns the contents e.g. Foo(bar baz) returns "bar baz". Or None if there is no
/// cast in the input string.
fn strip_cast_func<'a>(s: &'a str, cast: &str) -> Option<&'a str> {
    s.strip_prefix(cast)
        .and_then(|s| s.strip_prefix('('))
        .and_then(|s| s.strip_suffix(')'))
}

pub enum ParsedArg<'a> {
    Amount(Amount),
    String(&'a str),
    Workspace(Vec<u8>),
    Bytes(Vec<u8>),
    SubstateId(SubstateId),
    TemplateAddress(TemplateAddress),
    UnsignedInteger(u64),
    SignedInteger(i64),
    Bool(bool),
    Metadata(Metadata),
}

impl From<ParsedArg<'_>> for Arg {
    fn from(value: ParsedArg<'_>) -> Self {
        match value {
            ParsedArg::Amount(v) => arg!(v),
            ParsedArg::String(v) => arg!(v),
            ParsedArg::SubstateId(v) => match v {
                SubstateId::Component(v) => arg!(v),
                SubstateId::Resource(v) => arg!(v),
                SubstateId::Vault(v) => arg!(v),
                SubstateId::UnclaimedConfidentialOutput(v) => arg!(v),
                SubstateId::NonFungible(v) => arg!(v),
                SubstateId::NonFungibleIndex(v) => arg!(v),
                SubstateId::TransactionReceipt(v) => arg!(v),
                SubstateId::FeeClaim(v) => arg!(v),
            },
            ParsedArg::TemplateAddress(v) => arg!(v),
            ParsedArg::UnsignedInteger(v) => arg!(v),
            ParsedArg::SignedInteger(v) => arg!(v),
            ParsedArg::Bool(v) => arg!(v),
            // Ensure bytes are encoded as Cbor Bytes, not Array<u8>
            ParsedArg::Bytes(v) => Arg::Literal(encode(&tari_bor::Value::Bytes(v)).unwrap()),
            ParsedArg::Workspace(s) => arg!(Workspace(s)),
            ParsedArg::Metadata(m) => arg!(m),
        }
    }
}

fn convert_to_cbor(value: json::Value) -> tari_bor::Value {
    match value {
        json::Value::Null => tari_bor::Value::Null,
        json::Value::Bool(v) => tari_bor::Value::Bool(v),
        json::Value::Number(n) => n
            .as_i64()
            .map(|v| tari_bor::Value::Integer(v.into()))
            .or_else(|| n.as_f64().map(tari_bor::Value::Float))
            .expect("A JSON number is always convertable to an integer or a float"),
        // Allow special string parsing within nested arrays and objects
        json::Value::String(s) => match try_parse_special_string_arg(&s) {
            Ok(parsed) => match parsed {
                ParsedArg::Amount(amount) => tari_bor::Value::Integer(amount.value().into()),
                ParsedArg::String(s) => tari_bor::Value::Text(s.to_string()),
                ParsedArg::Workspace(key) => cbor!({"Workspace" => key}).unwrap(),
                ParsedArg::SubstateId(s) => match s {
                    SubstateId::Component(id) => to_value(&id).unwrap(),
                    SubstateId::Resource(id) => to_value(&id).unwrap(),
                    SubstateId::Vault(id) => to_value(&id).unwrap(),
                    SubstateId::UnclaimedConfidentialOutput(id) => to_value(&id).unwrap(),
                    SubstateId::NonFungible(id) => to_value(&id).unwrap(),
                    SubstateId::NonFungibleIndex(id) => to_value(&id).unwrap(),
                    SubstateId::TransactionReceipt(id) => to_value(&id).unwrap(),
                    SubstateId::FeeClaim(id) => to_value(&id).unwrap(),
                },
                ParsedArg::TemplateAddress(address) => to_value(&address).unwrap(),
                ParsedArg::UnsignedInteger(i) => tari_bor::Value::Integer(i.into()),
                ParsedArg::SignedInteger(i) => tari_bor::Value::Integer(i.into()),
                ParsedArg::Bool(b) => tari_bor::Value::Bool(b),
                ParsedArg::Metadata(metadata) => to_value(&metadata).unwrap(),
                ParsedArg::Bytes(bytes) => tari_bor::Value::Bytes(bytes),
            },
            Err(_) => tari_bor::Value::Text(s),
        },
        json::Value::Array(arr) => tari_bor::Value::Array(arr.into_iter().map(convert_to_cbor).collect::<Vec<_>>()),
        json::Value::Object(map) => tari_bor::Value::Map(
            map.into_iter()
                .map(|(k, v)| (tari_bor::Value::Text(k), convert_to_cbor(v)))
                .collect(),
        ),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArgParseError {
    #[error("Expected an integer, got '{got}'")]
    ExpectedAmount { got: String },
    #[error("JSON error: {0}")]
    JsonError(#[from] json::Error),
    #[error("CBOR error: {0}")]
    BorError(#[from] tari_bor::BorError),
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use tari_bor::decode_exact;
    use tari_template_lib::{
        args,
        models::{ComponentAddress, ResourceAddress},
    };

    use super::*;
    use crate::serde_with;

    #[test]
    fn struct_test() {
        #[derive(PartialEq, Deserialize, Debug, Serialize)]
        struct SomeArgs {
            #[serde(deserialize_with = "json_deserialize")]
            args: Vec<Arg>,
        }

        let args = SomeArgs {
            args: args!(ResourceAddress::new(Default::default())),
        };
        // Serialize and deserialize from JSON representation
        let s = json::to_string(&args).unwrap();
        let from_str: SomeArgs = json::from_str(&s).unwrap();
        assert_eq!(args, from_str);

        // Deserialize from special string representation
        let some_args: SomeArgs =
            json::from_str(r#"{"args": ["component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d"] }"#)
                .unwrap();
        match &some_args.args[0] {
            Arg::Workspace(_) => panic!(),
            Arg::Literal(a) => {
                let a: ComponentAddress = decode_exact(a).unwrap();
                assert_eq!(
                    a.to_string(),
                    "component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d"
                );
            },
        }
    }

    #[test]
    fn it_parses_args_into_bor() {
        #[derive(PartialEq, Deserialize, Debug, Serialize)]
        struct SomeArgs {
            #[serde(deserialize_with = "json_deserialize")]
            args: Vec<Arg>,
        }

        #[derive(PartialEq, Deserialize, Debug, Serialize)]
        struct StructInWasm {
            name: String,
            number: u64,
            float: f64,
            boolean: bool,
            array: Vec<String>,
            map: std::collections::HashMap<String, String>,
            #[serde(with = "serde_with::string::option")]
            opt: Option<ComponentAddress>,
        }

        let struct_sample = StructInWasm {
            name: "John".to_string(),
            number: 123,
            float: 1.2,
            boolean: true,
            array: vec!["a".to_string(), "b".to_string()],
            map: [("c", "d"), ("e", "f")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            opt: Some(
                ComponentAddress::from_str("component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d")
                    .unwrap(),
            ),
        };

        let args = SomeArgs {
            args: args!(struct_sample),
        };
        // Serialize and deserialize from JSON representation
        let s = json::to_string(&args).unwrap();
        let from_str: SomeArgs = json::from_str(&s).unwrap();
        assert_eq!(args, from_str);

        // Deserialize from special string representation
        let some_args: SomeArgs = json::from_str(&format!(
            r#"{{"args": [{}]}}"#,
            json::to_string(&struct_sample).unwrap()
        ))
        .unwrap();
        let bytes = some_args.args[0].as_literal_bytes().unwrap();
        let a: StructInWasm = decode_exact(bytes).unwrap();
        assert_eq!(a, struct_sample);
    }

    #[test]
    fn it_parses_amounts() {
        let a = parse_arg("Amount(123)").unwrap();
        assert_eq!(a, arg!(Amount(123)));

        let a = parse_arg("Amount(-123)").unwrap();
        assert_eq!(a, arg!(Amount(-123)));
    }

    #[test]
    fn it_errors_if_amount_cast_is_incorrect() {
        let e = parse_arg("Amount(xyz)").unwrap_err();
        assert!(matches!(e, ArgParseError::ExpectedAmount { .. }));
    }

    #[test]
    fn it_parses_integers() {
        let u64_max = u64::MAX.to_string();
        let i64_min = i64::MIN.to_string();

        let cases = &[
            ("123", arg!(123u64)),
            ("-123", arg!(-123i64)),
            ("0", arg!(0u64)),
            (u64_max.as_str(), arg!(u64::MAX)),
            (i64_min.as_str(), arg!(i64::MIN)),
        ];

        for (case, expected) in cases {
            let a = parse_arg(case).unwrap();
            assert_eq!(a, *expected, "Unexpected value for case '{}'", case);
        }
    }

    #[test]
    fn it_parses_addresses() {
        let cases = &[
            "component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d",
            "resource_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d",
            "vault_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d",
        ];

        for case in cases {
            let a = parse_arg(case).unwrap();

            match SubstateId::from_str(case).unwrap() {
                SubstateId::Component(c) => {
                    assert_eq!(a, arg!(c), "Unexpected value for case '{}'", case);
                },
                SubstateId::Resource(r) => {
                    assert_eq!(a, arg!(r), "Unexpected value for case '{}'", case);
                },
                SubstateId::Vault(v) => {
                    assert_eq!(a, arg!(v), "Unexpected value for case '{}'", case);
                },
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn it_parses_template_addresses() {
        // valid template addreses are parsed
        let valid_template_address = "template_a9c017256ed22cb004c001b0db965a40b91ad557e1ace408ce306227d95f0f1c";
        let a = parse_arg(valid_template_address).unwrap();
        assert_eq!(
            a,
            arg!(
                TemplateAddress::from_str("a9c017256ed22cb004c001b0db965a40b91ad557e1ace408ce306227d95f0f1c").unwrap()
            )
        );

        // invalid template addreses are ignored
        let invalid_template_address = "template_xxxxxx";
        let a = parse_arg(invalid_template_address).unwrap();
        assert_eq!(a, arg!(invalid_template_address));
    }

    #[test]
    fn it_returns_string_lit_if_string_or_unknown() {
        let cases = &["this is a string", "123ab"];

        for case in cases {
            let a = parse_arg(case).unwrap();
            assert_eq!(a, arg!(case));
        }
    }

    #[test]
    fn it_parses_workspace_references() {
        let a = parse_arg("Workspace(abc)").unwrap();
        assert_eq!(a, arg!(Workspace("abc")));
    }
}
