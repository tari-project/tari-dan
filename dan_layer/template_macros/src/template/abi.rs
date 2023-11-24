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
use syn::{AngleBracketedGenericArguments, GenericArgument, PathArguments, PathSegment, Result, Type, TypeTuple};
use tari_template_abi::{
    ArgDef,
    FunctionDef,
    TemplateDef,
    TemplateDefV1,
    Type as ArgType,
    ABI_TEMPLATE_DEF_GLOBAL_NAME,
};

use crate::template::ast::{TemplateAst, TypeAst};

pub fn generate_abi(ast: &TemplateAst) -> Result<TokenStream> {
    let template_name_as_str = ast.template_name.to_string();

    let template_def = TemplateDef::V1(TemplateDefV1 {
        template_name: template_name_as_str.clone(),
        functions: ast
            .get_functions()
            .map(|func| {
                let is_mut = func.is_mut();
                Ok::<_, syn::Error>(FunctionDef {
                    name: func.name,
                    arguments: func
                        .input_types
                        .iter()
                        .map(|ty| convert_to_arg_def(&template_name_as_str, ty))
                        .collect::<Result<_>>()?,
                    output: func
                        .output_type
                        .as_ref()
                        .map(|ty| convert_to_arg_type(&template_name_as_str, ty))
                        .unwrap_or(ArgType::Unit),
                    is_mut,
                })
            })
            .collect::<Result<_>>()?,
    });

    let template_def_data = tari_bor::encode_with_len(&template_def);
    let len = template_def_data.len();
    let template_def_name = format_ident!("{ABI_TEMPLATE_DEF_GLOBAL_NAME}");

    let output = quote! {
        #[no_mangle]
        pub static #template_def_name: [u8;#len] = [#(#template_def_data),*];
    };

    Ok(output)
}

fn convert_to_arg_type(template_name: &str, ty: &TypeAst) -> ArgType {
    match ty {
        TypeAst::Receiver { mutability: true } => ArgType::Other {
            name: "&mut self".to_string(),
        },
        TypeAst::Receiver { mutability: false } => ArgType::Other {
            name: "&self".to_string(),
        },
        TypeAst::Typed { type_path, .. } => path_segment_to_arg_type(template_name, &type_path.path.segments[0]),
        TypeAst::Tuple { type_tuple, .. } => tuple_to_arg_type(template_name, type_tuple),
    }
}

fn convert_to_arg_def(template_name: &str, rust_type: &TypeAst) -> Result<ArgDef> {
    match rust_type {
        // on "&self" we want to pass the component id
        TypeAst::Receiver { mutability: false } => Ok(ArgDef {
            name: "self".to_string(),
            arg_type: ArgType::Other {
                name: "&self".to_string(),
            },
        }),
        TypeAst::Receiver { mutability: true } => Ok(ArgDef {
            name: "self".to_string(),
            arg_type: ArgType::Other {
                name: "&mut self".to_string(),
            },
        }),
        // basic type
        TypeAst::Typed {
            name: arg_name,
            type_path: path,
        } => {
            let Some(arg_name) = arg_name else {
                return Err(syn::Error::new_spanned(
                    path,
                    "convert_to_arg_def: Unnamed type is not valid in this context",
                ));
            };

            let arg_type = path_segment_to_arg_type(template_name, &path.path.segments[0]);

            Ok(ArgDef {
                name: arg_name.to_string(),
                arg_type,
            })
        },
        TypeAst::Tuple {
            name: arg_name,
            type_tuple,
        } => {
            let Some(arg_name) = arg_name else {
                return Err(syn::Error::new_spanned(
                    type_tuple,
                    "convert_to_arg_def: Unnamed type is not valid in this context",
                ));
            };
            let arg_type = tuple_to_arg_type(template_name, type_tuple);
            Ok(ArgDef {
                name: arg_name.to_string(),
                arg_type,
            })
        },
    }
}

fn path_segment_to_arg_type(template_name: &str, segment: &PathSegment) -> ArgType {
    match segment.ident.to_string().as_str() {
        "" => ArgType::Unit,
        "bool" => ArgType::Bool,
        "i8" => ArgType::I8,
        "i16" => ArgType::I16,
        "i32" => ArgType::I32,
        "i64" => ArgType::I64,
        "i128" => ArgType::I128,
        "u8" => ArgType::U8,
        "u16" => ArgType::U16,
        "u32" => ArgType::U32,
        "u64" => ArgType::U64,
        "u128" => ArgType::U128,
        "String" => ArgType::String,
        "Vec" => {
            match &segment.arguments {
                PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) => {
                    match &args[0] {
                        GenericArgument::Type(Type::Path(path)) => {
                            let ty = path_segment_to_arg_type(template_name, &path.path.segments[0]);
                            ArgType::Vec(Box::new(ty))
                        },
                        GenericArgument::Type(Type::Tuple(tuple)) => tuple_to_arg_type(template_name, tuple),
                        // TODO: These should be errors
                        a => panic!("Invalid vec generic argument {:?}", a),
                    }
                },
                PathArguments::Parenthesized(_) | PathArguments::None => {
                    panic!("Vec must specify a type {:?}", segment)
                },
            }
        },
        "Self" => ArgType::Other {
            name: format!("Component<{}>", template_name),
        },
        type_name => ArgType::Other {
            name: type_name.to_string(),
        },
    }
}

fn tuple_to_arg_type(template_name: &str, tuple: &TypeTuple) -> ArgType {
    let subtypes = tuple
        .elems
        .iter()
        .map(|t| {
            match t {
                Type::Path(path) => path_segment_to_arg_type(template_name, &path.path.segments[0]),
                Type::Tuple(subtuple) => tuple_to_arg_type(template_name, subtuple),
                // TODO: These should be errors
                a => panic!("Invalid Tuple subtype argument {:?}", a),
            }
        })
        .collect::<Vec<_>>();

    ArgType::Tuple(subtypes)
}
