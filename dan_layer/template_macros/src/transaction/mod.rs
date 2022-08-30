use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, Result};

use self::ast::TransactionAst;

mod ast;
mod builder;

pub fn generate_transaction(input: TokenStream) -> Result<TokenStream> {
    let ast = parse2::<TransactionAst>(input).unwrap();
    let stmts = ast.stmts;

    let output = quote! {
        #(#stmts)*
    };

    Ok(output)
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
            let price = Amount(1_000);
            let mut picture_seller = PictureSeller::new(price);

            let mut account = Account::new();
            account.add_fungible(ThaumFaucet::take(price));

            let payment: Bucket<Thaum> = account.take_fungible(price);
            let (picture, _) = picture_seller.buy(payment).unwrap();

            account.add_non_fungible(picture);
        });
    }

    fn assert_code_eq(a: TokenStream, b: TokenStream) {
        assert_eq!(a.to_string(), b.to_string());
    }
}
