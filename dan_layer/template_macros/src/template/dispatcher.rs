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

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::{parse_quote, token::Brace, Block, Expr, ExprBlock, ExprField, Result, Stmt, TypePath, TypeTuple};

use crate::template::ast::{FunctionAst, TemplateAst, TypeAst};

pub fn generate_dispatcher(ast: &TemplateAst) -> Result<TokenStream> {
    let dispatcher_function_name = format_ident!("{}_main", ast.template_name);
    let function_names = get_function_names(ast);
    let function_blocks = get_function_blocks(ast);
    let uses = &ast.uses;

    let output = quote! {
        #[no_mangle]
        pub unsafe extern "C" fn #dispatcher_function_name(call_info: *mut u8, call_info_len: u32) -> *mut u8 {
            use ::tari_template_lib::template_dependencies::*;
            // include all use statements from the template module here as these may be used in the function arguments.
            #(
                #[allow(unused_imports)]
                #uses
            )*

            #[cfg(not(target_arch = "wasm32"))]
            compile_error!("Must compile template with --target wasm32-unknown-unknown");

            register_panic_hook();

            if call_info.is_null() {
                panic!("call_info is null");
            }

            let call_data = unsafe { Vec::from_raw_parts(call_info, call_info_len as usize, call_info_len as usize) };
            let call_info: CallInfo = decode_exact(&call_data).expect("Failed to decode CallArgs");

            init_context(&call_info);
            engine().emit_log(LogLevel::Info, format!("Dispatcher called with function {}", call_info.func_name));

            let result;
            match call_info.func_name.as_str() {
                #( #function_names => #function_blocks ),*,
                _ => panic!("invalid function name")
            };

            wrap_ptr(result)
        }
    };

    Ok(output)
}

fn get_function_names(ast: &TemplateAst) -> impl Iterator<Item = String> + '_ {
    ast.get_functions().map(|f| f.name)
}

fn get_function_blocks(ast: &TemplateAst) -> impl Iterator<Item = Expr> + '_ {
    ast.get_functions()
        .map(|function| get_function_block(&ast.template_name, function))
}

fn get_function_block(template_ident: &Ident, ast: FunctionAst) -> Expr {
    let template_mod_name = format_ident!("{}_template", template_ident);
    let mut args: Vec<Expr> = vec![];
    let expected_num_args = ast.input_types.len();
    let mut stmts = vec![];
    stmts.push(parse_quote! {
        assert_eq!(
            call_info.args.len(),
            #expected_num_args,
            "Call \"{}\" had unexpected number of args. Got = {} expected = {}",
            call_info.func_name,
            call_info.args.len(),
            #expected_num_args,
        );
    });
    let func_name = &ast.name;
    let mut is_mutable_call = false;
    // encode all arguments of the functions
    for (i, input_type) in ast.input_types.iter().enumerate() {
        let arg_ident = format_ident!("arg_{}", i);

        match input_type {
            // "self" argument
            TypeAst::Receiver { mutability } => {
                is_mutable_call = *mutability;
                if is_mutable_call {
                    args.push(parse_quote! { &mut state });
                } else {
                    args.push(parse_quote! { &state });
                }
                stmts.extend(
                [
                    parse_quote! {
                    let component_address = from_value::<::tari_template_lib::models::ComponentAddress>(&call_info.args[#i])
                        .unwrap_or_else(|e| panic!("failed to decode component instance for function '{}': {}",  #func_name, e));
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
                args.push(parse_quote! { #arg_ident });
                stmts.push(parse_quote! {
                    let #arg_ident = from_value::<#type_path>(&call_info.args[#i])
                        .unwrap_or_else(|e| panic!("failed to decode argument at position {} for function '{}': {}", #i, #func_name, e));
                })
            },
            TypeAst::Tuple { type_tuple, .. } => {
                args.push(parse_quote! { #arg_ident });
                stmts.push(parse_quote! {
                    let #arg_ident = from_value::<#type_tuple>(&call_info.args[#i])
                        .unwrap_or_else(|e| panic!("failed to decode tuple argument at position {} for function '{}': {}", #i, #func_name, e));
                });
            },
        }
    }

    // call the user defined function in the template
    let function_ident = Ident::new(&ast.name, Span::call_site());
    stmts.push(parse_quote! {
        let rtn = #template_mod_name::#template_ident::#function_ident(#(#args),*);
    });

    // replace "Self" if present in the return value
    stmts.extend(replace_self_in_output(&ast));

    // encode the result value
    stmts.push(parse_quote! {
        result = encode_with_len(&rtn);
    });

    // after user function invocation, update the component state
    if is_mutable_call {
        stmts.push(parse_quote! {
            component_manager.set_state(state);
        });
    }

    // construct the code block for the function
    Expr::Block(ExprBlock {
        attrs: vec![],
        label: None,
        block: Block {
            brace_token: Brace {
                span: Span::call_site(),
            },
            stmts,
        },
    })
}

fn replace_self_in_output(ast: &FunctionAst) -> Vec<Stmt> {
    let mut stmts: Vec<Stmt> = vec![];
    if let Some(output_type) = &ast.output_type {
        match output_type {
            TypeAst::Typed { type_path, .. } => {
                if let Some(stmt) = replace_self_in_single_value(type_path) {
                    stmts.push(stmt);
                }
            },
            TypeAst::Tuple { type_tuple, .. } => {
                stmts.push(replace_self_in_tuple(type_tuple));
            },
            _ => todo!("replace_self_in_output only supports typed and tuple"),
        }
    }

    stmts
}

fn replace_self_in_single_value(type_path: &TypePath) -> Option<Stmt> {
    let type_ident = &type_path.path.segments[0].ident;

    if type_ident == "Self" {
        // When we return self we use default rules - which only permit the owner of the component to call methods
        return Some(parse_quote! {
            let rtn = engine().create_component(
                rtn,
                ::tari_template_lib::auth::OwnerRule::default(),
                ::tari_template_lib::auth::ComponentAccessRules::new(),
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
                let ident = path.path.segments[0].ident.clone();
                let field_expr = build_tuple_field_expr("rtn".to_string(), i as u32);
                if ident == "Self" {
                    // When we return self we use default rules - which only permit the owner of the component to call
                    // methods
                    parse_quote! {
                        engine().create_component(
                            #field_expr,
                            ::tari_template_lib::auth::OwnerRule::default(),
                            :tari_template_lib::auth::ComponentAccessRules::new(),
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
