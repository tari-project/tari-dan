use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, parse_quote, Expr, Local, Result, Stmt};

use self::ast::TransactionAst;

mod ast;
mod builder;

pub fn generate_transaction(input: TokenStream) -> Result<TokenStream> {
    let ast = parse2::<TransactionAst>(input).unwrap();
    let input_stmts = ast.stmts;

    let mut output_stmts: Vec<Stmt> = vec![];
    output_stmts.push(parse_quote! {
        let mut builder = TransactionBuilder::new();
    });

    for stmt in input_stmts {
        let mut instruction_stmts: Vec<Stmt> = match stmt {
            syn::Stmt::Local(binding) => instruction_from_local(binding),
            syn::Stmt::Expr(expr) => instruction_from_expr(expr),
            syn::Stmt::Semi(expr, _) => instruction_from_expr(expr),
            syn::Stmt::Item(_) => todo!(),
        };

        output_stmts.append(&mut instruction_stmts);
    }

    // we return the transaction builder
    output_stmts.push(parse_quote! {
        return builder;
    });

    let output_tokens = quote! {
        {
            #(#output_stmts)*
        }
    };

    Ok(output_tokens)
}

pub fn instruction_from_local(binding: Local) -> Vec<Stmt> {
    vec![]
}

pub fn instruction_from_expr(expr: Expr) -> Vec<Stmt> {
    vec![]
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use indoc::indoc;
    use proc_macro2::TokenStream;
    use quote::quote;

    use super::generate_transaction;

    #[test]
    #[allow(clippy::too_many_lines)]
    fn check_correct_code_generation() {
        let input = TokenStream::from_str(indoc! {"
            // initialize the component
            let price = Amount(1_000);
            let mut picture_seller = PictureSeller::new(price);

            // initialize a user account with enough funds
            let mut account = Account::new();
            account.add_fungible(ThaumFaucet::take(price));

            // buy a picture
            let payment: Bucket<Thaum> = account.take_fungible(price);
            let (picture, _) = picture_seller.buy(payment).unwrap();

            // store our brand new picture in our account
            account.add_non_fungible(picture);
        "})
        .unwrap();

        let output = generate_transaction(input).unwrap();

        assert_code_eq(output, quote! {
            {
                let mut builder = TransactionBuilder::new();
                return builder;
            }
        });
    }

    fn assert_code_eq(a: TokenStream, b: TokenStream) {
        assert_eq!(a.to_string(), b.to_string());
    }
}
