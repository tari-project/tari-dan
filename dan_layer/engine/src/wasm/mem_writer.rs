//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use wasmer::{MemoryAccessError, MemoryView, WasmPtr};

pub struct MemWriter<'a> {
    ptr: WasmPtr<u8>,
    view: MemoryView<'a>,
}
impl<'a> MemWriter<'a> {
    pub fn new(ptr: WasmPtr<u8>, view: MemoryView<'a>) -> Self {
        Self { ptr, view }
    }
}

impl tari_bor::Write for &mut MemWriter<'_> {
    type Error = MemoryAccessError;

    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.view.write(u64::from(self.ptr.offset()), data)?;
        self.ptr = self.ptr.add_offset(data.len() as u32)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
