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

use crate::template::ast::TemplateAst;

pub fn generate_definition(ast: &TemplateAst) -> TokenStream {
    let template_mod_name = format_ident!("{}_template", ast.template_name);
    let component_ident = ast.template_name.clone();
    let component_ident_as_str = component_ident.to_string();
    let component_wrapper_ident = format_ident!("{}Component", ast.template_name);
    let (_, items) = ast.module.content.as_ref().unwrap();

    quote! {
        #[allow(non_snake_case)]
        pub mod #template_mod_name {
            use ::tari_template_lib::template_dependencies::*;

            #(#items)*

            impl ::tari_template_lib::component::interface::ComponentInterface for #component_ident {
                type Component = #component_wrapper_ident;

                fn create_with_access_rules(self, access_rules: ::tari_template_lib::auth::AccessRules) -> Self::Component {
                    let address = engine().create_component(#component_ident_as_str.to_string(), self, access_rules);
                    #component_wrapper_ident{ address }
                }
            }

            #[derive(serde::Serialize, serde::Deserialize)]
            #[serde(transparent, crate = "self::serde")]
            pub struct #component_wrapper_ident {
                address: tari_template_lib::models::ComponentAddress,
            }

            impl ::tari_template_lib::component::interface::ComponentInstanceInterface for #component_wrapper_ident {
                fn set_access_rules(self, rules: tari_template_lib::auth::AccessRules) -> Self {
                    engine().component_manager(self.address).set_access_rules(rules);
                    self
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use indoc::indoc;
    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::parse2;

    use super::generate_definition;
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
                    pub fn constructor() -> Self {
                      Self{}
                    }
                    pub fn method(&self){}
                    fn private_function() {}
                }
            }
        "})
        .unwrap();

        let ast = parse2::<TemplateAst>(input).unwrap();

        let output = generate_definition(&ast);

        assert_code_eq(output, quote! {
                   # [allow (non_snake_case)]
        pub mod Foo_template {
            use :: tari_template_lib :: template_dependencies :: * ;
            # [derive (Debug ,  serde :: Serialize , serde :: Deserialize)] # [serde (crate = "self::serde")] struct Foo { }
            impl Foo { pub fn no_args_function () -> String { "Hello World!" . to_string () }
                pub fn some_args_function (a : i8 , b : String) -> u32 { 1_u32 }
                pub fn no_return_function () { }
                pub fn constructor () -> Self { Self { } }
                pub fn method (& self) { }
                fn private_function () { }
            }
            impl :: tari_template_lib :: component :: interface :: ComponentInterface for Foo {
                type Component = FooComponent ;
                fn create_with_access_rules (self , access_rules : :: tari_template_lib :: auth :: AccessRules) -> Self :: Component {
                    let address = engine () . create_component ("Foo" . to_string () , self , access_rules) ;
                    FooComponent { address }
                }
            }
            # [derive (serde :: Serialize , serde :: Deserialize)] # [serde (transparent , crate = "self::serde")]
            pub struct FooComponent {
                address : tari_template_lib :: models :: ComponentAddress ,
            }
            impl :: tari_template_lib :: component :: interface :: ComponentInstanceInterface for FooComponent {
                fn set_access_rules (self , rules : tari_template_lib :: auth :: AccessRules) -> Self {
                    engine () . component_manager (self . address) . set_access_rules (rules) ;
                    self
                }
            }
        }
                });
    }

    fn assert_code_eq(a: TokenStream, b: TokenStream) {
        assert_eq!(a.to_string(), b.to_string());
    }
}
