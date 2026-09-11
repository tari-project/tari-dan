//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Static extraction of a template's `TemplateDef` directly from WASM bytes,
//! without invoking cranelift.
//!
//! Templates compiled from the `tari_template_lib` macros embed their ABI definition in a custom
//! WASM section named [`TEMPLATE_DEF_CUSTOM_SECTION`] (`tari_tdef`): the bor-encoded `TemplateDef`
//! written into the section as `[u32 LE: full_len] || [bor bytes]`. The section is independent of
//! linear-memory layout, needs no global or data-segment walk, and works for any WASM toolchain that
//! can emit a custom section.
//!
//! Intended exclusively for callers that only need the type/function metadata
//! (today: only the wallet daemon's template monitor) and want to avoid the
//! cranelift compile cost. It does **not** validate the WASM module — anyone
//! using a template for execution should go through
//! `WasmModule::load_template_from_code`.

use tari_template_abi::{TEMPLATE_DEF_CUSTOM_SECTION, TemplateDef, WASM_PTR_SIZE};
use wasmer::wasmparser::{BinaryReaderError, Parser, Payload};

/// Statically extract the embedded `TemplateDef` from a template's WASM bytes.
///
/// Cheap relative to a full compile: a single linear pass over the WASM payload to find the custom
/// section. No cranelift, no instantiation, no linear-memory allocation.
///
/// All arithmetic on offsets / lengths is checked. Adversarial inputs are
/// rejected with an explicit error rather than panicking or wrapping.
pub fn extract_template_def(code: &[u8]) -> Result<TemplateDef, ExtractTemplateDefError> {
    for payload in Parser::new(0).parse_all(code) {
        if let Payload::CustomSection(reader) = payload? &&
            reader.name() == TEMPLATE_DEF_CUSTOM_SECTION
        {
            return decode_template_def_from_blob(reader.data());
        }
    }

    Err(ExtractTemplateDefError::SectionMissing)
}

/// Decode a `[u32 LE: full_len] || [bor bytes]` blob (the format produced by
/// `TemplateDef::encode_for_wasm_embedding`). `full_len` is the total length
/// including the 4-byte prefix itself; the bor payload occupies
/// `blob[4..full_len]`.
fn decode_template_def_from_blob(blob: &[u8]) -> Result<TemplateDef, ExtractTemplateDefError> {
    if blob.len() < WASM_PTR_SIZE {
        return Err(ExtractTemplateDefError::SectionTooShort { len: blob.len() });
    }
    let prefix: [u8; WASM_PTR_SIZE] = blob[..WASM_PTR_SIZE]
        .try_into()
        .expect("blob.len() >= WASM_PTR_SIZE checked above");
    let full_len = u32::from_le_bytes(prefix) as usize;
    if full_len < WASM_PTR_SIZE || full_len > blob.len() {
        return Err(ExtractTemplateDefError::SectionLengthMismatch {
            declared: full_len,
            actual: blob.len(),
        });
    }
    tari_bor::decode(&blob[WASM_PTR_SIZE..full_len]).map_err(ExtractTemplateDefError::Decode)
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractTemplateDefError {
    #[error("WASM parse error: {0}")]
    Parse(#[from] BinaryReaderError),
    #[error("Module has no `{TEMPLATE_DEF_CUSTOM_SECTION}` custom section")]
    SectionMissing,
    #[error("`{TEMPLATE_DEF_CUSTOM_SECTION}` custom section is too short ({len} bytes) to hold a length prefix")]
    SectionTooShort { len: usize },
    #[error("`{TEMPLATE_DEF_CUSTOM_SECTION}` declares length {declared} but the section is {actual} bytes")]
    SectionLengthMismatch { declared: usize, actual: usize },
    #[error("Failed to decode TemplateDef: {0}")]
    Decode(#[source] tari_bor::BorError),
}

#[cfg(test)]
mod tests {
    use tari_template_abi::{TemplateDef, TemplateDefV1, version};
    use tari_template_builtin::all_builtin_templates;

    use super::*;

    /// Encode an unsigned LEB128 integer into `out`. Just enough for the
    /// ranges we use in tests (section sizes, name lengths up to 64 KiB).
    fn write_uleb128(out: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    /// Build a minimal valid WASM binary that contains a single custom section
    /// with the given name and payload. No type/import/export/etc. sections —
    /// the extractor only needs the custom section, and `wasmparser` accepts
    /// modules that have just the magic+version header plus custom sections.
    fn make_wasm_with_custom_section(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut wasm = Vec::with_capacity(8 + 16 + name.len() + payload.len());
        // Magic + version
        wasm.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
        // Custom section: section id 0, size LEB, name length LEB, name bytes, data
        let mut section_body = Vec::with_capacity(name.len() + payload.len() + 8);
        write_uleb128(&mut section_body, name.len() as u32);
        section_body.extend_from_slice(name.as_bytes());
        section_body.extend_from_slice(payload);
        wasm.push(0); // section id 0 = custom
        write_uleb128(&mut wasm, section_body.len() as u32);
        wasm.extend_from_slice(&section_body);
        wasm
    }

    fn synthetic_template_def() -> TemplateDef {
        TemplateDef::V1(TemplateDefV1 {
            template_name: "Synthetic".to_string(),
            abi_version: version::LATEST_TEMPLATE_VERSION,
            functions: Vec::new(),
        })
    }

    #[test]
    fn extracts_builtin_template_defs() {
        for template in all_builtin_templates() {
            let def = extract_template_def(template.binary)
                .unwrap_or_else(|e| panic!("extract failed for {}: {}", template.name, e));
            assert_eq!(
                def.template_name(),
                template.name,
                "extracted template_name should match the static builtin name",
            );
        }
    }

    #[test]
    fn extracts_from_custom_section() {
        let blob = synthetic_template_def().encode_for_wasm_embedding().expect("encode");
        let wasm = make_wasm_with_custom_section(TEMPLATE_DEF_CUSTOM_SECTION, &blob);
        let def = extract_template_def(&wasm).expect("extract from custom section");
        assert_eq!(def.template_name(), "Synthetic");
    }

    #[test]
    fn rejects_malformed_section_length() {
        // Length prefix declares more bytes than the section actually holds.
        let mut blob = vec![0u8; 16];
        blob[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        let wasm = make_wasm_with_custom_section(TEMPLATE_DEF_CUSTOM_SECTION, &blob);
        match extract_template_def(&wasm) {
            Err(ExtractTemplateDefError::SectionLengthMismatch { .. }) => {},
            other => panic!("expected SectionLengthMismatch, got {:?}", other),
        }
    }
}
