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

use std::fmt::{Debug, Formatter};

use tari_template_abi::{EngineOp, WASM_PTR_SIZE};
use tari_wasmer_middlewares::metering::{MeteringPoints, get_remaining_points, set_remaining_points};
use wasmer::{AsStoreMut, AsStoreRef, Instance, Memory, MemoryAccessError, MemoryView, TypedFunction, WasmPtr};

use crate::{
    runtime::ComputeAllowance,
    wasm::{WasmExecutionError, mem_writer::MemWriter},
};

pub(crate) type WasmAllocFn = TypedFunction<u32, WasmPtr<u8>>;
pub(crate) type WasmFreeFn = TypedFunction<WasmPtr<u8>, ()>;

/// State shared between the host and one WASM instance. It lives in the instance's
/// [`wasmer::FunctionEnv`], which hands out `&mut` to whoever holds the store, so nothing here
/// needs interior mutability: the engine executes one instance at a time on one thread.
///
/// An instance outlives a single call: it serves every call to its template within a transaction.
/// `state` and the per-invocation fields are therefore bound per call by [`Self::bind_call`] and
/// released by [`Self::unbind_call`], so no call runs against what an earlier call left behind.
pub struct WasmEnv<T> {
    memory: Option<Memory>,
    state: Option<T>,
    mem_alloc: Option<WasmAllocFn>,
    mem_free: Option<WasmFreeFn>,
    last_panic: Option<String>,
    last_engine_error: Option<WasmExecutionError>,
    invocation_meter: Option<InvocationMeter>,
    in_template_invocation: bool,
    refused_engine_call: Option<EngineOp>,
}

/// Per-invocation view of the Wasmer meter, letting host calls read the in-flight consumption of
/// the invocation they interrupt. `synced` is the portion already recorded on the transaction's
/// running total, so consumption is recorded exactly once across incremental syncs and the final
/// end-of-invocation accounting.
pub(super) struct InvocationMeter {
    instance: Instance,
    /// What the meter would read had the invocation consumed nothing, so that `start_points` less
    /// the meter's reading is the invocation's own consumption. Lowering the meter lowers this by
    /// the same amount.
    start_points: u64,
    synced: u64,
    binding_allowance: Option<ComputeAllowance>,
}

impl InvocationMeter {
    fn consumed<S: AsStoreMut>(&self, store: &mut S) -> u64 {
        match get_remaining_points(store, &self.instance) {
            MeteringPoints::Remaining(n) => self.start_points.saturating_sub(n),
            MeteringPoints::Exhausted => self.start_points,
        }
    }
}

/// How a metered invocation ended, as [`WasmEnv::end_metered_invocation`] reports it.
pub(super) struct MeteredInvocation {
    /// Points the invocation's own code consumed and has not yet recorded on the transaction's
    /// running total. Nested calls it made are metered and recorded by theirs.
    pub unsynced: u64,
    /// Whether the invocation ran its meter out.
    pub exhausted: bool,
    /// Set when the authorized compute, not a hard cap, last bounded the meter.
    pub binding_allowance: Option<ComputeAllowance>,
}

impl<T> WasmEnv<T> {
    pub fn new() -> Self {
        Self {
            memory: None,
            state: None,
            mem_alloc: None,
            mem_free: None,
            last_panic: None,
            last_engine_error: None,
            invocation_meter: None,
            in_template_invocation: false,
            refused_engine_call: None,
        }
    }

    /// Binds the state one call runs against and clears everything an earlier call on this instance
    /// may have left in the per-invocation fields.
    pub(super) fn bind_call(&mut self, state: T) {
        self.state = Some(state);
        self.last_panic = None;
        self.last_engine_error = None;
        self.invocation_meter = None;
        self.in_template_invocation = false;
        self.refused_engine_call = None;
    }

    /// Releases the state bound by [`Self::bind_call`]. An idle instance holds none, so a cached
    /// instance keeps no reference to the transaction state that owns the cache.
    pub(super) fn unbind_call(&mut self) -> Option<T> {
        self.state.take()
    }

    /// Marks template code as running inside a function invocation, which is the only context
    /// permitted to call the engine.
    ///
    /// This is deliberately its own state rather than a reading of [`Self::invocation_meter`]. The
    /// metering window is a billing concern and may legitimately be widened — for instance to
    /// charge the `tari_alloc`/`tari_free` the engine drives around a call — whereas this window
    /// must stay closed around the template function itself. Deriving one from the other would let
    /// such a change silently re-admit engine calls from `tari_alloc`/`tari_free`.
    ///
    /// One invocation is in flight per process at a time; cross-template calls run in their own
    /// process with their own [`WasmEnv`].
    pub(super) fn enter_template_invocation(&mut self) {
        self.in_template_invocation = true;
    }

    /// Marks the template function invocation as finished. Template code the engine drives after
    /// this point — `tari_free` on the returned pointer — may no longer call the engine.
    pub(super) fn exit_template_invocation(&mut self) {
        self.in_template_invocation = false;
    }

    /// Closes the window for a call into template code the engine drives from inside an invocation,
    /// returning its previous state. Pair with [`Self::restore_template_invocation`] so the window
    /// is put back as it was rather than opened.
    pub(super) fn suspend_template_invocation(&mut self) -> bool {
        let was_open = self.in_template_invocation;
        self.in_template_invocation = false;
        was_open
    }

    pub(super) fn restore_template_invocation(&mut self, was_open: bool) {
        self.in_template_invocation = was_open;
    }

    pub(super) fn is_in_template_invocation(&self) -> bool {
        self.in_template_invocation
    }

    /// Records that an engine call was refused for being made outside a template function
    /// invocation. Kept apart from [`Self::last_engine_error`], which carries failures of calls
    /// that were dispatched: a refusal must fail the transaction even when the template ignores
    /// the null pointer it is handed, so the host reads it back unambiguously.
    pub(super) fn set_refused_engine_call(&mut self, op: EngineOp) {
        self.refused_engine_call = Some(op);
    }

    pub(super) fn take_refused_engine_call(&mut self) -> Option<EngineOp> {
        self.refused_engine_call.take()
    }

    /// Begins metering an invocation that starts with `start_points` on the Wasmer meter, bounded by
    /// `binding_allowance` when the authorized compute rather than a hard cap sized it. One
    /// invocation is in flight per process instance at a time (cross-template calls run in their
    /// own process, with their own meter).
    pub(super) fn begin_metered_invocation(
        &mut self,
        instance: Instance,
        start_points: u64,
        binding_allowance: Option<ComputeAllowance>,
    ) {
        self.invocation_meter = Some(InvocationMeter {
            instance,
            start_points,
            synced: 0,
            binding_allowance,
        });
    }

    /// Ends the in-flight invocation and reports how it ended, including the tail of its
    /// consumption not already synced to the transaction total.
    pub(super) fn end_metered_invocation<S: AsStoreMut>(&mut self, store: &mut S) -> MeteredInvocation {
        let Some(meter) = self.invocation_meter.take() else {
            return MeteredInvocation {
                unsynced: 0,
                exhausted: false,
                binding_allowance: None,
            };
        };
        MeteredInvocation {
            unsynced: meter.consumed(store).saturating_sub(meter.synced),
            exhausted: matches!(get_remaining_points(store, &meter.instance), MeteringPoints::Exhausted),
            binding_allowance: meter.binding_allowance,
        }
    }

    /// Reads the in-flight invocation's consumed-but-unsynced points from the Wasmer meter and
    /// marks them synced. Returns `None` when no invocation is in flight (host calls made outside
    /// a WASM invocation) or nothing new was consumed.
    pub(super) fn take_unsynced_in_flight_points<S: AsStoreMut>(&mut self, store: &mut S) -> Option<u64> {
        let meter = self.invocation_meter.as_mut()?;
        let consumed = meter.consumed(store);
        let delta = consumed.saturating_sub(meter.synced);
        if delta == 0 {
            return None;
        }
        meter.synced = consumed;
        Some(delta)
    }

    /// Lowers the in-flight invocation's meter to at most `limit` remaining points, `limit` being
    /// what the transaction may still spend and `binding_allowance` what bounds it. The points taken
    /// off the meter were never run, so they come off `start_points` too and the invocation's
    /// consumption is unchanged. A meter already at or below `limit` is left as it is.
    pub(super) fn clamp_in_flight_meter<S: AsStoreMut>(
        &mut self,
        store: &mut S,
        limit: u64,
        binding_allowance: Option<ComputeAllowance>,
    ) {
        let Some(meter) = self.invocation_meter.as_mut() else {
            return;
        };
        let MeteringPoints::Remaining(remaining) = get_remaining_points(store, &meter.instance) else {
            return;
        };
        if remaining <= limit {
            return;
        }
        set_remaining_points(store, &meter.instance, limit);
        meter.start_points = meter.start_points.saturating_sub(remaining - limit);
        meter.binding_allowance = binding_allowance;
    }

    pub(super) fn set_last_panic(&mut self, message: String) {
        self.last_panic = Some(message);
    }

    pub(super) fn has_last_panic(&self) -> bool {
        self.last_panic.is_some()
    }

    /// Hands out the template's `tari_alloc` as an owned handle, for callers that must let go of
    /// their borrow of this environment before calling it. `tari_alloc` is template code, and
    /// template code can call `tari_engine`, which takes its own `&mut` to this environment.
    pub(super) fn mem_alloc_func(&self) -> Result<WasmAllocFn, WasmExecutionError> {
        self.mem_alloc
            .clone()
            .ok_or(WasmExecutionError::MissingAbiFunction { function: "tari_alloc" })
    }

    /// Hands out the template's `tari_free` as an owned handle, under the same borrowing rule as
    /// [`Self::mem_alloc_func`].
    pub(super) fn mem_free_func(&self) -> Result<WasmFreeFn, WasmExecutionError> {
        self.mem_free
            .clone()
            .ok_or(WasmExecutionError::MissingAbiFunction { function: "tari_free" })
    }

    pub(super) fn take_last_panic_message(&mut self) -> Option<String> {
        self.last_panic.take()
    }

    /// Records why an engine call could not be answered. `tari_engine_entrypoint` can only reply to
    /// the WASM with a null pointer, which carries no reason and which a template is free to
    /// ignore, so the reason is left here for `WasmProcess::invoke` to fail the call with.
    ///
    /// Every path that answers null must record one, or the failure reaches the transaction as
    /// whatever the template panicked with on the null instead.
    pub(super) fn set_last_engine_error<E: Into<WasmExecutionError>>(&mut self, error: E) {
        self.last_engine_error = Some(error.into());
    }

    pub(super) fn has_last_engine_error(&self) -> bool {
        self.last_engine_error.is_some()
    }

    pub(super) fn take_last_engine_error(&mut self) -> Option<WasmExecutionError> {
        self.last_engine_error.take()
    }

    pub(super) fn memory_writer<'a, S: AsStoreMut>(
        &self,
        store: &'a mut S,
        ptr: WasmPtr<u8>,
    ) -> Result<MemWriter<'a>, WasmExecutionError> {
        let view = self.get_memory()?.view(store);
        Ok(MemWriter::new(ptr, view))
    }

    /// Retrieves a slice of memory at the given pointer and length, and calls the provided callback with that slice.
    /// Returns an error if the pointer and length are out of memory bounds.
    ///
    /// # Safety
    /// This function provides direct access to the memory slice. The caller must ensure that the memory is not
    /// modified while the slice is in use.
    /// It is undefined behaviour to modify the memory contents in any way including by calling a wasm
    /// function that writes to the memory or by resizing the memory.
    pub(super) unsafe fn with_memory_slice<S: AsStoreRef, F: FnMut(&[u8]) -> R, R>(
        &self,
        store: &mut S,
        ptr: WasmPtr<u8>,
        len: u32,
        mut callback: F,
    ) -> Result<R, WasmExecutionError> {
        let memory = self.get_memory()?;
        let view = memory.view(store);

        let slice = unsafe { view.data_unchecked() };

        let start = ptr.offset() as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or(WasmExecutionError::MemoryPointerOutOfRange {
                size: slice.len() as u64,
                pointer: ptr.offset(),
                len,
            })?;

        let slice = slice
            .get(start..end)
            .ok_or(WasmExecutionError::MemoryPointerOutOfRange {
                size: slice.len() as u64,
                pointer: ptr.offset(),
                len,
            })?;

        Ok(callback(slice))
    }

    /// Reads the 4-byte length prefix at the given offset and calls the provided callback with the payload slice
    /// (`offset + 4..offset + 4 + len`) i.e. excluding the length prefix. Returns an error if the length prefix or
    /// payload is out of memory bounds.
    ///
    /// # Safety
    /// This function provides direct access to the memory slice. The caller must ensure that the memory is not
    /// modified while the slice is in use.
    /// It is undefined behaviour to modify the memory contents in any way including by calling a wasm
    /// function that writes to the memory or by resizing the memory.
    pub(super) unsafe fn with_memory_embedded_len<S: AsStoreRef, F: FnMut(&[u8]) -> R, R>(
        &self,
        store: &mut S,
        offset: u32,
        mut callback: F,
    ) -> Result<R, WasmExecutionError> {
        let memory = self.get_memory()?;
        let view = memory.view(store);
        let len_prefix_ptr =
            offset
                .checked_sub(WASM_PTR_SIZE as u32)
                .ok_or(WasmExecutionError::MemoryPointerOutOfRange {
                    size: view.data_size(),
                    pointer: offset,
                    len: WASM_PTR_SIZE as u32,
                })?;
        // alloc_len = size_of<u32> + payload_len
        let alloc_len = read_len_from_memory(&view, len_prefix_ptr)?;

        let start = offset;
        let end = len_prefix_ptr
            .checked_add(alloc_len)
            .ok_or(WasmExecutionError::MemoryPointerOutOfRange {
                size: view.data_size(),
                pointer: start,
                len: alloc_len,
            })?;

        let slice = unsafe { view.data_unchecked() };
        let slice = slice
            .get(start as usize..end as usize)
            .ok_or(WasmExecutionError::MemoryPointerOutOfRange {
                size: slice.len() as u64,
                pointer: start,
                len: alloc_len,
            })?;

        Ok(callback(slice))
    }

    /// The state bound for the call in flight.
    pub fn state(&self) -> Result<&T, WasmExecutionError> {
        self.state.as_ref().ok_or(WasmExecutionError::NoCallBound)
    }

    fn get_memory(&self) -> Result<&Memory, WasmExecutionError> {
        let memory = self.memory.as_ref().ok_or_else(|| WasmExecutionError::MemoryNotSet)?;
        Ok(memory)
    }
}

impl<T> Default for WasmEnv<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> WasmEnv<T> {
    pub fn set_memory(&mut self, memory: Memory) -> &mut Self {
        self.memory = Some(memory);
        self
    }

    pub fn set_alloc_funcs(&mut self, mem_alloc: WasmAllocFn, mem_free: WasmFreeFn) -> &mut Self {
        self.mem_alloc = Some(mem_alloc);
        self.mem_free = Some(mem_free);
        self
    }
}

impl<T: Debug> Debug for WasmEnv<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmEnv")
            .field("memory", &"LazyInit<Memory>")
            .field("tari_alloc", &" LazyInit<NativeFunc<(i32), (i32)>")
            .field("tari_free", &" LazyInit<NativeFunc<(i32), ()>")
            .field("state", &self.state)
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

fn read_len_from_memory(view: &MemoryView, offset: u32) -> Result<u32, MemoryAccessError> {
    let mut buf = [0u8; 4];
    view.read(u64::from(offset), &mut buf)?;
    Ok(u32::from_le_bytes(buf))
}
