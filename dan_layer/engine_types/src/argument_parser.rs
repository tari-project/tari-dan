//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use serde::{Deserialize, Deserializer};
use serde_json as json;
use tari_template_lib::{
    arg,
    args::Arg,
    models::{Amount, Metadata},
};

use crate::{substate::SubstateAddress, template::parse_template_address, TemplateAddress};

pub fn json_deserialize<'de, D>(d: D) -> Result<Vec<Arg>, D::Error>
where D: Deserializer<'de> {
    if d.is_human_readable() {
        // human_readable !== json. This is why the function name is json_deserialize
        let value = json::Value::deserialize(d)?;
        match value {
            json::Value::Array(args) => args
                .into_iter()
                .map(|arg| {
                    if let Some(s) = arg.as_str() {
                        parse_arg(s).map_err(serde::de::Error::custom)
                    } else {
                        let parsed = json::from_value(arg).map_err(serde::de::Error::custom)?;
                        Ok(parsed)
                    }
                })
                .collect(),
            _ => json::from_value(value).map_err(serde::de::Error::custom),
        }
    } else {
        Vec::<Arg>::deserialize(d)
    }
}

pub fn parse_arg(s: &str) -> Result<Arg, ArgParseError> {
    let ty = try_parse_special_string_arg(s)?;
    Ok(ty.into())
}

fn try_parse_special_string_arg(s: &str) -> Result<StringArg<'_>, ArgParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(StringArg::String(""));
    }

    if s.chars().all(|c| c.is_ascii_digit() || c == '-') {
        if let Ok(ty) = s
            .parse()
            .map(StringArg::UnsignedInteger)
            .or_else(|_| s.parse().map(StringArg::SignedInteger))
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
        return Ok(StringArg::Amount(amt));
    }

    if let Some(contents) = strip_cast_func(s, "Workspace") {
        return Ok(StringArg::Workspace(contents.as_bytes().to_vec()));
    }

    if let Ok(address) = SubstateAddress::from_str(s) {
        return Ok(StringArg::SubstateAddress(address));
    }

    if let Some(address) = parse_template_address(s.to_owned()) {
        return Ok(StringArg::TemplateAddress(address));
    }

    if let Ok(metadata) = Metadata::from_str(s) {
        return Ok(StringArg::Metadata(metadata));
    }

    match s {
        "true" => return Ok(StringArg::Bool(true)),
        "false" => return Ok(StringArg::Bool(false)),
        _ => (),
    }

    Ok(StringArg::String(s))
}

/// Strips off "casting" syntax and returns the contents e.g. Foo(bar baz) returns "bar baz". Or None if there is no
/// cast in the input string.
fn strip_cast_func<'a>(s: &'a str, cast: &str) -> Option<&'a str> {
    s.strip_prefix(cast)
        .and_then(|s| s.strip_prefix('('))
        .and_then(|s| s.strip_suffix(')'))
}

pub enum StringArg<'a> {
    Amount(Amount),
    String(&'a str),
    Workspace(Vec<u8>),
    SubstateAddress(SubstateAddress),
    TemplateAddress(TemplateAddress),
    UnsignedInteger(u64),
    SignedInteger(i64),
    Bool(bool),
    Metadata(Metadata),
}

impl From<StringArg<'_>> for Arg {
    fn from(value: StringArg<'_>) -> Self {
        match value {
            StringArg::Amount(v) => arg!(v),
            StringArg::String(v) => arg!(v),
            StringArg::SubstateAddress(v) => match v {
                SubstateAddress::Component(v) => arg!(v),
                SubstateAddress::Resource(v) => arg!(v),
                SubstateAddress::Vault(v) => arg!(v),
                SubstateAddress::UnclaimedConfidentialOutput(v) => arg!(v),
                SubstateAddress::NonFungible(v) => arg!(v),
                SubstateAddress::NonFungibleIndex(v) => arg!(v),
                SubstateAddress::TransactionReceipt(v) => arg!(v),
                SubstateAddress::FeeClaim(v) => arg!(v),
            },
            StringArg::TemplateAddress(v) => arg!(v),
            StringArg::UnsignedInteger(v) => arg!(v),
            StringArg::SignedInteger(v) => arg!(v),
            StringArg::Bool(v) => arg!(v),
            StringArg::Workspace(s) => arg!(Workspace(s)),
            StringArg::Metadata(m) => arg!(m),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArgParseError {
    #[error("Expected an integer, got '{got}'")]
    ExpectedAmount { got: String },
    #[error("JSON error: {0}")]
    JsonError(#[from] json::Error),
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
        let some_args: SomeArgs = json::from_str(
            r#"{"args": ["component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d29006db9"] }"#,
        )
        .unwrap();
        match &some_args.args[0] {
            Arg::Workspace(_) => panic!(),
            Arg::Literal(a) => {
                let a: ComponentAddress = decode_exact(a).unwrap();
                assert_eq!(
                    a.to_string(),
                    "component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d29006db9"
                );
            },
        }
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
            "component_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d29006db9",
            "resource_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d29006db9",
            "vault_4e146f73f764ddc21a89c315bd00c939cfaae7d86df082a36e47028d29006db9",
        ];

        for case in cases {
            let a = parse_arg(case).unwrap();

            match SubstateAddress::from_str(case).unwrap() {
                SubstateAddress::Component(c) => {
                    assert_eq!(a, arg!(c), "Unexpected value for case '{}'", case);
                },
                SubstateAddress::Resource(r) => {
                    assert_eq!(a, arg!(r), "Unexpected value for case '{}'", case);
                },
                SubstateAddress::Vault(v) => {
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
        let cases = &["this is a string", "123a"];

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
