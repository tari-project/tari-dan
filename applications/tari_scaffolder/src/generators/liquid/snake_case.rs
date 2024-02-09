//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use convert_case::{Case, Casing};
use liquid::{model::Value, ValueView};
use liquid_core::{Display_filter, Filter, FilterReflection, ParseFilter, Runtime};

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "snake_case",
    description = "Converts a string to snake_case",
    parsed(SnakeCaseFilter)
)]
pub struct SnakeCase;

#[derive(Debug, Default, Display_filter)]
#[name = "snake_case"]
struct SnakeCaseFilter;

impl Filter for SnakeCaseFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> liquid_core::Result<Value> {
        let s = input.to_kstr().to_owned();
        let snake_case = s.to_string().to_case(Case::Snake);
        Ok(Value::scalar(snake_case))
    }
}
