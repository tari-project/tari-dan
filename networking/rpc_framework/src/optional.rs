//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub trait OrOptional<T> {
    type Error;
    fn or_optional(self) -> Result<Option<T>, Self::Error>;
}
