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

use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token::Comma,
    Error,
    FnArg,
    Ident,
    ImplItem,
    ImplItemMethod,
    Item,
    ItemMod,
    Result,
    ReturnType,
    Signature,
    Stmt,
    TypePath,
    TypeTuple,
};

#[allow(dead_code)]
pub struct TemplateAst {
    pub template_name: Ident,
    pub module: ItemMod,
}

impl Parse for TemplateAst {
    fn parse(input: ParseStream) -> Result<Self> {
        // parse the "mod" block
        let mut module: ItemMod = input.parse()?;

        // get the contents of the "mod" block
        let items = match module.content {
            Some((_, ref mut items)) => items,
            None => return Err(Error::new(module.ident.span(), "empty module")),
        };

        // add derive macros to all structs
        let mut template_name = None;
        let mut has_impl = false;

        for item in items {
            match item {
                Item::Struct(ref mut item) => {
                    item.attrs.push(syn::parse_quote!(#[derive(Debug, Decode, Encode)]));
                    // Use the first struct name as the template name
                    // TODO: remove this assumption in favor of "marking" the struct as a template struct
                    // #[template(Component)]
                    if template_name.is_none() {
                        template_name = Some(item.ident.clone());
                    }
                },
                // TODO: check name matches template name
                Item::Impl(_) => {
                    has_impl = true;
                },
                _ => {},
            }
        }

        if template_name.is_none() {
            return Err(Error::new(module.ident.span(), "a template must define a struct"));
        }

        if !has_impl {
            return Err(Error::new(
                module.ident.span(),
                "a template must have associated functions and/or methods",
            ));
        }

        Ok(Self {
            template_name: template_name.unwrap(),
            module,
        })
    }
}

impl TemplateAst {
    pub fn get_functions(&self) -> impl Iterator<Item = FunctionAst> + '_ {
        self.module
            .content
            .iter()
            .flat_map(|(_, items)| items)
            .filter_map(|i| match i {
                Item::Impl(impl_item) => Some(&impl_item.items),
                _ => None,
            })
            .flatten()
            .filter_map(Self::get_function_from_item)
    }

    fn get_function_from_item(item: &ImplItem) -> Option<FunctionAst> {
        match item {
            ImplItem::Method(m) => {
                if !Self::is_public_function(m) {
                    return None;
                }
                Some(FunctionAst {
                    name: m.sig.ident.to_string(),
                    input_types: Self::get_input_types(&m.sig.inputs),
                    output_type: Self::get_output_type_token(&m.sig.output),
                    statements: Self::get_statements(m),
                    is_constructor: Self::is_constructor(&m.sig),
                    is_public: true,
                })
            },
            _ => todo!("get_function_from_item does not support anything other than methods"),
        }
    }

    fn get_input_types(inputs: &Punctuated<FnArg, Comma>) -> Vec<TypeAst> {
        inputs
            .iter()
            .map(|arg| match arg {
                // TODO: handle the "self" case
                syn::FnArg::Receiver(r) => {
                    if r.reference.is_none() {
                        panic!("Consuming methods are not supported")
                    }

                    let mutability = r.mutability.is_some();
                    TypeAst::Receiver { mutability }
                },
                syn::FnArg::Typed(t) => Self::get_type_ast(Some(&t.pat), &t.ty),
            })
            .collect()
    }

    fn get_output_type_token(ast_type: &ReturnType) -> Option<TypeAst> {
        match ast_type {
            ReturnType::Default => None, // the function does not return anything
            ReturnType::Type(_, t) => Some(Self::get_type_ast(None, t)),
        }
    }

    fn get_type_ast(pat: Option<&syn::Pat>, syn_type: &syn::Type) -> TypeAst {
        match syn_type {
            syn::Type::Path(type_path) => {
                // TODO: handle "Self"
                // TODO: detect more complex types
                TypeAst::Typed {
                    name: pat.map(|p| format!("{:?}", p)),
                    type_path: type_path.clone(),
                }
            },
            syn::Type::Tuple(tuple) => TypeAst::Tuple(tuple.clone()),
            _ => todo!(
                "get_type_ast only supports paths and tuples. Encountered:{:?}",
                syn_type
            ),
        }
    }

    fn get_statements(method: &ImplItemMethod) -> Vec<Stmt> {
        method.block.stmts.clone()
    }

    fn is_constructor(sig: &Signature) -> bool {
        match &sig.output {
            ReturnType::Default => false, // the function does not return anything
            ReturnType::Type(_, t) => match t.as_ref() {
                syn::Type::Path(type_path) => type_path.path.segments[0].ident == "Self",
                _ => false,
            },
        }
    }

    fn is_public_function(item: &ImplItemMethod) -> bool {
        matches!(item.vis, syn::Visibility::Public(_))
    }
}

pub struct FunctionAst {
    pub name: String,
    pub input_types: Vec<TypeAst>,
    pub output_type: Option<TypeAst>,
    pub statements: Vec<Stmt>,
    pub is_constructor: bool,
    pub is_public: bool,
}

pub enum TypeAst {
    Receiver { mutability: bool },
    Typed { name: Option<String>, type_path: TypePath },
    Tuple(TypeTuple),
}

impl TypeAst {
    pub fn is_mut(&self) -> bool {
        match self {
            TypeAst::Receiver { mutability } => *mutability,
            _ => false,
        }
    }
}
