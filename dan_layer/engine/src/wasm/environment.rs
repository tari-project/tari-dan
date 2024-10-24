//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    fmt::{Debug, Formatter},
    ops::Range,
    sync::{Arc, Mutex},
};

use tari_template_abi::{TemplateDef, ABI_TEMPLATE_DEF_GLOBAL_NAME};
use wasmer::{
    AsStoreMut,
    AsStoreRef,
    ExportError,
    Instance,
    Memory,
    MemoryAccessError,
    MemoryView,
    TypedFunction,
    WasmPtr,
};

use crate::{
    runtime::RuntimeError,
    wasm::{mem_writer::MemWriter, WasmExecutionError},
};

#[derive(Clone)]
pub struct WasmEnv<T> {
    memory: Option<Memory>,
    state: T,
    mem_alloc: Option<TypedFunction<u32, WasmPtr<u8>>>,
    last_panic: Arc<Mutex<Option<String>>>,
    last_engine_error: Arc<Mutex<Option<RuntimeError>>>,
}

impl<T: Send + 'static> WasmEnv<T> {
    pub fn new(state: T) -> Self {
        Self {
            memory: None,
            state,
            mem_alloc: None,
            last_panic: Arc::new(Mutex::new(None)),
            last_engine_error: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn set_last_panic(&self, message: String) {
        *self.last_panic.lock().unwrap() = Some(message);
    }

    pub(super) fn alloc<S: AsStoreMut>(&self, store: &mut S, len: u32) -> Result<WasmPtr<u8>, WasmExecutionError> {
        let ptr = self.get_mem_alloc_func()?.call(store, len)?;
        if ptr.offset() == 0 {
            return Err(WasmExecutionError::MemoryAllocationFailed);
        }

        Ok(ptr)
    }

    pub(super) fn take_last_panic_message(&self) -> Option<String> {
        self.last_panic.lock().unwrap().take()
    }

    pub(super) fn set_last_engine_error(&self, error: RuntimeError) {
        *self.last_engine_error.lock().unwrap() = Some(error);
    }

    pub(super) fn take_last_engine_error(&self) -> Option<RuntimeError> {
        self.last_engine_error.lock().unwrap().take()
    }

    pub(super) fn load_abi<S: AsStoreMut>(
        &self,
        store: &mut S,
        instance: &Instance,
    ) -> Result<TemplateDef, WasmExecutionError> {
        let ptr = instance
            .exports
            .get_global(ABI_TEMPLATE_DEF_GLOBAL_NAME)?
            .get(store)
            .i32()
            .ok_or(WasmExecutionError::ExportError(ExportError::IncompatibleType))? as u32;

        // Load ABI from memory
        let data = self.read_memory_with_embedded_len(store, ptr)?;
        let decoded = tari_bor::decode(&data).map_err(WasmExecutionError::AbiDecodeError)?;
        Ok(decoded)
    }

    pub(super) fn memory_writer<'a, S: AsStoreMut>(
        &self,
        store: &'a mut S,
        ptr: WasmPtr<u8>,
    ) -> Result<MemWriter<'a>, WasmExecutionError> {
        let view = self.get_memory()?.view(store);
        Ok(MemWriter::new(ptr, view))
    }

    pub(super) fn read_memory_with_embedded_len<S: AsStoreRef>(
        &self,
        store: &mut S,
        offset: u32,
    ) -> Result<Vec<u8>, WasmExecutionError> {
        let memory = self.get_memory()?;
        let view = memory.view(store);
        let mut buf = [0u8; 4];
        view.read(u64::from(offset), &mut buf)?;

        let len = u32::from_le_bytes(buf);
        let start = offset + 4;
        let data = copy_range_to_vec(&view, start..start + len)?;

        Ok(data)
    }

    pub(super) fn read_from_memory<S: AsStoreRef>(
        &self,
        store: &mut S,
        ptr: WasmPtr<u8>,
        len: u32,
    ) -> Result<Vec<u8>, WasmExecutionError> {
        let memory = self.get_memory()?;
        let view = memory.view(store);
        let mem_size = view.data_size();
        let ptr_plus_len = ptr
            .offset()
            .checked_add(len)
            .ok_or(WasmExecutionError::MaxMemorySizeExceeded)?;
        if u64::from(ptr.offset()) >= mem_size || u64::from(ptr_plus_len) >= mem_size {
            return Err(WasmExecutionError::MemoryPointerOutOfRange {
                size: mem_size,
                pointer: u64::from(ptr.offset()),
                len: u64::from(len),
            });
        }
        let data = copy_range_to_vec(&view, ptr.offset()..ptr_plus_len)?;
        Ok(data)
    }

    pub fn state(&self) -> &T {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut T {
        &mut self.state
    }

    fn get_mem_alloc_func(&self) -> Result<&TypedFunction<u32, WasmPtr<u8>>, WasmExecutionError> {
        self.mem_alloc
            .as_ref()
            .ok_or_else(|| WasmExecutionError::MissingAbiFunction { function: "tari_alloc" })
    }

    fn get_memory(&self) -> Result<&Memory, WasmExecutionError> {
        let memory = self.memory.as_ref().ok_or_else(|| WasmExecutionError::MemoryNotSet)?;
        Ok(memory)
    }
}

impl<T: Clone + Sync + Send> WasmEnv<T> {
    pub fn set_memory(&mut self, memory: Memory) -> &mut Self {
        self.memory = Some(memory);
        self
    }

    pub fn set_alloc_funcs(&mut self, mem_alloc: TypedFunction<u32, WasmPtr<u8>>) -> &mut Self {
        self.mem_alloc = Some(mem_alloc);
        self
    }
}

impl<T: Debug> Debug for WasmEnv<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmEnv")
            .field("memory", &"LazyInit<Memory>")
            .field("tari_alloc", &" LazyInit<NativeFunc<(i32), (i32)>")
            .field("State", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub struct AllocPtr(u32, u32);

impl AllocPtr {
    pub fn new(offset: u32, len: u32) -> Self {
        Self(offset, len)
    }

    pub fn get(&self) -> u32 {
        self.0
    }

    pub fn len(&self) -> u32 {
        self.1
    }

    pub fn as_wasm_ptr<T>(&self) -> WasmPtr<T> {
        WasmPtr::new(self.get())
    }
}

/// Copies a range of the memory and returns it as a vector of bytes
/// This is a u32 version of MemoryView::copy_range_to_vec
fn copy_range_to_vec(view: &MemoryView, range: Range<u32>) -> Result<Vec<u8>, MemoryAccessError> {
    let mut new_memory = Vec::new();
    let mut offset = range.start;
    let size = u32::try_from(view.data_size()).map_err(|_| MemoryAccessError::Overflow)?;
    let end = range.end.min(size);
    let mut chunk = [0u8; 40960];
    while offset < end {
        let remaining = end - offset;
        let sublen = remaining.min(chunk.len() as u32) as usize;
        view.read(u64::from(offset), &mut chunk[..sublen])?;
        new_memory.extend_from_slice(&chunk[..sublen]);
        offset += sublen as u32;
    }
    Ok(new_memory)
}
