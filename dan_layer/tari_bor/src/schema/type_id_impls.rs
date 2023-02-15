//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::schema::type_id::{TypeId, TypeIdRepr};

macro_rules! primitive_type_id_impl {
    ($ty:ty, $ty_variant:ident) => {{
        impl<T> $crate::schema::type_id::TypeId<T> for $ty {
            fn as_type_id() -> $crate::schema::type_id::TypeIdRepr<T> {
                $crate::schema::type_id::TypeIdRepr::$ty_variant
            }
        }
    }};
}

primitive_type_id_impl!((), Unit);
primitive_type_id_impl!(bool, Bool);
primitive_type_id_impl!(i8, I8);
primitive_type_id_impl!(i16, I16);
primitive_type_id_impl!(i32, I32);
primitive_type_id_impl!(i64, I64);
primitive_type_id_impl!(i128, I128);
primitive_type_id_impl!(u8, U8);
primitive_type_id_impl!(u16, U16);
primitive_type_id_impl!(u32, U32);
primitive_type_id_impl!(u64, U64);
primitive_type_id_impl!(u128, U128);
primitive_type_id_impl!($crate::borsh::maybe_std::string::String, String);
