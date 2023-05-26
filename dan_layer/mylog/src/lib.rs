#![feature(async_closure)]

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn mylog(_: TokenStream, input: TokenStream) -> TokenStream {
    // Parse the function item
    let ItemFn { attrs, vis, sig, block } = parse_macro_input!(input as ItemFn);
    let fn_name = sig.ident.to_string();
    let expanded;
    if sig.asyncness.is_some() {
        expanded = quote! {
          #(#attrs)*
          #vis #sig {
            use std::time::Instant;
            let mut ret = async move || {
              #block
            };
            println!("[+] {}", #fn_name);
            let instant = Instant::now();
            let res = ret().await;
            println!("[-] {} time {:?} returned {:?}", #fn_name, instant.elapsed(), res);
            res
          }
        };
    } else {
        expanded = quote! {
          #(#attrs)*
          #vis #sig {
            use std::time::Instant;
            let mut ret = move || {
              #block
            };
            println!("[+] {}", #fn_name);
            let instant = Instant::now();
            let res = ret();
            println!("[-] {} time {:?} returned {:?}", #fn_name, instant.elapsed(), res);
            res
          }
        };
    }
    TokenStream::from(expanded)
}
