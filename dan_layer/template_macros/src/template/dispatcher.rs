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
use syn::{parse_quote, token::Brace, Block, Expr, ExprBlock, ExprField, Result, Stmt, TypeTuple};

use crate::ast::{FunctionAst, TemplateAst, TypeAst};

pub fn generate_dispatcher(ast: &TemplateAst) -> Result<TokenStream> {
    let dispatcher_function_name = format_ident!("{}_main", ast.template_name);
    let function_names = get_function_names(ast);
    let function_blocks = get_function_blocks(ast);

    let output = quote! {
        #[no_mangle]
        pub extern "C" fn #dispatcher_function_name(call_info: *mut u8, call_info_len: usize) -> *mut u8 {
            use ::tari_template_abi::{decode, encode_with_len, CallInfo, wrap_ptr};
            use ::tari_template_lib::init_context;

            if call_info.is_null() {
                panic!("call_info is null");
            }

            let call_data = unsafe { Vec::from_raw_parts(call_info, call_info_len, call_info_len) };
            let call_info: CallInfo = decode(&call_data).unwrap();

            init_context(&call_info);
            // TODO: wrap this in a nice macro
            engine().emit_log(LogLevel::Debug, format!("Dispatcher called with function {}", call_info.func_name));

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

fn get_function_names(ast: &TemplateAst) -> Vec<String> {
    ast.get_functions().iter().map(|f| f.name.clone()).collect()
}

fn get_function_blocks(ast: &TemplateAst) -> Vec<Expr> {
    let mut blocks = vec![];

    for function in ast.get_functions() {
        let block = get_function_block(&ast.template_name, function);
        blocks.push(block);
    }

    blocks
}

fn get_function_block(template_ident: &Ident, ast: FunctionAst) -> Expr {
    let template_mod_name = format_ident!("{}_template", template_ident);
    let mut args: Vec<Expr> = vec![];
    let expected_num_args = ast.input_types.len();
    let mut stmts = vec![];
    let mut should_set_state = false;

    // encode all arguments of the functions
    for (i, input_type) in ast.input_types.iter().enumerate() {
        let arg_ident = format_ident!("arg_{}", i);
        stmts.push(parse_quote! {
            assert_eq!(call_info.args.len(), #expected_num_args, "Call had unexpected number of args. Got = {} expected = {}", call_info.args.len(), #expected_num_args); 
        });
        let stmt = match input_type {
            // "self" argument
            TypeAst::Receiver { mutability } => {
                should_set_state = *mutability;
                args.push(parse_quote! { &mut state });
                vec![
                    parse_quote! {
                        let component =
                            decode::<::tari_template_lib::models::ComponentInstance>(&call_info.args[#i])
                            .unwrap();
                    },
                    parse_quote! {
                        let mut state = decode::<#template_mod_name::#template_ident>(&component.state).unwrap();
                    },
                ]
            },
            // non-self argument
            TypeAst::Typed(type_ident) => {
                args.push(parse_quote! { #arg_ident });
                vec![parse_quote! {
                    let #arg_ident =
                        decode::<#type_ident>(&call_info.args[#i])
                        .unwrap();
                }]
            },
            TypeAst::Tuple(tuple) => {
                args.push(parse_quote! { #arg_ident });
                vec![parse_quote! {
                    let #arg_ident =
                        decode::<#tuple>(&call_info.args[#i])
                        .unwrap();
                }]
            },
        };
        stmts.extend(stmt);
    }

    // call the user defined function in the template
    let function_ident = Ident::new(&ast.name, Span::call_site());
    stmts.push(parse_quote! {
        let rtn = #template_mod_name::#template_ident::#function_ident(#(#args),*);
    });

    // replace "Self" if present in the return value
    stmts.append(&mut replace_self_in_output(template_ident, &ast));

    // encode the result value
    stmts.push(parse_quote! {
        result = encode_with_len(&rtn);
    });

    // after user function invocation, update the component state
    if should_set_state {
        stmts.push(parse_quote! {
            engine().set_component_state(component.id(), state);
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

fn replace_self_in_output(template_ident: &Ident, ast: &FunctionAst) -> Vec<Stmt> {
    let mut stmts: Vec<Stmt> = vec![];
    match &ast.output_type {
        Some(output_type) => match output_type {
            TypeAst::Typed(type_ident) => {
                if let Some(stmt) = replace_self_in_single_value(template_ident, type_ident) {
                    stmts.push(stmt);
                }
            },
            TypeAst::Tuple(type_tuple) => {
                stmts.push(replace_self_in_tuple(template_ident, type_tuple));
            },
            _ => todo!(),
        },
        None => {},
    }

    stmts
}

fn replace_self_in_single_value(template_ident: &Ident, type_ident: &Ident) -> Option<Stmt> {
    let template_name_str = template_ident.to_string();

    if type_ident == "Self" {
        return Some(parse_quote! {
            let rtn = engine().instantiate(#template_name_str.to_string(), rtn);
        });
    }

    None
}

fn replace_self_in_tuple(template_ident: &Ident, type_tuple: &TypeTuple) -> Stmt {
    let template_name_str = template_ident.to_string();

    // build the expresions for each element in the tuple
    let elems: Vec<Expr> = type_tuple
        .elems
        .iter()
        .enumerate()
        .map(|(i, t)| match t {
            syn::Type::Path(path) => {
                let ident = path.path.segments[0].ident.clone();
                let field_expr = build_tuple_field_expr("rtn".to_string(), i as u32);
                if ident == "Self" {
                    parse_quote! {
                        engine().instantiate(#template_name_str.to_string(), #field_expr)
                    }
                } else {
                    field_expr
                }
            },
            _ => todo!(),
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
            unnamed.index = i as u32;
        },
        _ => todo!(),
    }

    Expr::Field(field_expr)
}
