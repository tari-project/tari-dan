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

use std::collections::HashMap;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Block, Expr, ExprBlock, ExprField, Result, Stmt, TypePath, TypeTuple, parse_quote, token::Brace};
use tari_template_abi::{FunctionIdent, diagnostics::PanicDiagnostic, func_hasher::hash_function_name};

use crate::template::ast::{FunctionAst, TemplateAst, TypeAst};

pub fn generate_dispatcher(ast: &TemplateAst) -> Result<TokenStream> {
    let dispatcher_function_name = format_ident!("{}_main", ast.template_name);
    let function_idents = get_function_idents(ast)?;
    let function_blocks = get_function_blocks(ast);
    let uses = &ast.uses;

    let output = quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #dispatcher_function_name(call_info_ptr: *mut u8, call_info_len: u32) -> *mut u8 {
            use ::tari_template_lib::template_macro_deps::*;
            // include all use statements from the template module here as these may be used in the function arguments.
            #(
                #[allow(unused_imports)]
                #uses
            )*

            #[cfg(not(target_arch = "wasm32"))]
            compile_error!("Must compile template with --target wasm32-unknown-unknown");

            // Custom panic hook that invokes the on_panic host function
            register_panic_hook();

            if call_info_ptr.is_null() {
                panic!("CALLINFO_NULL");
            }

            let owned = unsafe { OwnedData::owned_from_ptr(call_info_ptr) };
            let mut call_info = CallInfo::v1_packed_reader(owned.data());
            let call_header = call_info.decode_header();

            let result_ptr;
            match call_header.func {
                #( #function_idents => #function_blocks ),*,
                _ => panic!("UNKWN_FN {}", call_header.func),
            };

            result_ptr
        }
    };

    Ok(output)
}

fn get_function_idents(ast: &TemplateAst) -> Result<impl Iterator<Item = FunctionIdent> + '_> {
    let mut collisions = HashMap::with_capacity(ast.functions.len());
    // Collect the idents to preserve order
    let mut idents = Vec::with_capacity(ast.functions.len());
    for f in ast.get_functions() {
        let ident = hash_function_name(&f.name);
        idents.push(ident);
        if let Some(other) = collisions.insert(ident, &f.name) {
            // NOTE: if a user has the maximum permitted functions (8192) the chance of collision is ~0.778% (<8 in
            // 1000) (birthday problem: p(k) = 1 - exp(-k(k-1)/2N) where k is the number of hashes).
            // Moreover, This check is not strictly necessary as the dispatcher will fail to compile due to duplicate
            // match arms. This is to provide a clearer error message to the user.
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "Function name hash collision detected between '{}' and '{}' (hash = {}). Please rename one of \
                     these functions to avoid the collision.",
                    other, f.name, ident
                ),
            ));
        }
    }
    Ok(idents.into_iter())
}

fn get_function_blocks(ast: &TemplateAst) -> impl Iterator<Item = Expr> + '_ {
    ast.get_functions()
        .map(|function| get_function_block(&ast.template_name, function, ast.stateless))
}

#[allow(clippy::too_many_lines)]
fn get_function_block(template_ident: &Ident, ast: &FunctionAst, stateless: bool) -> Expr {
    let template_mod_name = format_ident!("{}_template", template_ident);
    let mut args: Vec<Expr> = vec![];
    let mut stmts = vec![];
    // The engine renders these markers into their full text from the template definition, so the
    // published binary carries neither the function name nor the argument type. See
    // `tari_template_abi::diagnostics`.
    let error_failed_decode_component = PanicDiagnostic::ComponentDecode.panic_format_string();

    let mut is_mutable_call = false;

    // encode all arguments of the functions
    for (i, input_type) in ast.input_types.iter().enumerate() {
        let arg_ident = format_ident!("arg_{}", i);

        match input_type {
            // "self" argument
            TypeAst::Receiver { mutability } => {
                assert!(
                    !ast.is_migration,
                    "migration functions cannot have &self or &mut self arguments"
                );
                is_mutable_call = *mutability;
                if is_mutable_call {
                    args.push(parse_quote! { &mut state });
                } else {
                    args.push(parse_quote! { &state });
                }
                stmts.extend([
                    parse_quote! { let next_arg = call_info.next_arg_unchecked(); },
                    parse_quote! {
                        let component_address = decode_exact::<::tari_template_lib::types::ComponentAddress>(next_arg)
                            .unwrap_or_else(|e| panic!(#error_failed_decode_component, e));
                    },
                    parse_quote! {
                        let component_manager = engine().component_manager(component_address);
                    },
                    parse_quote! {
                        let mut state = component_manager.get_state::<#template_mod_name::#template_ident>();
                    },
                ]);
            },
            // non-self argument
            TypeAst::Typed { type_path, .. } => {
                let error_failed_decode_arg = PanicDiagnostic::ArgDecode { index: i }.panic_format_string();
                if i == 0 && ast.is_migration {
                    stmts.extend([
                        parse_quote! { let next_arg = call_info.next_arg_unchecked(); },
                        parse_quote! {
                            let component_address = decode_exact::<::tari_template_lib::types::ComponentAddress>(next_arg)
                                .unwrap_or_else(|e| panic!(#error_failed_decode_component, e));
                        },
                        parse_quote! {
                            let component_manager = engine().component_manager(component_address);
                        },
                        parse_quote! {
                            let old_state = component_manager.get_state::<#type_path>();
                        },
                    ]);
                    args.push(parse_quote! { old_state });
                } else {
                    args.push(parse_quote! { #arg_ident });
                    stmts.extend([
                        parse_quote! { let next_arg = call_info.next_arg_unchecked(); },
                        parse_quote! {
                            let #arg_ident = decode_exact::<#type_path>(next_arg)
                                .unwrap_or_else(|e| panic!(#error_failed_decode_arg, e));
                        },
                    ]);
                }
            },
            TypeAst::Tuple { type_tuple, .. } => {
                let error_failed_decode_arg = PanicDiagnostic::ArgDecode { index: i }.panic_format_string();
                args.push(parse_quote! { #arg_ident });
                stmts.extend([
                    parse_quote! { let next_arg = call_info.next_arg_unchecked(); },
                    parse_quote! {
                        let #arg_ident = decode_exact::<#type_tuple>(next_arg)
                            .unwrap_or_else(|e| panic!(#error_failed_decode_arg, e));
                    },
                ]);
            },
        }
    }

    // call the user defined function in the template
    let function_ident = Ident::new(&ast.name, Span::call_site());
    stmts.push(if stateless {
        // Stateless templates expose free functions directly in the generated module, so there is
        // no component struct to qualify the call with.
        parse_quote! {
            let rtn = #template_mod_name::#function_ident(#(#args),*);
        }
    } else {
        parse_quote! {
            let rtn = #template_mod_name::#template_ident::#function_ident(#(#args),*);
        }
    });

    if ast.is_migration {
        stmts.extend([
            // Empty result
            parse_quote! { result_ptr = alloc_and_encode(&()); },
            // Set the state to the return value
            parse_quote! { component_manager.set_state(rtn); },
        ]);
    } else {
        // Handle "Self" (if present) in the return position by creating a new component
        stmts.extend(replace_self_in_output(ast));

        // encode the result value
        stmts.push(parse_quote! {
            result_ptr = alloc_and_encode(&rtn);
        });

        // after user function invocation, update the component state
        if is_mutable_call {
            stmts.push(parse_quote! { component_manager.set_state(state); });
        }
    }

    // construct the code block for the function
    Expr::Block(ExprBlock {
        attrs: vec![],
        label: None,
        block: Block {
            brace_token: Brace::default(),
            stmts,
        },
    })
}

fn replace_self_in_output(ast: &FunctionAst) -> Vec<Stmt> {
    if let Some(output_type) = &ast.output_type {
        match output_type {
            TypeAst::Typed { type_path, .. } => {
                if let Some(stmt) = replace_self_in_single_value(type_path) {
                    return vec![stmt];
                }
            },
            TypeAst::Tuple { type_tuple, .. } => {
                let stmt = replace_self_in_tuple(type_tuple);
                return vec![stmt];
            },
            _ => todo!("replace_self_in_output only supports typed and tuple"),
        }
    }

    vec![]
}

fn replace_self_in_single_value(type_path: &TypePath) -> Option<Stmt> {
    let type_ident = &type_path.path.segments.first()?.ident;

    if type_ident == "Self" {
        // When we return self we use default rules - which only permit the owner of the component to call methods
        return Some(parse_quote! {
            let rtn = engine().create_component(
                rtn,
                ::tari_template_lib::template_macro_deps::OwnerRule::default(),
                ::tari_template_lib::template_macro_deps::ComponentAccessRules::new(),
                None,
            );
        });
    }

    None
}

fn replace_self_in_tuple(type_tuple: &TypeTuple) -> Stmt {
    // build the expressions for each element in the tuple
    let elems: Vec<Expr> = type_tuple
        .elems
        .iter()
        .enumerate()
        .map(|(i, t)| match t {
            syn::Type::Path(path) => {
                let ident = path
                    .path
                    .segments
                    .first()
                    .expect("path segments is empty")
                    .ident
                    .clone();
                let field_expr = build_tuple_field_expr("rtn".to_string(), i as u32);
                if ident == "Self" {
                    // When we return self we use default rules - which only permit the owner of the component to call
                    // methods
                    parse_quote! {
                        engine().create_component(
                            #field_expr,
                            ::tari_template_lib::template_macro_deps::OwnerRule::default(),
                            ::tari_template_lib::template_macro_deps::ComponentAccessRules::new(),
                            None,
                        )
                    }
                } else {
                    field_expr
                }
            },
            _ => todo!("replace_self_in_tuple only supports paths"),
        })
        .collect();

    parse_quote! {
        let rtn = (#(#elems),*);
    }
}

fn build_tuple_field_expr(name: String, i: u32) -> Expr {
    let name = Ident::new(&name, Span::call_site());

    let mut field_expr: ExprField = parse_quote! {
        #name.0
    };

    match field_expr.member {
        syn::Member::Unnamed(ref mut unnamed) => {
            unnamed.index = i;
        },
        _ => todo!("build_tuple_field_expr only supports Unnamed"),
    }

    Expr::Field(field_expr)
}
