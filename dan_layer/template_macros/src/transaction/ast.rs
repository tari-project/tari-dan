use syn::{
    parse::{Parse, ParseStream},
    Block,
    Result,
    Stmt,
};

pub struct TransactionAst {
    pub stmts: Vec<Stmt>,
}

impl Parse for TransactionAst {
    fn parse(input: ParseStream) -> Result<Self> {
        let stmts = Block::parse_within(input)?;

        Ok(Self { stmts })
    }
}
