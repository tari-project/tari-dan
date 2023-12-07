//  Copyright 2023. The Tari Project
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

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// The name of the global export for the function that returns the template lib version
pub const TEMPLATE_LIB_VERSION_FUNCTION_GLOBAL_NAME: &str = "_GET_TEMPLATE_LIB_VERSION";

pub fn generate_template_lib_version() -> TokenStream {
    let version_function_name = format_ident!("{TEMPLATE_LIB_VERSION_FUNCTION_GLOBAL_NAME}");

    quote! {
        #[no_mangle]
        pub unsafe extern "C" fn #version_function_name() -> *mut u8 {
            use ::tari_template_lib::VERSION;
            use ::tari_template_lib::template_dependencies::{wrap_ptr, encode_with_len};
            let result = encode_with_len(&VERSION);
            wrap_ptr(result)
        }
    }
}
