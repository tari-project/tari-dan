//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::collections::HashMap;

use proc_macro2::{Ident, TokenStream};
use syn::{
    Block,
    Expr,
    ExprArray,
    ExprCall,
    ExprLit,
    ExprMacro,
    ExprMethodCall,
    ExprPath,
    ExprUnary,
    Item,
    ItemFn,
    ItemUse,
    Lit,
    LitStr,
    Local,
    Macro,
    Pat,
    PatIdent,
    PatTuple,
    Path,
    Signature,
    Stmt,
    UnOp,
    UseTree,
    ext::IdentExt,
    parse::{ParseStream, Parser},
    parse2,
    punctuated::Punctuated,
    token::Comma,
};
use tari_engine_types::substate::SubstateId;
use tari_ootle_transaction::AllocatableAddressType;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::{
    Amount,
    Metadata,
    NonFungibleId,
    TemplateAddress,
    constants::TARI_TOKEN,
    hex::bytes_from_hex,
};

use crate::{
    cbor_literal::{cbor_from_macro, metadata_from_macro, negative_int_value},
    error::ManifestError,
    value::MAX_LIST_DEPTH,
};

#[derive(Debug, Clone)]
pub enum ManifestIntent {
    InvokeTemplate(InvokeIntent),
    InvokeComponent(InvokeIntent),
    AssignInput(AssignInputStmt),
    AssignBlob(AssignBlobStmt),
    AllocateAddress(AllocateAddressStmt),
    CreateAccount(CreateAccountIntent),
    DropAllProofs,
    PublishTemplate(PublishTemplateIntent),
    PutIntoBucket(PutIntoBucketIntent),
    CallLocalFunction(Ident),
}

#[derive(Debug, Clone)]
pub struct PutIntoBucketIntent {
    pub src: Ident,
    pub dest: Ident,
}

/// `publish_template!(name)` — publishes the WASM binary referenced by `name`, which is either a
/// blob variable bound with `let name = blob!("...")` or a blob name supplied via `blob_inputs`.
#[derive(Debug, Clone)]
pub struct PublishTemplateIntent {
    /// A blob variable (from `let x = blob!(...)`) or a blob name. Either way it resolves to a blob
    /// provided via `blob_inputs`.
    pub blob_name: Ident,
    /// Optional off-chain CBOR metadata multihash (hex-encoded literal).
    pub metadata_hash_hex: Option<String>,
}

/// `let x = blob!("name")` — binds the variable `x` to the blob named `name` (supplied via
/// `blob_inputs`) so it can be referenced by name later, e.g. `publish_template!(x)`.
#[derive(Debug, Clone)]
pub struct AssignBlobStmt {
    pub variable_name: Ident,
    pub blob_name: Ident,
}

#[derive(Debug, Clone)]
pub struct ManifestImport {
    pub template_address: Option<TemplateAddress>,
    pub alias: Ident,
}

#[derive(Debug, Clone)]
pub enum OutputBinding {
    Single(Ident),
    Tuple(Vec<Ident>),
}

#[derive(Debug, Clone)]
pub struct InvokeIntent {
    pub output_variable: Option<OutputBinding>,
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
pub struct AllocateAddressStmt {
    pub output_variable: Ident,
    pub allocatable_type: AllocatableAddressType,
}

#[derive(Debug, Clone)]
pub struct CreateAccountIntent {
    pub output_variable: Option<Ident>,
    pub owner_public_key: Ident,
    pub owner_rule: Option<Ident>,
    pub access_rules: Option<Ident>,
    pub bucket: Option<Ident>,
}

#[derive(Debug, Clone)]
pub enum ManifestLiteral {
    Lit(Lit),
    Workspace(Ident),
    /// A reference to a transaction blob by caller-supplied name. The generator resolves the
    /// name to a `BlobIndex` against the blobs map passed to `parse_manifest`.
    Blob(Ident),
    /// An input variable named by `var!`/`arg!`/`global!`, resolved against the globals passed to
    /// `parse_manifest`.
    Global(LitStr),
    /// A list of arguments, passed as one CBOR array. Every element must be known when the manifest
    /// is generated, so a workspace value or a blob cannot be an element.
    Array(Vec<ManifestLiteral>),
    Special(SpecialLiteral),
}

#[derive(Debug, Clone)]
pub enum SpecialLiteral {
    Null,
    Amount(Amount),
    NonFungibleId(NonFungibleId),
    Cbor(tari_bor::Value),
    Metadata(Metadata),
    SubstateId(OrVar<SubstateId>),
    Address(OrVar<SubstateId>),
}

#[derive(Debug, Clone)]
pub enum OrVar<T> {
    Var(Ident),
    Value(T),
}

pub struct ManifestParser;

#[derive(Debug, Clone)]
pub struct ParsedManifest {
    pub defines: Vec<ManifestImport>,
    pub instruction_intents: Vec<ManifestIntent>,
    pub fee_instruction_intents: Vec<ManifestIntent>,
    pub functions: HashMap<String, Vec<ManifestIntent>>,
}

impl ManifestParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse(&self, input: ParseStream) -> Result<ParsedManifest, syn::Error> {
        let mut instruction_intents = vec![];
        let mut fee_instruction_intents = vec![];
        let mut functions = HashMap::new();
        let mut defines = vec![];
        defines.push(ManifestImport {
            template_address: Some(ACCOUNT_TEMPLATE_ADDRESS),
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

                    defines.push(ManifestImport {
                        template_address: Some(template_address),
                        alias: rename.rename,
                    });
                },
                // use Name; // (predefined template)
                Stmt::Item(Item::Use(ItemUse {
                    tree: UseTree::Name(name),
                    ..
                })) => {
                    defines.push(ManifestImport {
                        template_address: None,
                        alias: name.ident,
                    });
                },
                Stmt::Item(Item::Fn(ItemFn {
                    block,
                    sig: Signature { ident, .. },
                    ..
                })) => {
                    if ident == "fee_main" {
                        fee_instruction_intents.extend(self.parse_block(*block)?);
                    } else if ident == "main" {
                        instruction_intents.extend(self.parse_block(*block)?);
                    } else {
                        let name = ident.to_string();
                        let body = self.parse_block(*block)?;
                        functions.insert(name, body);
                    }
                },
                _ => {
                    return Err(syn::Error::new_spanned(
                        stmt.clone(),
                        format!("Unsupported outer statement {:?}", stmt),
                    ));
                },
            }
        }

        Ok(ParsedManifest {
            defines,
            instruction_intents,
            fee_instruction_intents,
            functions,
        })
    }

    fn parse_block(&self, block: Block) -> Result<Vec<ManifestIntent>, syn::Error> {
        block.stmts.into_iter().map(|stmt| self.parse_stmt(stmt)).collect()
    }

    pub fn parse_stmt(&self, stmt: Stmt) -> Result<ManifestIntent, syn::Error> {
        match stmt {
            Stmt::Local(local) => self.handle_local(local),
            // component.function_name(arg1, arg2);
            Stmt::Expr(expr, _) => self.handle_semi_expr(expr),
            Stmt::Macro(mac) => self.handle_macro_stmt(mac),
            _ => Err(syn::Error::new_spanned(
                stmt.clone(),
                format!("Invalid statement {:?}", stmt),
            )),
        }
    }

    fn handle_macro_stmt(&self, mac: syn::StmtMacro) -> Result<ManifestIntent, syn::Error> {
        let Macro { path, tokens, .. } = mac.mac;

        let Some(mac_ident) = path.segments.first() else {
            return Err(syn::Error::new_spanned(path, "macro path must have a single segment"));
        };

        macro_call(&mac_ident.ident, tokens)
    }

    fn handle_local(&self, local: Local) -> Result<ManifestIntent, syn::Error> {
        // Parse let variable binding
        let output_binding = match &local.pat {
            Pat::Ident(PatIdent { ident, .. }) => OutputBinding::Single(ident.clone()),
            Pat::Tuple(PatTuple { elems, .. }) => {
                let idents = elems
                    .iter()
                    .map(|p| match p {
                        Pat::Ident(PatIdent { ident, .. }) => Ok(ident.clone()),
                        _ => Err(syn::Error::new_spanned(
                            p,
                            "Only identifiers are supported in tuple destructuring patterns",
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if idents.is_empty() {
                    return Err(syn::Error::new_spanned(
                        &local.pat,
                        "Empty tuple pattern is not supported",
                    ));
                }
                OutputBinding::Tuple(idents)
            },
            p => return Err(syn::Error::new_spanned(p, format!("{:?} pattern is not supported", p))),
        };

        let expr = local.init.as_ref().map(|init| &init.expr).ok_or_else(|| {
            syn::Error::new_spanned(
                local.clone(),
                // I think this is `let x;`?
                "let expressions without an assignment are unsupported",
            )
        })?;

        let result = match *expr.clone() {
            Expr::Call(call) => {
                match &*call.func {
                    Expr::Path(path) => {
                        let mut iter = path.path.segments.iter();
                        let first = iter
                            .next()
                            .ok_or_else(|| syn::Error::new_spanned(path, "Invalid function call, empty path"))?;

                        // Two segments: Template::function(). A local function call has one.
                        let Some(second) = iter.next() else {
                            return Err(syn::Error::new_spanned(
                                path,
                                "a local function call has no value to bind",
                            ));
                        };
                        ManifestIntent::InvokeTemplate(InvokeIntent {
                            output_variable: Some(output_binding),
                            component_variable: None,
                            template_variable: Some(first.ident.clone()),
                            function_name: second.ident.clone(),
                            arguments: build_arguments(call.args)?,
                        })
                    },
                    _ => return Err(syn::Error::new_spanned(call.func, "Invalid function call")),
                }
            },
            Expr::MethodCall(ExprMethodCall {
                receiver, method, args, ..
            }) => {
                let receiver = extract_single_var_name(&receiver)?;
                ManifestIntent::InvokeComponent(InvokeIntent {
                    output_variable: Some(output_binding),
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
                    return Err(syn::Error::new_spanned(path, "macro path must have a single segment"));
                }

                let var_ident = match output_binding {
                    OutputBinding::Single(ident) => ident,
                    OutputBinding::Tuple(_) => {
                        return Err(syn::Error::new_spanned(
                            &local.pat,
                            "Tuple destructuring is not supported for macro assignments",
                        ));
                    },
                };

                assignment_from_macro(
                    var_ident,
                    &path
                        .segments
                        .first()
                        .expect("macro path must have a single segment")
                        .ident,
                    tokens,
                )?
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    expr.clone(),
                    format!("Only function calls are supported in let statements. {:?}", expr),
                ));
            },
        };

        Ok(result)
    }

    fn handle_semi_expr(&self, expr: Expr) -> Result<ManifestIntent, syn::Error> {
        match expr {
            Expr::Call(call) => {
                match &*call.func {
                    Expr::Path(path) => {
                        let mut iter = path.path.segments.iter();
                        let first = iter
                            .next()
                            .ok_or_else(|| syn::Error::new_spanned(path, "Invalid function call, empty path"))?;

                        if let Some(second) = iter.next() {
                            // Two segments: Template::function()
                            Ok(ManifestIntent::InvokeTemplate(InvokeIntent {
                                output_variable: None,
                                component_variable: None,
                                template_variable: Some(first.ident.clone()),
                                function_name: second.ident.clone(),
                                arguments: build_arguments(call.args)?,
                            }))
                        } else {
                            // Single segment: local function call
                            Ok(ManifestIntent::CallLocalFunction(first.ident.clone()))
                        }
                    },
                    _ => Err(syn::Error::new_spanned(call.func, "Invalid function call")),
                }
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
                let Some(mac) = path.segments.first() else {
                    return Err(syn::Error::new_spanned(path, "macro path must have a single segment"));
                };

                macro_call(&mac.ident, tokens)
            },
            _ => Err(syn::Error::new_spanned(
                expr.clone(),
                format!("Only function calls are supported in let statements. {:?}", expr),
            )),
        }
    }
}

fn assignment_from_macro(var_name: Ident, mac: &Ident, tokens: TokenStream) -> Result<ManifestIntent, syn::Error> {
    match mac.to_string().as_str() {
        "global" | "var" | "arg" => Ok(ManifestIntent::AssignInput(AssignInputStmt {
            variable_name: var_name,
            global_variable_name: parse2(tokens)?,
        })),
        "blob" => {
            // Accept `blob!("name")` or `blob!(name)`, mirroring the argument-position `blob!`.
            Ok(ManifestIntent::AssignBlob(AssignBlobStmt {
                variable_name: var_name,
                blob_name: parse_blob_name(mac, tokens)?,
            }))
        },
        "new_component_addr" => Ok(ManifestIntent::AllocateAddress(AllocateAddressStmt {
            output_variable: var_name,
            allocatable_type: AllocatableAddressType::Component,
        })),
        "new_resource_addr" => Ok(ManifestIntent::AllocateAddress(AllocateAddressStmt {
            output_variable: var_name,
            allocatable_type: AllocatableAddressType::Resource,
        })),
        "create_account" => {
            let args = parse_create_account_args(tokens)?;
            Ok(ManifestIntent::CreateAccount(CreateAccountIntent {
                output_variable: Some(var_name),
                owner_public_key: args.owner_public_key,
                owner_rule: args.owner_rule,
                access_rules: args.access_rules,
                bucket: args.bucket,
            }))
        },
        _ => Err(syn::Error::new_spanned(mac, "Invalid macro name")),
    }
}

fn macro_call(mac: &Ident, tokens: TokenStream) -> Result<ManifestIntent, syn::Error> {
    match mac.to_string().as_str() {
        "drop_all_proofs" => Ok(ManifestIntent::DropAllProofs),
        "put_into_bucket" => {
            let args = syn::parse::Parser::parse2(
                |input: ParseStream| {
                    let src: Ident = input.parse()?;
                    input.parse::<syn::Token![,]>()?;
                    let dest: Ident = input.parse()?;
                    Ok(PutIntoBucketIntent { src, dest })
                },
                tokens,
            )?;
            Ok(ManifestIntent::PutIntoBucket(args))
        },
        "create_account" => {
            let args = parse_create_account_args(tokens)?;
            Ok(ManifestIntent::CreateAccount(CreateAccountIntent {
                output_variable: None,
                owner_public_key: args.owner_public_key,
                owner_rule: args.owner_rule,
                access_rules: args.access_rules,
                bucket: args.bucket,
            }))
        },
        "publish_template" => parse_publish_template_args(tokens),
        _ => Err(syn::Error::new_spanned(mac, "Invalid macro name")),
    }
}

/// Parse the name in `blob!(name)` or `blob!("name")`. A string literal must hold a single identifier because the
/// name is carried as an `Ident` from here on.
fn parse_blob_name(mac: &Ident, tokens: TokenStream) -> Result<Ident, syn::Error> {
    if let Ok(lit_str) = parse2::<LitStr>(tokens.clone()) {
        let value = lit_str.value();
        // Tokenizing drops surrounding whitespace and comments, so the parsed ident must match the literal exactly.
        return match Parser::parse_str(Ident::parse_any, &value) {
            Ok(mut ident) if ident == value => {
                ident.set_span(lit_str.span());
                Ok(ident)
            },
            _ => Err(syn::Error::new_spanned(
                &lit_str,
                format!("blob! name {value:?} is not a valid identifier"),
            )),
        };
    }
    parse2::<Ident>(tokens)
        .map_err(|e| syn::Error::new_spanned(mac, format!("Expected identifier or string literal in blob!: {}", e)))
}

/// Parse `publish_template!(blob_name)` or `publish_template!(blob_name, metadata = "0x...")`.
/// The blob name must have a payload supplied via `parse_manifest`'s `blob_inputs` map.
fn parse_publish_template_args(tokens: TokenStream) -> Result<ManifestIntent, syn::Error> {
    syn::parse::Parser::parse2(
        |input: ParseStream| {
            let blob_name: Ident = input.parse()?;
            let mut metadata_hash_hex = None;

            while input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
                if input.is_empty() {
                    break;
                }
                let key: Ident = input.parse()?;
                input.parse::<syn::Token![=]>()?;
                match key.to_string().as_str() {
                    "metadata" => {
                        let value: LitStr = input.parse()?;
                        metadata_hash_hex = Some(value.value());
                    },
                    _ => {
                        return Err(syn::Error::new_spanned(
                            key,
                            "Unknown publish_template argument, expected 'metadata'",
                        ));
                    },
                }
            }

            Ok(ManifestIntent::PublishTemplate(PublishTemplateIntent {
                blob_name,
                metadata_hash_hex,
            }))
        },
        tokens,
    )
}

fn build_arguments(args: Punctuated<Expr, Comma>) -> Result<Vec<ManifestLiteral>, syn::Error> {
    args.into_iter().map(|arg| build_argument(arg, 0)).collect()
}

fn build_argument(arg: Expr, depth: usize) -> Result<ManifestLiteral, syn::Error> {
    match arg {
        Expr::Lit(lit) => Ok(ManifestLiteral::Lit(lit.lit)),

        Expr::Path(ExprPath { path, .. }) => {
            if let Some(seg) = path.segments.first() {
                if seg.ident == "None" {
                    Ok(ManifestLiteral::Special(SpecialLiteral::Null))
                } else if seg.ident == "XTR" || seg.ident == "TARI" {
                    Ok(ManifestLiteral::Special(SpecialLiteral::Address(OrVar::Value(
                        TARI_TOKEN.into(),
                    ))))
                } else {
                    Ok(ManifestLiteral::Workspace(seg.ident.clone()))
                }
            } else {
                Err(syn::Error::new_spanned(
                    path,
                    "Invalid path, only single segment paths are supported",
                ))
            }
        },
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_), expr, ..
        }) => match *expr {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok(ManifestLiteral::Special(SpecialLiteral::Cbor(
                negative_int_value(&lit)?,
            ))),
            expr => Err(syn::Error::new_spanned(expr, "expected an integer after `-`")),
        },
        Expr::Call(ExprCall { func, .. }) => Err(syn::Error::new_spanned(&func, match macro_for_call(&func) {
            Some(mac) => format!("function-call literals are not supported, use `{mac}!(..)` instead"),
            None => "function calls are not supported as arguments".to_string(),
        })),
        Expr::Macro(ExprMacro { mac, .. }) => Ok(handle_macro_argument(mac)?),
        Expr::Array(ExprArray { elems, .. }) => {
            if depth >= MAX_LIST_DEPTH {
                return Err(syn::Error::new_spanned(
                    elems,
                    format!("lists nest at most {MAX_LIST_DEPTH} levels deep"),
                ));
            }
            elems
                .into_iter()
                .map(|elem| build_argument(elem, depth + 1))
                .collect::<Result<_, _>>()
                .map(ManifestLiteral::Array)
        },
        _ => Err(syn::Error::new_spanned(
            arg,
            "Invalid argument, only literals, lists and variables are supported",
        )),
    }
}

/// The argument macro for a function-call literal named after the macro's type, e.g. `amount!` for
/// `Amount(..)`, so that the error can point at it.
fn macro_for_call(func: &Expr) -> Option<&'static str> {
    let Expr::Path(ExprPath { path, .. }) = func else {
        return None;
    };
    let name = path.get_ident()?.to_string();
    Some(match name.as_str() {
        "Amount" => "amount",
        "Address" => "address",
        "SubstateId" => "substate_id",
        "NonFungibleId" => "non_fungible_id",
        "Metadata" => "metadata",
        "HexBytes" => "hex_bytes",
        "PublicKey" => "public_key",
        "Cbor" => "cbor",
        _ => return None,
    })
}

pub(crate) fn handle_macro_argument(mac: Macro) -> Result<ManifestLiteral, syn::Error> {
    let name = mac
        .path
        .get_ident()
        .ok_or_else(|| syn::Error::new_spanned(&mac.path, "macro path must have a single segment"))?;

    match name.to_string().as_str() {
        "global" | "var" | "arg" => Ok(ManifestLiteral::Global(parse2(mac.tokens)?)),
        "cbor" => Ok(ManifestLiteral::Special(SpecialLiteral::Cbor(cbor_from_macro(&mac)?))),
        "metadata" => Ok(ManifestLiteral::Special(SpecialLiteral::Metadata(metadata_from_macro(
            &mac,
        )?))),
        "amount" => {
            let lit: syn::LitInt = parse2(mac.tokens)
                .map_err(|e| syn::Error::new_spanned(name, format!("Expected integer literal in amount!: {}", e)))?;
            Ok(ManifestLiteral::Special(SpecialLiteral::Amount(lit.base10_parse()?)))
        },
        "hex_bytes" | "public_key" | "bytes" => {
            let lit_str: LitStr = parse2(mac.tokens)
                .map_err(|e| syn::Error::new_spanned(name, format!("Expected string literal in {}!: {}", name, e)))?;
            let bytes = bytes_from_hex(&lit_str.value())
                .map_err(|e| syn::Error::new_spanned(&lit_str, format!("Failed to parse hex bytes: {}", e)))?;
            Ok(ManifestLiteral::Special(SpecialLiteral::Cbor(tari_bor::Value::Bytes(
                bytes,
            ))))
        },
        "address" | "substate_id" => {
            if let Ok(lit_str) = parse2::<LitStr>(mac.tokens.clone()) {
                let id: SubstateId = lit_str
                    .value()
                    .parse()
                    .map_err(|e| syn::Error::new_spanned(&lit_str, format!("Failed to parse substate id: {}", e)))?;
                if name == "address" {
                    Ok(ManifestLiteral::Special(SpecialLiteral::Address(OrVar::Value(id))))
                } else {
                    Ok(ManifestLiteral::Special(SpecialLiteral::SubstateId(OrVar::Value(id))))
                }
            } else {
                let ident: Ident = parse2(mac.tokens).map_err(|e| {
                    syn::Error::new_spanned(
                        name,
                        format!("Expected identifier or string literal in {}!: {}", name, e),
                    )
                })?;
                if name == "address" {
                    Ok(ManifestLiteral::Special(SpecialLiteral::Address(OrVar::Var(ident))))
                } else {
                    Ok(ManifestLiteral::Special(SpecialLiteral::SubstateId(OrVar::Var(ident))))
                }
            }
        },
        "non_fungible_id" => {
            let lit: Lit = parse2(mac.tokens)
                .map_err(|e| syn::Error::new_spanned(name, format!("Expected literal in non_fungible_id!: {}", e)))?;
            let id = lit_to_nonfungible_id(&lit)
                .map_err(|e| syn::Error::new_spanned(&lit, format!("Failed to parse NonFungibleId: {}", e)))?;
            Ok(ManifestLiteral::Special(SpecialLiteral::NonFungibleId(id)))
        },
        "blob" => {
            // Accept either an identifier (`blob!(my_data)`) or a string literal
            // (`blob!("my_data")`). Both resolve the same name against the blobs map supplied
            // to `parse_manifest`.
            Ok(ManifestLiteral::Blob(parse_blob_name(name, mac.tokens)?))
        },
        _ => Err(syn::Error::new_spanned(
            name,
            format!(
                "Invalid argument macro '{name}', supported macros: var!, arg!, global!, cbor!, metadata!, amount!, \
                 hex_bytes!, bytes!, public_key!, address!, substate_id!, non_fungible_id!, blob!"
            ),
        )),
    }
}

struct CreateAccountArgs {
    owner_public_key: Ident,
    owner_rule: Option<Ident>,
    access_rules: Option<Ident>,
    bucket: Option<Ident>,
}

fn parse_create_account_args(tokens: TokenStream) -> Result<CreateAccountArgs, syn::Error> {
    syn::parse::Parser::parse2(
        |input: ParseStream| {
            let owner_public_key: Ident = input.parse()?;
            let mut owner_rule = None;
            let mut access_rules = None;
            let mut bucket = None;

            while input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
                if input.is_empty() {
                    break;
                }
                let key: Ident = input.parse()?;
                input.parse::<syn::Token![=]>()?;
                let value: Ident = input.parse()?;

                match key.to_string().as_str() {
                    "owner_rule" => owner_rule = Some(value),
                    "access_rules" => access_rules = Some(value),
                    "bucket" => bucket = Some(value),
                    _ => {
                        return Err(syn::Error::new_spanned(
                            key,
                            "Unknown create_account argument, expected 'owner_rule', 'access_rules', or 'bucket'",
                        ));
                    },
                }
            }

            Ok(CreateAccountArgs {
                owner_public_key,
                owner_rule,
                access_rules,
                bucket,
            })
        },
        tokens,
    )
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

fn lit_to_nonfungible_id(lit: &Lit) -> Result<NonFungibleId, ManifestError> {
    match lit {
        Lit::Str(s) => Ok(NonFungibleId::try_from_string(s.value()).map_err(|e| {
            ManifestError::UnsupportedExpr(format!(
                "Invalid non-fungible ID string literal ({:?}) ({})",
                e,
                s.value()
            ))
        })?),
        Lit::ByteStr(v) => {
            let bytes = v.value();
            if bytes.len() != 32 {
                return Err(ManifestError::UnsupportedExpr(
                    "Non-fungible ID byte string literal length must be less than 32 bytes".to_string(),
                ));
            }

            let mut id = [0u8; 32];
            id.copy_from_slice(&bytes);
            Ok(NonFungibleId::from_u256(id))
        },
        Lit::Int(v) => match v.suffix() {
            "u8" | "u16" | "u32" => Ok(NonFungibleId::from_u32(v.base10_parse()?)),
            "u64" => Ok(NonFungibleId::from_u64(v.base10_parse()?)),
            "" => Err(ManifestError::UnsupportedExpr(
                "Non-fungible ID integer literal must have a type suffix specified (1u32, 2u64 etc)".to_string(),
            )),
            _ => Err(ManifestError::UnsupportedExpr(format!(
                "Invalid non-fungible ID integer literal suffix ({})",
                v.suffix()
            ))),
        },
        _ => Err(ManifestError::UnsupportedExpr(format!(
            "Unsupported non-fungible ID literal ({:?})",
            lit
        ))),
    }
}
