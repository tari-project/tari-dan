//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ptr::NonNull;

use wasmer::{
    MemoryError,
    MemoryStyle,
    MemoryType,
    Pages,
    TableStyle,
    TableType,
    sys::{
        Tunables,
        vm::{VMMemory, VMMemoryDefinition, VMTable, VMTableDefinition},
    },
};

/// A custom tunables that bounds the host allocations a guest module can ask for: its linear
/// memory and each of its tables.
///
/// After adjusting those limits, it delegates all other logic to the base tunables.
pub struct LimitingTunables<T: Tunables> {
    /// The maximum a linear memory is allowed to be (in Wasm pages, 64 KiB each).
    /// Since Wasmer ensures there is only none or one memory, this is practically
    /// an upper limit for the guest memory.
    limit: Pages,
    /// The maximum number of elements a table is allowed to hold.
    table_limit: u32,
    /// The base implementation we delegate all the logic to
    base: T,
}

impl<T: Tunables> LimitingTunables<T> {
    pub fn new(base: T, limit: Pages, table_limit: u32) -> Self {
        Self {
            limit,
            table_limit,
            base,
        }
    }

    /// Takes an input memory type as requested by the guest and sets
    /// a maximum if missing. The resulting memory type is final if
    /// valid. However, this can produce invalid types, such that
    /// validate_memory must be called before creating the memory.
    fn adjust_memory(&self, requested: &MemoryType) -> MemoryType {
        let mut adjusted = *requested;
        if requested.maximum.is_none() {
            adjusted.maximum = Some(self.limit);
        }
        adjusted
    }

    /// Ensures a given memory type does not exceed the memory limit.
    /// Call this after adjusting the memory.
    fn validate_memory(&self, ty: &MemoryType) -> Result<(), MemoryError> {
        if ty.minimum > self.limit {
            return Err(MemoryError::Generic(format!(
                "Minimum {} exceeds the allowed memory limit {}",
                ty.minimum.0, self.limit.0
            )));
        }

        if let Some(max) = ty.maximum {
            if max > self.limit {
                return Err(MemoryError::Generic(format!(
                    "Maximum {} exceeds the allowed memory limit {}",
                    max.0, self.limit.0
                )));
            }
        } else {
            return Err(MemoryError::Generic("Maximum not set".to_string()));
        }

        Ok(())
    }

    /// Takes a table type as requested by the guest and sets a maximum if missing. A table without
    /// a declared maximum is grown by `table.grow` up to `u32::MAX` entries, and the runtime bounds
    /// that only by the maximum, so every table must carry one.
    ///
    /// The result can still exceed the limit, so `validate_table` must be called on it before the
    /// table is created.
    fn adjust_table(&self, requested: &TableType) -> TableType {
        let mut adjusted = *requested;
        if requested.maximum.is_none() {
            // A module may declare a minimum above the limit; leaving the maximum below it would be
            // an invalid type. `validate_table` rejects that module.
            adjusted.maximum = Some(self.table_limit.max(requested.minimum));
        }
        adjusted
    }

    /// Ensures a given table type does not exceed the element limit.
    /// Call this after adjusting the table.
    fn validate_table(&self, ty: &TableType) -> Result<(), String> {
        if ty.minimum > self.table_limit {
            return Err(format!(
                "Minimum {} exceeds the allowed table element limit {}",
                ty.minimum, self.table_limit
            ));
        }

        match ty.maximum {
            Some(max) if max > self.table_limit => Err(format!(
                "Maximum {} exceeds the allowed table element limit {}",
                max, self.table_limit
            )),
            Some(_) => Ok(()),
            None => Err("Maximum not set".to_string()),
        }
    }
}

impl<T: Tunables> Tunables for LimitingTunables<T> {
    /// Construct a `MemoryStyle` for the provided `MemoryType`
    ///
    /// Delegated to base.
    fn memory_style(&self, memory: &MemoryType) -> MemoryStyle {
        let adjusted = self.adjust_memory(memory);
        self.base.memory_style(&adjusted)
    }

    /// Construct a `TableStyle` for the provided `TableType`
    ///
    /// Delegated to base.
    fn table_style(&self, table: &TableType) -> TableStyle {
        let adjusted = self.adjust_table(table);
        self.base.table_style(&adjusted)
    }

    /// Create a memory owned by the host given a [`MemoryType`] and a [`MemoryStyle`].
    ///
    /// The requested memory type is validated, adjusted to the limited and then passed to base.
    fn create_host_memory(&self, ty: &MemoryType, style: &MemoryStyle) -> Result<VMMemory, MemoryError> {
        let adjusted = self.adjust_memory(ty);
        self.validate_memory(&adjusted)?;
        self.base.create_host_memory(&adjusted, style)
    }

    /// Create a memory owned by the VM given a [`MemoryType`] and a [`MemoryStyle`].
    ///
    /// Delegated to base.
    unsafe fn create_vm_memory(
        &self,
        ty: &MemoryType,
        style: &MemoryStyle,
        vm_definition_location: NonNull<VMMemoryDefinition>,
    ) -> Result<VMMemory, MemoryError> {
        let adjusted = self.adjust_memory(ty);
        self.validate_memory(&adjusted)?;
        unsafe { self.base.create_vm_memory(&adjusted, style, vm_definition_location) }
    }

    /// Create a table owned by the host given a [`TableType`] and a [`TableStyle`].
    ///
    /// The requested table type is adjusted to carry a maximum, validated against the limit and
    /// then passed to base.
    fn create_host_table(&self, ty: &TableType, style: &TableStyle) -> Result<VMTable, String> {
        let adjusted = self.adjust_table(ty);
        self.validate_table(&adjusted)?;
        self.base.create_host_table(&adjusted, style)
    }

    /// Create a table owned by the VM given a [`TableType`] and a [`TableStyle`].
    ///
    /// The requested table type is adjusted to carry a maximum, validated against the limit and
    /// then passed to base.
    unsafe fn create_vm_table(
        &self,
        ty: &TableType,
        style: &TableStyle,
        vm_definition_location: NonNull<VMTableDefinition>,
    ) -> Result<VMTable, String> {
        let adjusted = self.adjust_table(ty);
        self.validate_table(&adjusted)?;
        unsafe { self.base.create_vm_table(&adjusted, style, vm_definition_location) }
    }
}
