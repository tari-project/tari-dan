//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::str::FromStr;

use syn::{Lit, parse2};
use tari_bor::{BorError, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_ootle_transaction::{args::InstructionArg, call_arg};
use tari_template_lib_types::{NonFungibleId, hex::bytes_from_hex};

use crate::error::ManifestError;

#[derive(Debug, Clone)]
pub enum ManifestValue {
    SubstateId(SubstateId),
    Literal(Lit),
    NonFungibleId(NonFungibleId),
    Value(tari_bor::Value),
}

impl ManifestValue {
    pub fn new_value<T: Serialize + tari_bor::Encode<()>>(value: &T) -> Result<Self, BorError> {
        Ok(Self::Value(tari_bor::to_value(value)?))
    }

    pub fn as_address(&self) -> Option<&SubstateId> {
        match self {
            Self::SubstateId(addr) => Some(addr),
            _ => None,
        }
    }

    pub fn to_arg(&self) -> Result<InstructionArg, ManifestError> {
        match self {
            ManifestValue::SubstateId(addr) => Ok(InstructionArg::literal(address_value(addr)?)?),
            ManifestValue::Literal(lit) => lit_to_arg(lit),
            ManifestValue::NonFungibleId(id) => Ok(call_arg!(id.clone())),
            ManifestValue::Value(blob) => Ok(InstructionArg::literal(blob.clone())?),
        }
    }

    /// The CBOR value this passes as an argument, so that it can be an element of a list.
    pub fn to_value(&self) -> Result<tari_bor::Value, ManifestError> {
        match self {
            ManifestValue::Value(value) => Ok(value.clone()),
            _ => arg_to_value(self.to_arg()?),
        }
    }
}

/// The CBOR value a literal argument holds.
pub(crate) fn arg_to_value(arg: InstructionArg) -> Result<tari_bor::Value, ManifestError> {
    let bytes = arg
        .as_literal_bytes()
        .ok_or_else(|| ManifestError::InvalidInstruction {
            reason: format!("{arg:?} is not a literal"),
        })?;
    Ok(tari_bor::decode(bytes)?)
}

/// The CBOR value `address!` writes for `id`: the typed address it holds, which is what a template
/// parameter of that address type decodes. A template parameter takes a `TemplateAddress`, which is plain
/// `Hash32` bytes, so a template address is written untagged.
pub(crate) fn address_value(id: &SubstateId) -> Result<tari_bor::Value, BorError> {
    match id {
        SubstateId::Component(addr) => tari_bor::to_value(addr),
        SubstateId::Resource(addr) => tari_bor::to_value(addr),
        SubstateId::TransactionReceipt(addr) => tari_bor::to_value(addr),
        SubstateId::Vault(addr) => tari_bor::to_value(addr),
        SubstateId::NonFungible(addr) => tari_bor::to_value(addr),
        SubstateId::ClaimedOutputTombstone(addr) => tari_bor::to_value(addr),
        SubstateId::Template(addr) => tari_bor::to_value(&addr.as_template_address()),
        SubstateId::ValidatorFeePool(addr) => tari_bor::to_value(addr),
        SubstateId::Utxo(addr) => tari_bor::to_value(addr),
        SubstateId::ConfidentialOutput(addr) => tari_bor::to_value(addr),
    }
}

impl<T: Into<SubstateId>> From<T> for ManifestValue {
    fn from(addr: T) -> Self {
        ManifestValue::SubstateId(addr.into())
    }
}

pub fn lit_to_arg(lit: &Lit) -> Result<InstructionArg, ManifestError> {
    match lit {
        Lit::Str(s) => Ok(call_arg!(s.value())),
        Lit::Int(i) => match i.suffix() {
            "u8" => Ok(call_arg!(i.base10_parse::<u8>()?)),
            "u16" => Ok(call_arg!(i.base10_parse::<u16>()?)),
            "u32" => Ok(call_arg!(i.base10_parse::<u32>()?)),
            "u64" => Ok(call_arg!(i.base10_parse::<u64>()?)),
            "i8" => Ok(call_arg!(i.base10_parse::<i8>()?)),
            "i16" => Ok(call_arg!(i.base10_parse::<i16>()?)),
            "i32" => Ok(call_arg!(i.base10_parse::<i32>()?)),
            "i64" => Ok(call_arg!(i.base10_parse::<i64>()?)),
            "" => Ok(call_arg!(i.base10_parse::<i64>()?)),
            // 128-bit integer literals are emitted as `tari_bor::Value::Integer(i128)` and on the
            // wire become a CBOR integer (Value's encoder accepts the i64::MIN..=u64::MAX range
            // and errors at submission time for anything wider). Templates taking native `u128` /
            // `i128` parameters decode that form too, so both they and templates whose arg type is
            // `tari_bor::Value` (read back via `Value::as_integer()`) accept these literals.
            "u128" => {
                let parsed = i.base10_parse::<u128>()?;
                let as_i128 = i128::try_from(parsed).map_err(|_| {
                    ManifestError::UnsupportedExpr(format!(
                        "u128 literal {} exceeds i128 (tari_bor::Value::Integer) range",
                        i.base10_digits()
                    ))
                })?;
                Ok(InstructionArg::literal(tari_bor::Value::Integer(as_i128))?)
            },
            "i128" => {
                let parsed = i.base10_parse::<i128>()?;
                Ok(InstructionArg::literal(tari_bor::Value::Integer(parsed))?)
            },
            _ => Err(ManifestError::UnsupportedExpr(format!(
                r#"Unsupported integer suffix "{}""#,
                i.suffix()
            ))),
        },
        Lit::Bool(b) => Ok(call_arg!(b.value())),
        Lit::ByteStr(v) => Ok(call_arg!(v.value())),
        Lit::Byte(v) => Ok(call_arg!(v.value())),
        Lit::Char(v) => Ok(call_arg!(v.value().to_string())),
        Lit::Float(v) => Err(ManifestError::UnsupportedExpr(format!(
            "Float literals not supported ({})",
            v
        ))),
        Lit::Verbatim(v) => Err(ManifestError::UnsupportedExpr(format!(
            "Raw token literals not supported ({})",
            v
        ))),
        _ => Err(ManifestError::UnsupportedExpr(format!(
            "Unsupported literal type ({:?})",
            lit
        ))),
    }
}

// https://github.com/rust-lang/rfcs/issues/2758 :/
// impl From<NonFungibleId> for ManifestValue {
//     fn from(id: NonFungibleId) -> Self {
//         ManifestValue::NonFungibleId(id)
//     }
// }

impl FromStr for ManifestValue {
    type Err = ManifestParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_value(s, 0)
    }
}

/// Bounds how deeply a list nests, whether written in a manifest or passed as a variable. The engine
/// decodes nothing deeper, and the bound keeps parsing an untrusted variable off a deep native stack.
pub(crate) const MAX_LIST_DEPTH: usize = tari_bor::MAX_DECODE_DEPTH;

/// Parses `s` as a value nested `depth` lists deep.
fn parse_value(s: &str, depth: usize) -> Result<ManifestValue, ManifestParseError> {
    if let Some(inner) = s.trim().strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if depth >= MAX_LIST_DEPTH {
            return Err(ManifestParseError(format!(
                "list nested deeper than {MAX_LIST_DEPTH} levels"
            )));
        }
        return split_list(inner)
            .ok_or_else(|| ManifestParseError(s.to_string()))?
            .into_iter()
            .map(|elem| {
                parse_value(elem, depth + 1)?
                    .to_value()
                    .map_err(|_| ManifestParseError(elem.to_string()))
            })
            .collect::<Result<_, _>>()
            .map(|items| ManifestValue::Value(tari_bor::Value::Array(items)));
    }

    SubstateId::from_str(s)
        .ok()
        .map(ManifestValue::SubstateId)
        .or_else(|| {
            let id = NonFungibleId::try_from_canonical_string(s).ok()?;
            Some(ManifestValue::NonFungibleId(id))
        })
        .or_else(|| {
            let tokens = s.parse().ok()?;
            let lit: Lit = parse2(tokens).ok()?;
            // Reject literals that are not supported or have unrecognized suffixes (e.g. hex strings
            // like "044bccd4..." that syn misinterprets as integer + suffix, or "1e23" as float).
            match &lit {
                Lit::Str(_) | Lit::Bool(_) | Lit::ByteStr(_) | Lit::Byte(_) | Lit::Char(_) => {},
                Lit::Int(i) => match i.suffix() {
                    "" | "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" => {},
                    _ => return None,
                },
                _ => return None,
            }
            Some(ManifestValue::Literal(lit))
        })
        .or_else(|| {
            // Try parsing as hex bytes (e.g. public keys)
            let bytes = bytes_from_hex(s).ok()?;
            Some(ManifestValue::Value(tari_bor::Value::Bytes(bytes)))
        })
        .ok_or_else(|| ManifestParseError(s.to_string()))
}

/// The comma-separated elements of a list's contents, split at the commas outside any nested list
/// or quoted literal. A trailing comma is allowed; an empty element is not.
fn split_list(inner: &str) -> Option<Vec<&str>> {
    let mut elems = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        if let Some(q) = quote {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                _ if c == q => quote = None,
                _ => {},
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '[' => depth += 1,
            ']' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                elems.push(inner[start..i].trim());
                start = i + 1;
            },
            _ => {},
        }
    }
    if depth != 0 || quote.is_some() {
        return None;
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        elems.push(last);
    }
    if elems.iter().any(|e| e.is_empty()) {
        return None;
    }
    Some(elems)
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid manifest value '{0}'")]
pub struct ManifestParseError(String);

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{ComponentAddress, ResourceAddress, TemplateAddress, VaultId};

    use super::*;

    #[test]
    fn it_parses_hex_bytes() {
        let val = "044bccd4d01ceb41816bc9106a836806e6f9412646ecda4c2d726d8372b2c843"
            .parse::<ManifestValue>()
            .unwrap();
        assert!(matches!(val, ManifestValue::Value(tari_bor::Value::Bytes(_))));

        // Hex string that looks like a float literal (contains 'e')
        let val = "1e2345".parse::<ManifestValue>().unwrap();
        assert!(matches!(val, ManifestValue::Value(tari_bor::Value::Bytes(_))));
    }

    #[test]
    fn it_parses_lists() {
        let pk = "044bccd4d01ceb41816bc9106a836806e6f9412646ecda4c2d726d8372b2c843";
        let pk_bytes = tari_bor::Value::Bytes(bytes_from_hex(pk).unwrap());
        let list = |s: &str| match s.parse::<ManifestValue>().unwrap() {
            ManifestValue::Value(tari_bor::Value::Array(items)) => items,
            v => panic!("{s} parsed as {v:?}"),
        };

        assert_eq!(list(&format!("[{pk}, {pk}]")), vec![pk_bytes.clone(), pk_bytes.clone()]);
        assert_eq!(list(&format!(" [ {pk} ,] ")), vec![pk_bytes]);
        assert_eq!(list("[]"), vec![]);
        assert_eq!(list(r#"["a,]b", 'c', 1u8]"#), vec![
            tari_bor::Value::Text("a,]b".to_string()),
            tari_bor::Value::Text("c".to_string()),
            tari_bor::Value::Integer(1),
        ]);
        assert_eq!(list("[[1], []]"), vec![
            tari_bor::Value::Array(vec![tari_bor::Value::Integer(1)]),
            tari_bor::Value::Array(vec![]),
        ]);
        let component = "component_0000000000000000000000000000000000000000000000000000000000000000";
        assert_eq!(list(&format!("[{component}]")), vec![
            ManifestValue::from_str(component).unwrap().to_value().unwrap()
        ]);

        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(deep(MAX_LIST_DEPTH).parse::<ManifestValue>().is_ok());
        assert!(deep(MAX_LIST_DEPTH + 1).parse::<ManifestValue>().is_err());
        assert!(deep(1_000_000).parse::<ManifestValue>().is_err());

        for invalid in ["[1,,2]", "[,]", "[1", "[[1]", "[\"a]", "[1]]", "[nope]"] {
            assert!(invalid.parse::<ManifestValue>().is_err(), "{invalid} parsed");
        }
    }

    /// A template parameter takes a `TemplateAddress`, so a template address passed by `address!` or by a variable
    /// must decode as one.
    #[test]
    fn a_template_address_decodes_as_a_template_address() {
        let hex = "0101010101010101010101010101010101010101010101010101010101010101";
        let template_address = TemplateAddress::from_hex(hex).unwrap();
        let id = format!("template_{hex}").parse::<SubstateId>().unwrap();

        let from_macro = address_value(&id).unwrap();
        assert_eq!(
            tari_bor::from_value::<TemplateAddress>(&from_macro).unwrap(),
            template_address
        );

        let from_variable = ManifestValue::from(id).to_value().unwrap();
        assert_eq!(from_variable, from_macro);
    }

    #[test]
    fn it_parses_address_strings() {
        let addr = "component_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            *addr.as_address().unwrap(),
            SubstateId::Component(
                ComponentAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );

        let addr = "resource_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            *addr.as_address().unwrap(),
            SubstateId::Resource(
                ResourceAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );

        let addr = "vault_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            *addr.as_address().unwrap(),
            SubstateId::Vault(
                VaultId::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );
    }
}
