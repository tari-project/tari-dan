//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use proc_macro2::{Ident, TokenStream};
use syn::{
    parse::ParseStream, parse2, punctuated::Punctuated, token::Comma, Block, Expr, ExprCall, ExprLit, ExprMacro,
    ExprMethodCall, ExprPath, Item, ItemFn, ItemUse, Lit, LitStr, Local, Macro, Pat, PatIdent, Path, Stmt, UseTree,
};
use tari_engine_types::TemplateAddress;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::args::LogLevel;

#[derive(Debug, Clone)]
pub enum ManifestIntent {
    DefineTemplate {
        template_address: TemplateAddress,
        alias: Ident,
    },
    InvokeTemplate(InvokeIntent),
    InvokeComponent(InvokeIntent),
    AssignInput(AssignInputStmt),
    Log(LogIntent),
}

#[derive(Debug, Clone)]
pub struct InvokeIntent {
    pub output_variable: Option<Ident>,
    pub component_variable: Option<Ident>,
    pub template_variable: Option<Ident>,
    pub function_name: Ident,
    pub arguments: Vec<ManifestLiteral>,
}

#[derive(Debug, Clone)]
pub struct AssignInputStmt {
    pub variable_name: Ident,
    pub global_variable_name: LitStr,
}

#[derive(Debug, Clone)]
pub struct LogIntent {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum ManifestLiteral {
    Lit(Lit),
    Variable(Ident),
    Special(SpecialLiteral),
}

#[derive(Debug, Clone)]
pub enum SpecialLiteral {
    Amount(i64),
    NonFungibleId(Lit),
}

pub struct ManifestParser;

impl ManifestParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse(&self, input: ParseStream) -> Result<Vec<ManifestIntent>, syn::Error> {
        let mut statements = vec![];
        statements.push(ManifestIntent::DefineTemplate {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            alias: Ident::new("Account", proc_macro2::Span::call_site()),
        });
        for stmt in Block::parse_within(input)? {
            match stmt {
                // use template_hash as TemplateName;
                Stmt::Item(Item::Use(ItemUse {
                    tree: UseTree::Rename(rename),
                    ..
                })) => {
                    let template_id = rename.ident.to_string();
                    let template_address = template_id
                        .split_once('_')
                        .and_then(|(_, s)| TemplateAddress::from_hex(s).ok())
                        .ok_or_else(|| syn::Error::new_spanned(rename.clone(), "Invalid template address"))?;

                    statements.push(ManifestIntent::DefineTemplate {
                        template_address,
                        alias: rename.rename,
                    });
                },
                Stmt::Item(Item::Fn(ItemFn { block, .. })) => {
                    statements.extend(self.parse_block(*block)?);
                },
                _ => {
                    return Err(syn::Error::new_spanned(
                        stmt.clone(),
                        format!("Unsupported outer statement {:?}", stmt),
                    ))
                },
            }
        }

        Ok(statements)
    }

    fn parse_block(&self, block: Block) -> Result<Vec<ManifestIntent>, syn::Error> {
        block.stmts.into_iter().map(|stmt| self.parse_stmt(stmt)).collect()
    }

    pub fn parse_stmt(&self, stmt: Stmt) -> Result<ManifestIntent, syn::Error> {
        match stmt {
            // use template_hash as TemplateName;
            Stmt::Item(Item::Use(ItemUse {
                tree: UseTree::Rename(rename),
                ..
            })) => {
                let template_id = rename.ident.to_string();
                let template_address = template_id
                    .split_once('_')
                    .and_then(|(_, s)| TemplateAddress::from_hex(s).ok())
                    .ok_or_else(|| syn::Error::new_spanned(rename.clone(), "Invalid template address"))?;

                Ok(ManifestIntent::DefineTemplate {
                    template_address,
                    alias: rename.rename,
                })
            },
            // let variable_name = TemplateName::function_name(arg1, arg2);
            Stmt::Local(local) => self.handle_local(local),
            // component.function_name(arg1, arg2);
            Stmt::Semi(expr, _) => self.handle_semi_expr(expr),
            _ => Err(syn::Error::new_spanned(
                stmt.clone(),
                format!("Invalid statement {:?}", stmt),
            )),
        }
    }

    fn handle_local(&self, local: Local) -> Result<ManifestIntent, syn::Error> {
        // Parse let variable ident
        let var_ident = match local.pat {
            Pat::Ident(PatIdent { ref ident, .. }) => ident,
            // Pat::Tuple(pat) => return Ok(ManifestStmt::Todo),
            // Pat::Type(_) => {}
            // Pat::Macro(_) => {}
            // Pat::Reference(_) => {}
            // Pat::Slice(_) => {}
            // Pat::Struct(_) => {}
            // Pat::TupleStruct(_) => {}
            p => unimplemented!("{:?} not supported", p),
        };

        let expr = local.init.as_ref().map(|(_, expr)| expr).ok_or_else(|| {
            syn::Error::new_spanned(
                local.clone(),
                // I think this is `let x;`?
                "let expressions without an assignment are unsupported",
            )
        })?;

        let result = match *expr.clone() {
            Expr::Call(call) => {
                let (template_ident, function_ident) = match &*call.func {
                    Expr::Path(path) => {
                        let mut iter = path.path.segments.iter();
                        let template_name = iter.next().ok_or_else(|| {
                            syn::Error::new_spanned(path, "Invalid template function call, no template name")
                        })?;

                        let function_name = iter.next().ok_or_else(|| {
                            syn::Error::new_spanned(path, "Invalid template function call, no function name")
                        })?;
                        (&template_name.ident, &function_name.ident)
                    },
                    _ => return Err(syn::Error::new_spanned(call.func, "Invalid function call")),
                };
                ManifestIntent::InvokeTemplate(InvokeIntent {
                    output_variable: Some(var_ident.clone()),
                    component_variable: None,
                    template_variable: Some(template_ident.clone()),
                    function_name: function_ident.clone(),
                    arguments: build_arguments(call.args)?,
                })
            },
            Expr::MethodCall(ExprMethodCall {
                receiver, method, args, ..
            }) => {
                let receiver = extract_single_var_name(&receiver)?;
                ManifestIntent::InvokeComponent(InvokeIntent {
                    output_variable: Some(var_ident.clone()),
                    component_variable: Some(receiver),
                    template_variable: None,
                    function_name: method,
                    arguments: build_arguments(args)?,
                })
            },
            Expr::Macro(ExprMacro {
                mac: Macro { path, tokens, .. },
                ..
            }) => {
                if path.segments.len() != 1 {
                    // TODO: improve error
                    return Err(syn::Error::new_spanned(path, "Invalid macro path"));
                }

                assignment_from_macro(var_ident.clone(), &path.segments[0].ident, tokens)?
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    expr.clone(),
                    format!("Only function calls are supported in let statements. {:?}", expr),
                ))
            },
        };

        Ok(result)
    }

    fn handle_semi_expr(&self, expr: Expr) -> Result<ManifestIntent, syn::Error> {
        match expr {
            Expr::Call(call) => {
                let (template_ident, function_ident) = match &*call.func {
                    Expr::Path(path) => {
                        let mut iter = path.path.segments.iter();
                        let template_name = iter.next().ok_or_else(|| {
                            syn::Error::new_spanned(path, "Invalid template function call, no template name")
                        })?;

                        let function_name = iter.next().ok_or_else(|| {
                            syn::Error::new_spanned(path, "Invalid template function call, no function name")
                        })?;
                        (&template_name.ident, &function_name.ident)
                    },
                    _ => return Err(syn::Error::new_spanned(call.func, "Invalid function call")),
                };
                Ok(ManifestIntent::InvokeTemplate(InvokeIntent {
                    output_variable: None,
                    component_variable: None,
                    template_variable: Some(template_ident.clone()),
                    function_name: function_ident.clone(),
                    arguments: build_arguments(call.args)?,
                }))
            },
            Expr::MethodCall(ExprMethodCall {
                receiver, method, args, ..
            }) => {
                let receiver = extract_single_var_name(&receiver)?;
                Ok(ManifestIntent::InvokeComponent(InvokeIntent {
                    output_variable: None,
                    component_variable: Some(receiver),
                    template_variable: None,
                    function_name: method,
                    arguments: build_arguments(args)?,
                }))
            },
            Expr::Macro(ExprMacro {
                mac: Macro { path, tokens, .. },
                ..
            }) => {
                if path.segments.len() != 1 {
                    // TODO: improve error
                    return Err(syn::Error::new_spanned(path, "Invalid macro path"));
                }

                let mac = &path.segments[0].ident;
                macro_call(mac, tokens)
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    expr.clone(),
                    format!("Only function calls are supported in let statements. {:?}", expr),
                ))
            },
        }
    }
}

fn assignment_from_macro(var_name: Ident, mac: &Ident, tokens: TokenStream) -> Result<ManifestIntent, syn::Error> {
    match mac.to_string().as_str() {
        "global" | "var" => Ok(ManifestIntent::AssignInput(AssignInputStmt {
            variable_name: var_name,
            global_variable_name: parse2(tokens)?,
        })),
        _ => Err(syn::Error::new_spanned(mac, "Invalid macro name")),
    }
}

fn macro_call(mac: &Ident, tokens: TokenStream) -> Result<ManifestIntent, syn::Error> {
    match mac.to_string().as_str() {
        "info" => Ok(ManifestIntent::Log(LogIntent {
            level: LogLevel::Info,
            // TODO: Support format args - of course, this requires runtime support so is quite a heavy lift.
            message: parse2::<LitStr>(tokens)?.value(),
        })),
        "debug" => Ok(ManifestIntent::Log(LogIntent {
            level: LogLevel::Debug,
            message: parse2::<LitStr>(tokens)?.value(),
        })),
        "warn" => Ok(ManifestIntent::Log(LogIntent {
            level: LogLevel::Warn,
            message: parse2::<LitStr>(tokens)?.value(),
        })),
        "error" => Ok(ManifestIntent::Log(LogIntent {
            level: LogLevel::Error,
            message: parse2::<LitStr>(tokens)?.value(),
        })),
        _ => Err(syn::Error::new_spanned(mac, "Invalid macro name")),
    }
}

fn build_arguments(args: Punctuated<Expr, Comma>) -> Result<Vec<ManifestLiteral>, syn::Error> {
    args.into_iter()
        .map(|arg| match arg {
            Expr::Lit(lit) => Ok(ManifestLiteral::Lit(lit.lit)),

            Expr::Path(expr_path) => {
                if expr_path.path.segments.len() == 1 {
                    Ok(ManifestLiteral::Variable(expr_path.path.segments[0].ident.clone()))
                } else {
                    Err(syn::Error::new_spanned(
                        expr_path,
                        "Invalid path, only single segment paths are supported",
                    ))
                }
            },
            // Support for Amount(100) syntax
            Expr::Call(ExprCall { func, args, .. }) => {
                if let Expr::Path(ExprPath {
                    path: Path { segments, .. },
                    ..
                }) = &*func
                {
                    let name = segments
                        .first()
                        .ok_or_else(|| syn::Error::new_spanned(func.clone(), "Invalid function call"))?;

                    handle_special_literals(&name.ident, args)
                } else {
                    Err(syn::Error::new_spanned(
                        func,
                        "Invalid function call, only Amount is supported",
                    ))
                }
            },
            _ => Err(syn::Error::new_spanned(
                arg,
                "Invalid argument, only literals and variables are supported",
            )),
        })
        .collect()
}

fn handle_special_literals(name: &Ident, args: Punctuated<Expr, Comma>) -> Result<ManifestLiteral, syn::Error> {
    if name == "Amount" {
        let amt = args
            .first()
            .ok_or_else(|| syn::Error::new_spanned(name, "Invalid function call"))?;
        match amt {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => {
                Ok(ManifestLiteral::Special(SpecialLiteral::Amount(lit.base10_parse()?)))
            },
            _ => Err(syn::Error::new_spanned(
                amt,
                "Invalid argument, only literals and variables are supported",
            )),
        }
    } else if name == "NonFungibleId" {
        let arg = args
            .first()
            .ok_or_else(|| syn::Error::new_spanned(name, "Invalid function call"))?;
        if let Expr::Lit(ExprLit { lit, .. }) = arg {
            Ok(ManifestLiteral::Special(SpecialLiteral::NonFungibleId(lit.clone())))
        } else {
            Err(syn::Error::new_spanned(
                arg,
                "Invalid argument, only literals and variables are supported",
            ))
        }
    } else {
        Err(syn::Error::new_spanned(
            name,
            "Invalid function call, only Amount is supported",
        ))
    }
}

fn extract_single_var_name(expr: &Expr) -> Result<Ident, syn::Error> {
    match expr {
        Expr::Path(ExprPath {
            path: Path { segments, .. },
            ..
        }) => {
            if segments.len() != 1 {
                return Err(syn::Error::new_spanned(expr, "Invalid method call"));
            }
            Ok(segments[0].ident.clone())
        },
        _ => Err(syn::Error::new_spanned(
            expr.clone(),
            format!("Invalid method call {:?}", expr),
        )),
    }
}
