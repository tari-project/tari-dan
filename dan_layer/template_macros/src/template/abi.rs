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

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_quote, AngleBracketedGenericArguments, Expr, GenericArgument, PathArguments, Result, Type};

use crate::template::ast::{FunctionAst, TemplateAst, TypeAst};

pub fn generate_abi(ast: &TemplateAst) -> Result<TokenStream> {
    let abi_function_name = format_ident!("{}_abi", ast.template_name);
    let template_name_as_str = ast.template_name.to_string();
    let function_defs = ast
        .get_functions()
        .map(|func| generate_function_def(&template_name_as_str, &func));

    let output = quote! {
        #[no_mangle]
        pub unsafe extern "C" fn #abi_function_name() -> *mut u8 {
            use ::tari_template_abi::{ArgDef, FunctionDef, TemplateDef, Type, wrap_ptr};
            use ::tari_template_lib::template_dependencies::encode_with_len;

            let template = TemplateDef {
                template_name: #template_name_as_str.to_string(),
                functions: vec![ #(#function_defs),* ],
            };

            let buf = encode_with_len(&template);
            wrap_ptr(buf)
        }
    };

    Ok(output)
}

fn generate_function_def(template_name: &str, f: &FunctionAst) -> Expr {
    let name = f.name.clone();
    let is_mut = f.input_types.first().map(|a| a.is_mut()).unwrap_or(false);
    let arguments = f.input_types.iter().map(|t| generate_abi_type(template_name, t));

    let output = match &f.output_type {
        Some(type_ast) => generate_abi_type(template_name, type_ast),
        None => parse_quote!(Type::Unit),
    };

    parse_quote!(
        FunctionDef {
            name: #name.to_string(),
            arguments: vec![ #(#arguments),* ],
            output: #output,
            is_mut: #is_mut,
        }
    )
}

fn generate_abi_type(template_name: &str, rust_type: &TypeAst) -> Expr {
    match rust_type {
        // on "&self" we want to pass the component id
        TypeAst::Receiver { mutability: false } => {
            let ty = get_component_address_type("&self");
            parse_quote!(ArgDef {
                name: "self".to_string(),
                arg_type: #ty
            })
        },
        TypeAst::Receiver { mutability: true } => {
            let ty = get_component_address_type("&mut self");
            parse_quote!(ArgDef {
                name: "self".to_string(),
                arg_type: #ty
            })
        },
        // basic type
        // TODO: there may be a better way of handling this
        TypeAst::Typed {
            name: arg_name,
            type_path: path,
        } => {
            let type_str = match path.path.segments[0].ident.to_string().as_str() {
                "" => parse_quote!(Type::Unit),
                "bool" => parse_quote!(Type::Bool),
                "i8" => parse_quote!(Type::I8),
                "i16" => parse_quote!(Type::I16),
                "i32" => parse_quote!(Type::I32),
                "i64" => parse_quote!(Type::I64),
                "i128" => parse_quote!(Type::I128),
                "u8" => parse_quote!(Type::U8),
                "u16" => parse_quote!(Type::U16),
                "u32" => parse_quote!(Type::U32),
                "u64" => parse_quote!(Type::U64),
                "u128" => parse_quote!(Type::U128),
                "String" => parse_quote!(Type::String),
                "Vec" => {
                    let ty = match &path.path.segments[0].arguments {
                        PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) => {
                            match &args[0] {
                                GenericArgument::Type(Type::Path(path)) => {
                                    match path.path.segments[0].ident.to_string().as_str() {
                                        "" => parse_quote!(Type::Unit),
                                        "bool" => parse_quote!(Type::Bool),
                                        "i8" => parse_quote!(Type::I8),
                                        "i16" => parse_quote!(Type::I16),
                                        "i32" => parse_quote!(Type::I32),
                                        "i64" => parse_quote!(Type::I64),
                                        "i128" => parse_quote!(Type::I128),
                                        "u8" => parse_quote!(Type::U8),
                                        "u16" => parse_quote!(Type::U16),
                                        "u32" => parse_quote!(Type::U32),
                                        "u64" => parse_quote!(Type::U64),
                                        "u128" => parse_quote!(Type::U128),
                                        "String" => parse_quote!(Type::String),
                                        "Vec" => {
                                            panic!("Nested Vecs are not supported")
                                        },
                                        "Self" => get_component_address_type(&format!("{}Component", template_name)),
                                        name => parse_quote!(Type::Other { name: #name.to_string() }),
                                    }
                                },
                                GenericArgument::Type(Type::Tuple(tuple)) => {
                                    // FIXME: improve
                                    let tuple_str = tuple
                                        .elems
                                        .iter()
                                        .map(|t| format!("{:?}", t))
                                        .collect::<Vec<_>>()
                                        .join(",");
                                    parse_quote!(Type::Other { name: #tuple_str.to_string() })
                                },
                                // TODO: These should be errors
                                a => panic!("Invalid vec generic argument {:?}", a),
                            }
                        },
                        PathArguments::Parenthesized(_) | PathArguments::None => {
                            panic!("Vec must specify a type {:?}", path.path)
                        },
                    };

                    parse_quote!(Type::Vec(Box::new(#ty)))
                },
                "Self" => get_component_address_type(&format!("{}Component", template_name)),
                type_name => parse_quote!(Type::Other { name: #type_name.to_string() }),
            };
            // For arguments, put the name and type. For return types, just return the type
            match arg_name {
                Some(name) => parse_quote!(ArgDef {
                    name: #name.to_string(),
                    arg_type: #type_str,
                }),
                None => type_str,
            }
        },

        TypeAst::Tuple(_) => {
            // TODO: Handle tuples properly
            parse_quote!(Type::Other {
                name: "tuple".to_string()
            })
        },
    }
}

fn get_component_address_type(name: &str) -> Expr {
    parse_quote!(Type::Other { name: #name.to_string() })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use indoc::indoc;
    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::parse2;

    use super::generate_abi;
    use crate::template::ast::TemplateAst;

    #[test]
    fn test_codegen() {
        let input = TokenStream::from_str(indoc! {"
            mod foo {
                struct Foo {}
                impl Foo {
                    pub fn no_args_function() -> String {
                        \"Hello World!\".to_string()
                    }
                    pub fn some_args_function(a: i8, b: String) -> u32 {
                        1_u32
                    }
                    pub fn no_return_function() {}
                    pub fn constructor() -> Self {}
                    pub fn method(&self){}
                    pub fn method_mut(&mut self){}
                    fn private_function() {}
                }
            }
        "})
        .unwrap();

        let ast = parse2::<TemplateAst>(input).unwrap();

        let output = generate_abi(&ast).unwrap();

        assert_code_eq(output, quote! {
            #[no_mangle]
            pub unsafe extern "C" fn Foo_abi() -> *mut u8 {
                use ::tari_template_abi::{ArgDef, FunctionDef, TemplateDef, Type, wrap_ptr};
                use ::tari_template_lib::template_dependencies::encode_with_len;

                let template = TemplateDef {
                    template_name: "Foo".to_string(),
                    functions: vec![
                        FunctionDef {
                            name: "no_args_function".to_string(),
                            arguments: vec![],
                            output: Type::String,
                            is_mut: false,
                        },
                        FunctionDef {
                            name: "some_args_function".to_string(),
                            arguments: vec![ArgDef{ name: "a".to_string(), arg_type: Type::I8, },
                                ArgDef{ name: "b".to_string(), arg_type: Type::String, }],
                            output: Type::U32,
                            is_mut: false,
                        },
                        FunctionDef {
                            name: "no_return_function".to_string(),
                            arguments: vec![],
                            output: Type::Unit,
                            is_mut: false,
                        },
                        FunctionDef {
                            name: "constructor".to_string(),
                            arguments: vec![],
                            output: Type::Other { name: "FooComponent".to_string() },
                            is_mut: false,
                        },
                        FunctionDef {
                            name: "method".to_string(),
                            arguments: vec![ArgDef{ name: "self".to_string(), arg_type: Type::Other { name: "&self".to_string() }}],
                            output: Type::Unit,
                            is_mut: false,
                        },
                         FunctionDef {
                            name: "method_mut".to_string(),
                            arguments: vec![ArgDef{ name: "self".to_string(), arg_type: Type::Other { name: "&mut self".to_string() }}],
                            output: Type::Unit,
                            is_mut: true,
                        }
                    ],
                };

                let buf = encode_with_len(&template);
                wrap_ptr(buf)
            }
        });
    }

    fn assert_code_eq(a: TokenStream, b: TokenStream) {
        assert_eq!(a.to_string(), b.to_string());
    }
}
