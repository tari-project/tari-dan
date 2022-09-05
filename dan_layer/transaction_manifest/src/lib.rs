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

use instruction::{Arg, Instruction, Value};
use proc_macro2::TokenStream;
use syn::{parse2, punctuated::Punctuated, token::Comma, Expr, ExprCall, ExprMethodCall, Local, Path};

use self::{ast::ManifestAst, instruction::VariableIdent};

mod ast;
pub mod instruction;

pub fn parse_manifest(input: String) -> Vec<Instruction> {
    let tokens: TokenStream = input.parse().unwrap();
    let ast = parse2::<ManifestAst>(tokens).unwrap();
    let input_stmts = ast.stmts;

    let mut instructions: Vec<Instruction> = vec![];
    for stmt in input_stmts {
        let mut expr_instructions: Vec<Instruction> = match stmt {
            syn::Stmt::Local(binding) => instruction_from_local(binding),
            syn::Stmt::Expr(expr) => instruction_from_expr(expr),
            syn::Stmt::Semi(expr, _) => instruction_from_expr(expr),
            syn::Stmt::Item(_) => todo!(),
        };

        instructions.append(&mut expr_instructions);
    }

    instructions
}

fn instruction_from_local(local: Local) -> Vec<Instruction> {
    let variable_name = match local.pat {
        syn::Pat::Ident(ident) => ident.ident.to_string(),
        syn::Pat::Type(pt) => {
            match *pt.pat {
                syn::Pat::Ident(ident) => ident.ident.to_string(),
                _ => todo!(),
            }

            // TODO: handle mutability? types?
        },
        syn::Pat::Tuple(tuple) => {
            // TODO: properly handle all elements in the tuple
            match &tuple.elems[0] {
                syn::Pat::Ident(ident) => ident.ident.to_string(),
                _ => todo!(),
            }
        },
        _ => {
            println!("TODO: {:#?}", local.pat);
            todo!()
        },
    };

    let (_, expr) = local.init.unwrap();
    let result = match *expr {
        Expr::Call(call) => build_call_function(call, variable_name),
        Expr::MethodCall(call) => Some(build_call_method(call)),
        _ => {
            println!("TODO: {:#?}", *expr);
            todo!()
        },
    };

    // TODO: handle literal assigments ("let x = Amount(1_000)")
    if let Some(expr) = result {
        return vec![expr];
    }

    vec![]
}

fn build_call_function(expr: ExprCall, variable_name: VariableIdent) -> Option<Instruction> {
    let segments = match *expr.func {
        Expr::Path(expr_path) => path_segments(expr_path.path),
        _ => todo!(),
    };

    // "Struct::function"
    if segments.len() != 2 {
        return None;
    }

    let template = segments[0].clone();
    let function = segments[1].clone();

    let args = build_call_args(expr.args);

    let instruction = Instruction::CallFunction {
        package_address: String::new(),
        template,
        function,
        proofs: vec![],
        args,
        return_variables: vec![variable_name],
    };

    Some(instruction)
}

fn path_segments(path: Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

fn instruction_from_expr(expr: Expr) -> Vec<Instruction> {
    let result = match expr {
        Expr::Assign(_) => todo!(),
        Expr::Call(_) => todo!(),
        Expr::Field(_) => todo!(),
        Expr::Lit(_) => todo!(),
        Expr::MethodCall(call) => build_call_method(call),
        Expr::Tuple(tuple_expr) => {
            println!("TODO: {:#?}", tuple_expr);
            todo!()
        },
        _ => todo!(),
    };

    vec![result]
}

fn build_call_method(expr: ExprMethodCall) -> Instruction {
    let method = expr.method.to_string();

    // TODO: component address, etc

    let args = build_call_args(expr.args);

    Instruction::CallMethod {
        package_address: String::new(),
        component_address: String::new(),
        method,
        proofs: vec![],
        args,
        return_variables: vec![],
    }
}

fn build_call_args(args: Punctuated<Expr, Comma>) -> Vec<Arg> {
    args.iter()
        .map(|arg| {
            match arg {
                Expr::Lit(lit) => match &lit.lit {
                    syn::Lit::Str(s) => Arg::Literal(Value::String(s.value())),
                    syn::Lit::Int(i) => Arg::Literal(Value::U64(i.base10_parse().unwrap())),
                    syn::Lit::Bool(b) => Arg::Literal(Value::Bool(b.value())),
                    _ => todo!(),
                },
                Expr::Path(expr_path) => {
                    // variable names should only have one segment
                    let variable_name = expr_path.path.segments[0].ident.to_string();
                    Arg::Variable(variable_name)
                },
                _ => todo!(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::parse_manifest;
    use crate::instruction::{Arg, Instruction, Value};

    #[test]
    #[allow(clippy::too_many_lines)]
    fn buy_nft() {
        let input = indoc! {"
            // initialize the component
            let mut picture_seller = PictureSeller::new(1_000);

            // initialize a user account with enough funds
            let mut account = Account::new();
            let funds = ThaumFaucet::take(1_000);
            account.add_fungible(funds);

            // buy a picture
            let payment: Bucket<Thaum> = account.take_fungible(1_000);
            let (picture, _) = picture_seller.buy(payment);

            // store our brand new picture in our account
            account.add_non_fungible(picture);
        "};
        let instructions = parse_manifest(input.to_string());

        let expected = vec![
            Instruction::CallFunction {
                package_address: String::new(),
                template: "PictureSeller".to_string(),
                function: "new".to_string(),
                proofs: vec![],
                args: vec![Arg::Literal(Value::U64(1_000))],
                return_variables: vec!["picture_seller".to_string()],
            },
            Instruction::CallFunction {
                package_address: String::new(),
                template: "Account".to_string(),
                function: "new".to_string(),
                proofs: vec![],
                args: vec![],
                return_variables: vec!["account".to_string()],
            },
            Instruction::CallFunction {
                package_address: String::new(),
                template: "ThaumFaucet".to_string(),
                function: "take".to_string(),
                proofs: vec![],
                args: vec![Arg::Literal(Value::U64(1_000))],
                return_variables: vec!["funds".to_string()],
            },
            Instruction::CallMethod {
                package_address: String::new(),
                component_address: String::new(),
                method: "add_fungible".to_string(),
                proofs: vec![],
                args: vec![Arg::Variable("funds".to_string())],
                return_variables: vec![],
            },
            Instruction::CallMethod {
                package_address: String::new(),
                component_address: String::new(),
                method: "take_fungible".to_string(),
                proofs: vec![],
                args: vec![Arg::Literal(Value::U64(1_000))],
                return_variables: vec![],
            },
            Instruction::CallMethod {
                package_address: String::new(),
                component_address: String::new(),
                method: "buy".to_string(),
                proofs: vec![],
                args: vec![Arg::Variable("payment".to_string())],
                return_variables: vec![],
            },
            Instruction::CallMethod {
                package_address: String::new(),
                component_address: String::new(),
                method: "add_non_fungible".to_string(),
                proofs: vec![],
                args: vec![Arg::Variable("picture".to_string())],
                return_variables: vec![],
            },
        ];

        assert_eq!(instructions, expected);
    }
}
