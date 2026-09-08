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

//! Call info encoding and decoding for engine function calls.
//!
//! This module provides functionality to encode and decode function call information,
//! including function identifiers and arguments, in a packed binary format.
//!
//! This API allows the WASM template to lazily deserialize each argument directly without first allocating new memory
//! for some container for those arguments.

pub type FunctionIdent = u32;

#[derive(Debug, Clone)]
pub struct CallInfo;

impl CallInfo {
    #[cfg(feature = "std")]
    pub fn encode_v1_packed_size(args: &[tari_bor::Value]) -> Result<usize, tari_bor::BorError> {
        let total_args_len = args.iter().map(|a| tari_bor::encoded_len(&a)).sum::<usize>();
        let total_len = CallHeader::SIZE + total_args_len + args.len() * size_of::<u32>();
        Ok(total_len)
    }

    #[cfg(feature = "std")]
    pub fn encode_v1_packed<W>(
        writer: &mut W,
        func: FunctionIdent,
        args: &[tari_bor::Value],
    ) -> Result<(), tari_bor::BorError>
    where
        W: std::io::Write,
    {
        // Header ([func])
        writer
            .write_all(&func.to_le_bytes())
            .map_err(|e| tari_bor::BorError::new(e.to_string()))?;

        let args_lens = args.iter().map(|a| tari_bor::encoded_len(&a));
        // Args
        for (arg, len) in args.iter().zip(args_lens) {
            let arg_len =
                u32::try_from(len).map_err(|_| tari_bor::BorError::new("Argument length exceeds u32".to_string()))?;
            writer
                .write_all(&arg_len.to_le_bytes())
                .map_err(|e| tari_bor::BorError::new(e.to_string()))?;
            tari_bor::encode_into_writer(arg, writer)?;
        }
        Ok(())
    }

    pub fn v1_packed_reader(data: &[u8]) -> PackedCallInfoReader<'_> {
        PackedCallInfoReader::new(data)
    }
}

pub struct CallHeader {
    pub func: FunctionIdent,
}

impl CallHeader {
    //  funcident (4)
    const SIZE: usize = size_of::<u32>();
}

pub struct PackedCallInfoReader<'a> {
    data: &'a [u8],
    payload_offset: usize,
}

impl<'a> PackedCallInfoReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            payload_offset: CallHeader::SIZE,
        }
    }

    pub fn decode_header(&mut self) -> CallHeader {
        if self.data.len() < CallHeader::SIZE {
            panic!("MALHDR");
        }
        let func = decode_u32_le(&self.data[0..4]);
        CallHeader { func }
    }

    /// Reads the next argument from the packed call info data.
    /// Returns None if there are no more arguments to read
    ///
    /// # Panics
    /// Panics if the argument length exceeds the data bounds (malformed data).
    pub fn next_arg(&mut self) -> Option<&'a [u8]> {
        // Read the length of the next argument
        let len_slice = self.data.get(self.payload_offset..self.payload_offset + 4)?;
        let arg_len = decode_u32_le(len_slice) as usize;
        let start = self.payload_offset + 4;
        let end = start + arg_len;
        self.payload_offset += 4 + arg_len;
        Some(self.data.get(start..end).expect("ARGOVR"))
    }

    /// Reads the next argument from the packed call info data.
    /// This function is called within WASM, to decode engine calls.
    ///
    /// # Panics
    /// Panics if there are no more arguments to read or if the argument length exceeds the data bounds.
    /// Panics are used to fail execution.
    pub fn next_arg_unchecked(&mut self) -> &'a [u8] {
        self.next_arg().expect("ARGOVR")
    }
}

impl<'a> Iterator for PackedCallInfoReader<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.next_arg()
    }
}

/// Decodes a little-endian u32 from the given byte slice.
/// Panics if the slice is less than 4 bytes.
/// It is up to the caller to ensure the slice is of sufficient length.
fn decode_u32_le(bytes: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(bytes);
    u32::from_le_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_encodes_decodes_call_info() {
        let func_id: FunctionIdent = 42;
        let args = vec![
            tari_bor::Value::Integer(100.into()),
            tari_bor::Value::Bytes(b"test".to_vec()),
            tari_bor::Value::Bool(true),
            tari_bor::Value::Text("hello".to_string()),
            tari_bor::Value::Float(2.14),
            tari_bor::Value::Null,
            tari_bor::Value::Array(vec![
                tari_bor::Value::Integer(1.into()),
                tari_bor::Value::Integer(2.into()),
                tari_bor::Value::Integer(3.into()),
            ]),
            tari_bor::Value::Map(vec![(
                tari_bor::Value::Text("key".to_string()),
                tari_bor::Value::Text("value".to_string()),
            )]),
            tari_bor::Value::Tag(123, Box::new(tari_bor::Value::Text("tagged".to_string()))),
        ];

        let size = CallInfo::encode_v1_packed_size(&args).unwrap();
        let mut encoded_data = Vec::with_capacity(size);
        CallInfo::encode_v1_packed(&mut encoded_data, func_id, &args).unwrap();
        assert_eq!(encoded_data.len(), size);
        let mut reader = CallInfo::v1_packed_reader(&encoded_data);
        let header = reader.decode_header();
        assert_eq!(header.func, func_id);
        for (i, arg) in args.iter().enumerate() {
            let arg_bytes = reader.next_arg_unchecked();
            let decoded_arg: tari_bor::Value = tari_bor::decode_exact(arg_bytes).unwrap();
            assert_eq!(&decoded_arg, arg, "Argument {} does not match", i);
        }
    }

    #[test]
    fn it_works_with_zero_args() {
        let func_id: FunctionIdent = 7;
        let args: Vec<tari_bor::Value> = vec![];

        let mut encoded_data = Vec::new();
        CallInfo::encode_v1_packed(&mut encoded_data, func_id, &args).unwrap();
        let mut reader = CallInfo::v1_packed_reader(&encoded_data);
        let header = reader.decode_header();
        assert_eq!(header.func, func_id);
        assert!(reader.next_arg().is_none());
    }

    #[test]
    #[should_panic(expected = "MALHDR")]
    fn it_panics_if_malformed() {
        let malformed_data = vec![0u8; CallHeader::SIZE - 1]; // Too short to contain a full header
        let mut reader = CallInfo::v1_packed_reader(&malformed_data);
        reader.decode_header();
    }

    #[test]
    #[should_panic(expected = "ARGOVR")]
    fn it_panics_if_arg_length_bounds_exceed_data_bound() {
        let func_id: FunctionIdent = 1;
        let args = vec![tari_bor::Value::Text("Hello hello".into())];
        let mut encoded_data = Vec::new();
        CallInfo::encode_v1_packed(&mut encoded_data, func_id, &args).unwrap();
        let mut reader = CallInfo::v1_packed_reader(&encoded_data);
        reader.decode_header();

        // Manually corrupt the payload to simulate malformed data
        let mut corrupted_data = encoded_data.clone();
        corrupted_data.truncate(corrupted_data.len() - 1); // Remove last byte

        let mut corrupted_reader = CallInfo::v1_packed_reader(&corrupted_data);
        corrupted_reader.decode_header();

        // PANIC
        corrupted_reader.next_arg();
    }
}
