//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::collections::HashMap;

use proc_macro2::{Group, Ident, Span, TokenStream};
use syn::{
    Expr,
    ExprArray,
    ExprCall,
    ExprLit,
    ExprMacro,
    ExprMethodCall,
    ExprPath,
    ExprUnary,
    Lit,
    LitStr,
    Macro,
    MacroDelimiter,
    Path,
    Token,
    UnOp,
    braced,
    bracketed,
    ext::IdentExt,
    parenthesized,
    parse::{ParseStream, Parser},
    parse2,
    punctuated::Punctuated,
    token,
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

        while !input.is_empty() {
            skip_attributes(input)?;
            if input.peek(Token![use]) {
                defines.push(parse_use(input)?);
            } else if input.peek(Token![fn]) {
                input.parse::<Token![fn]>()?;
                let ident: Ident = input.parse()?;
                let params;
                parenthesized!(params in input);
                if !params.is_empty() {
                    return Err(params.error("manifest functions take no parameters"));
                }
                let body;
                braced!(body in input);
                let intents = self.parse_block(&body)?;
                if ident == "fee_main" {
                    fee_instruction_intents.extend(intents);
                } else if ident == "main" {
                    instruction_intents.extend(intents);
                } else {
                    functions.insert(ident.to_string(), intents);
                }
            } else {
                return Err(input.error("expected a `use` declaration or a function"));
            }
        }

        Ok(ParsedManifest {
            defines,
            instruction_intents,
            fee_instruction_intents,
            functions,
        })
    }

    fn parse_block(&self, input: ParseStream) -> Result<Vec<ManifestIntent>, syn::Error> {
        let mut intents = vec![];
        while !input.is_empty() {
            skip_attributes(input)?;
            if input.is_empty() {
                break;
            }
            intents.push(self.parse_stmt(input)?);
        }
        Ok(intents)
    }

    fn parse_stmt(&self, input: ParseStream) -> Result<ManifestIntent, syn::Error> {
        if input.peek(Token![let]) {
            input.parse::<Token![let]>()?;
            let (binding, binding_span) = parse_binding(input)?;
            input.parse::<Token![=]>()?;
            let expr = parse_expr(input)?;
            input.parse::<Token![;]>()?;
            return self.handle_local(binding, binding_span, expr);
        }

        let expr = parse_expr(input)?;
        let is_brace_macro = matches!(&expr, Expr::Macro(m) if matches!(m.mac.delimiter, MacroDelimiter::Brace(_)));
        if input.peek(Token![;]) {
            input.parse::<Token![;]>()?;
        } else if !input.is_empty() && !is_brace_macro {
            return Err(input.error("expected `;`"));
        } else {
            // The last statement of a block, or a brace-delimited macro, ends without a `;`.
        }
        self.handle_semi_expr(expr)
    }

    fn handle_local(
        &self,
        output_binding: OutputBinding,
        binding_span: Span,
        expr: Expr,
    ) -> Result<ManifestIntent, syn::Error> {
        let result = match expr {
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
                        return Err(syn::Error::new(
                            binding_span,
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
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Only function calls are supported in let statements",
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

// The parsers below accept the manifest grammar only. Each consumes a token before it loops, and only a list
// argument recurses, so the parse depth is bounded by the group depth that `parse_manifest` caps.

/// Skips doc comments and other attributes, which carry no meaning in a manifest.
fn skip_attributes(input: ParseStream) -> Result<(), syn::Error> {
    while input.peek(Token![#]) {
        input.parse::<Token![#]>()?;
        if input.peek(Token![!]) {
            input.parse::<Token![!]>()?;
        }
        if !input.peek(token::Bracket) {
            return Err(input.error("expected `[` to open an attribute"));
        }
        input.parse::<Group>()?;
    }
    Ok(())
}

/// `use template_<hex> as Alias;` or `use Name;` for a predefined template.
fn parse_use(input: ParseStream) -> Result<ManifestImport, syn::Error> {
    input.parse::<Token![use]>()?;
    let ident: Ident = input.parse()?;
    let import = if input.peek(Token![as]) {
        input.parse::<Token![as]>()?;
        let alias: Ident = input.parse()?;
        let template_address = ident
            .to_string()
            .split_once('_')
            .and_then(|(_, s)| TemplateAddress::from_hex(s).ok())
            .ok_or_else(|| syn::Error::new_spanned(&ident, "Invalid template address"))?;
        ManifestImport {
            template_address: Some(template_address),
            alias,
        }
    } else {
        ManifestImport {
            template_address: None,
            alias: ident,
        }
    };
    input.parse::<Token![;]>()?;
    Ok(import)
}

/// The pattern of a `let`: `x`, `mut x` or a tuple of those.
fn parse_binding(input: ParseStream) -> Result<(OutputBinding, Span), syn::Error> {
    fn binding_ident(input: ParseStream) -> Result<Ident, syn::Error> {
        input.parse::<Option<Token![mut]>>()?;
        input.parse()
    }

    if input.peek(token::Paren) {
        let content;
        let paren = parenthesized!(content in input);
        let span = paren.span.join();
        let idents = Punctuated::<Ident, Comma>::parse_terminated_with(&content, binding_ident)?;
        if idents.is_empty() {
            return Err(syn::Error::new(span, "Empty tuple pattern is not supported"));
        }
        return Ok((OutputBinding::Tuple(idents.into_iter().collect()), span));
    }

    let ident = binding_ident(input)?;
    let span = ident.span();
    Ok((OutputBinding::Single(ident), span))
}

/// A statement's expression: `Template::function(args)`, `local_function()`, `component.method(args)` or a macro.
fn parse_expr(input: ParseStream) -> Result<Expr, syn::Error> {
    if input.peek(syn::Ident) && input.peek2(Token![!]) {
        return Ok(Expr::Macro(ExprMacro {
            attrs: vec![],
            mac: input.parse()?,
        }));
    }

    let path = input.call(Path::parse_mod_style)?;
    if input.peek(Token![.]) {
        let dot_token = input.parse()?;
        let method = input.parse()?;
        let content;
        let paren_token = parenthesized!(content in input);
        return Ok(Expr::MethodCall(ExprMethodCall {
            attrs: vec![],
            receiver: Box::new(path_expr(path)),
            dot_token,
            method,
            turbofish: None,
            paren_token,
            args: Punctuated::parse_terminated_with(&content, parse_arg)?,
        }));
    }
    if input.peek(token::Paren) {
        let content;
        let paren_token = parenthesized!(content in input);
        return Ok(Expr::Call(ExprCall {
            attrs: vec![],
            func: Box::new(path_expr(path)),
            paren_token,
            args: Punctuated::parse_terminated_with(&content, parse_arg)?,
        }));
    }
    Err(syn::Error::new_spanned(
        path,
        "expected a function call, a method call or a macro",
    ))
}

/// A call argument: a literal, a negated integer, a variable, a macro or a list of arguments.
fn parse_arg(input: ParseStream) -> Result<Expr, syn::Error> {
    if input.peek(token::Bracket) {
        let content;
        let bracket_token = bracketed!(content in input);
        return Ok(Expr::Array(ExprArray {
            attrs: vec![],
            bracket_token,
            elems: Punctuated::parse_terminated_with(&content, parse_arg)?,
        }));
    }
    if input.peek(Token![-]) {
        let minus = input.parse()?;
        if input.peek(Token![-]) || !input.peek(Lit) {
            return Err(input.error("expected an integer after `-`"));
        }
        return Ok(Expr::Unary(ExprUnary {
            attrs: vec![],
            op: UnOp::Neg(minus),
            expr: Box::new(Expr::Lit(ExprLit {
                attrs: vec![],
                lit: input.parse()?,
            })),
        }));
    }
    if input.peek(Lit) {
        return Ok(Expr::Lit(ExprLit {
            attrs: vec![],
            lit: input.parse()?,
        }));
    }
    if input.peek(syn::Ident) && input.peek2(Token![!]) {
        return Ok(Expr::Macro(ExprMacro {
            attrs: vec![],
            mac: input.parse()?,
        }));
    }
    if input.peek(syn::Ident) {
        let path = input.call(Path::parse_mod_style)?;
        if input.peek(token::Paren) {
            // Only parsed far enough for `build_argument` to suggest the macro to use instead.
            let args: Group = input.parse()?;
            return Ok(Expr::Call(ExprCall {
                attrs: vec![],
                func: Box::new(path_expr(path)),
                paren_token: token::Paren {
                    span: args.delim_span(),
                },
                args: Punctuated::new(),
            }));
        }
        return Ok(path_expr(path));
    }
    Err(input.error("Invalid argument, only literals, lists and variables are supported"))
}

fn path_expr(path: Path) -> Expr {
    Expr::Path(ExprPath {
        attrs: vec![],
        qself: None,
        path,
    })
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
