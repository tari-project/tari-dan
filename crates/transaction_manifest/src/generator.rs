//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::collections::HashMap;

use proc_macro2::Ident;
use tari_ootle_transaction::{
    Blob,
    BlobIndex,
    Blobs,
    ComponentReference,
    Instruction,
    args::{InstructionArg, WorkspaceId, WorkspaceOffsetId},
    call_arg,
};
use tari_template_lib_types::{ComponentAddress, TemplateAddress, crypto::RistrettoPublicKeyBytes};

use crate::{
    ManifestInstructions,
    ManifestValue,
    ast::ManifestAst,
    error::ManifestError,
    parser::{InvokeIntent, ManifestIntent, ManifestLiteral, OrVar, OutputBinding, SpecialLiteral},
    value::{MAX_LIST_DEPTH, address_value, arg_to_value, lit_to_arg},
};

const MAX_CALL_DEPTH: usize = 16;
/// Caps the intents a manifest may visit through local function calls. Local functions may call each other, so
/// the call depth alone bounds only nesting, not fan-out.
const MAX_GENERATED_INSTRUCTIONS: usize = 10_000;

/// A name the manifest itself binds with `let`.
enum Binding {
    /// `let x = var!["name"]`: the caller-supplied input value.
    Input(ManifestValue),
    /// A value the transaction places on the workspace when it runs.
    Workspace(WorkspaceOffsetId),
    /// `let x = blob!("name")`: the name of a caller-supplied blob.
    Blob(String),
}

/// What a bare identifier refers to.
enum Resolved<'a> {
    Value(&'a ManifestValue),
    Workspace(WorkspaceOffsetId),
    Blob,
}

pub struct ManifestInstructionGenerator {
    imported_templates: HashMap<Ident, TemplateAddress>,
    /// Names bound by the manifest. A later `let` replaces an earlier binding of the same name.
    bindings: HashMap<String, Binding>,
    /// Caller-supplied inputs. A bare identifier resolves to one only when the manifest does not bind that name.
    globals: HashMap<String, ManifestValue>,
    current_workspace_id: WorkspaceId,
    generated_instructions: usize,
    templates: HashMap<String, TemplateAddress>,
    /// Caller-supplied blob payloads keyed by name. Used to resolve `blob!(name)` references.
    blob_inputs: HashMap<String, Blob>,
    /// Output blob list — populated lazily as `blob!(name)` is encountered. The order matches
    /// the order of first reference, so emitted `BlobIndex`es match the returned `Blobs`.
    blob_indices: HashMap<String, BlobIndex>,
    output_blobs: Blobs,
    functions: HashMap<String, Vec<ManifestIntent>>,
    call_depth: usize,
}

impl ManifestInstructionGenerator {
    pub fn new(
        globals: HashMap<String, ManifestValue>,
        templates: HashMap<String, TemplateAddress>,
        blob_inputs: HashMap<String, Blob>,
    ) -> Self {
        Self {
            imported_templates: HashMap::new(),
            bindings: HashMap::new(),
            globals,
            current_workspace_id: WorkspaceId::default(),
            generated_instructions: 0,
            templates,
            blob_inputs,
            blob_indices: HashMap::new(),
            output_blobs: Blobs::empty(),
            functions: HashMap::new(),
            call_depth: 0,
        }
    }

    pub fn generate_instructions(&mut self, ast: ManifestAst) -> Result<ManifestInstructions, ManifestError> {
        for import in ast.parsed.defines {
            let address = match import.template_address {
                Some(addr) => addr,
                None => {
                    let alias = import.alias.to_string();
                    self.templates
                        .get(&alias)
                        .copied()
                        .ok_or(ManifestError::TemplateAliasNotDefined { alias })?
                },
            };
            match self.imported_templates.get(&import.alias) {
                Some(existing) if *existing != address => {
                    return Err(ManifestError::TemplateAliasRedefined {
                        alias: import.alias.to_string(),
                    });
                },
                Some(_) => {},
                None => {
                    self.imported_templates.insert(import.alias, address);
                },
            }
        }

        self.functions = ast.parsed.functions;

        let mut instructions = Vec::with_capacity(ast.parsed.instruction_intents.len());
        for intent in ast.parsed.instruction_intents {
            instructions.extend(self.translate_intent(intent)?);
        }

        let mut fee_instructions = Vec::with_capacity(ast.parsed.fee_instruction_intents.len());
        for intent in ast.parsed.fee_instruction_intents {
            fee_instructions.extend(self.translate_intent(intent)?);
        }

        Ok(ManifestInstructions {
            instructions,
            fee_instructions,
            blobs: std::mem::take(&mut self.output_blobs),
        })
    }

    #[expect(clippy::too_many_lines)]
    fn translate_intent(&mut self, intent: ManifestIntent) -> Result<Vec<Instruction>, ManifestError> {
        match intent {
            ManifestIntent::InvokeTemplate(InvokeIntent {
                output_variable,
                template_variable,
                function_name,
                arguments,
                ..
            }) => {
                let template_ident = template_variable
                    .as_ref()
                    .expect("AST parse should have failed: no template ident for TemplateInvoke statement");
                let mut instructions = vec![Instruction::CallFunction {
                    address: self.get_imported_template(template_ident)?,
                    function: function_name
                        .to_string()
                        .try_into()
                        .map_err(|e| ManifestError::InvalidInstruction {
                            reason: format!("Function name is too long: {}", e),
                        })?,
                    args: self.process_args(arguments)?,
                }];
                if let Some(binding) = output_variable {
                    let key = self.next_workspace_id_raw()?;
                    instructions.push(Instruction::PutLastInstructionOutputOnWorkspace { key });
                    self.register_output_binding(binding, key);
                }
                Ok(instructions)
            },
            ManifestIntent::InvokeComponent(InvokeIntent {
                output_variable,
                component_variable,
                function_name,
                arguments,
                ..
            }) => {
                let component_ident = component_variable
                    .as_ref()
                    .expect("AST parse should have failed: no component ident for ComponentInvoke statement")
                    .to_string();
                let call = match self.resolve(&component_ident)? {
                    Resolved::Value(value) => ComponentReference::Address(Self::extract_component_address(value)?),
                    Resolved::Workspace(workspace_offset) => {
                        if workspace_offset.offset().is_some() {
                            return Err(ManifestError::InvalidInstruction {
                                reason: format!(
                                    "Tuple-destructured variable '{component_ident}' cannot be used as a component \
                                     reference. Use a single variable binding instead."
                                ),
                            });
                        }
                        ComponentReference::Workspace(workspace_offset.id())
                    },
                    Resolved::Blob => return Err(Self::misplaced_blob(&component_ident)),
                };
                let mut instructions = vec![Instruction::CallMethod {
                    call,
                    method: function_name
                        .to_string()
                        .try_into()
                        .map_err(|e| ManifestError::InvalidInstruction {
                            reason: format!("Method name is too long: {}", e),
                        })?,
                    args: self.process_args(arguments)?,
                }];
                if let Some(binding) = output_variable {
                    let key = self.next_workspace_id_raw()?;
                    instructions.push(Instruction::PutLastInstructionOutputOnWorkspace { key });
                    self.register_output_binding(binding, key);
                }
                Ok(instructions)
            },
            ManifestIntent::AllocateAddress(alloc) => {
                let workspace_id = self.next_workspace_id(alloc.output_variable.to_string())?;
                Ok(vec![Instruction::AllocateAddress {
                    allocatable_type: alloc.allocatable_type,
                    workspace_id,
                }])
            },
            ManifestIntent::CreateAccount(create_account) => {
                let owner_public_key = self.extract_public_key(&create_account.owner_public_key.to_string())?;

                let owner_rule = create_account
                    .owner_rule
                    .map(|ident| self.extract_from_global(&ident.to_string()))
                    .transpose()?;

                let access_rules = create_account
                    .access_rules
                    .map(|ident| self.extract_from_global(&ident.to_string()))
                    .transpose()?;

                let bucket_workspace_id = create_account
                    .bucket
                    .map(|ident| self.get_workspace_id(&ident.to_string()))
                    .transpose()?;

                let mut instructions = vec![Instruction::CreateAccount {
                    owner_public_key,
                    owner_rule,
                    access_rules,
                    bucket_workspace_id,
                }];
                if let Some(var_name) = create_account.output_variable {
                    let key = self.next_workspace_id(var_name.to_string())?;
                    instructions.push(Instruction::PutLastInstructionOutputOnWorkspace { key });
                }
                Ok(instructions)
            },
            ManifestIntent::AssignInput(assign) => {
                let value = self.get_global(&assign.global_variable_name.value())?.clone();
                self.bindings
                    .insert(assign.variable_name.to_string(), Binding::Input(value));
                Ok(vec![])
            },
            ManifestIntent::AssignBlob(assign) => {
                self.bindings.insert(
                    assign.variable_name.to_string(),
                    Binding::Blob(assign.blob_name.to_string()),
                );
                Ok(vec![])
            },
            ManifestIntent::DropAllProofs => Ok(vec![Instruction::DropAllProofsInWorkspace]),
            ManifestIntent::PutIntoBucket(p) => {
                let src = self.get_workspace_id(&p.src.to_string())?;
                let dest = self.get_workspace_id(&p.dest.to_string())?;
                Ok(vec![Instruction::PutIntoBucket { src, dest }])
            },
            ManifestIntent::PublishTemplate(pt) => {
                // `pt.blob_name` is either a blob variable bound with `let x = blob!("name")` or a
                // blob name directly; resolve a variable to its blob name, then reuse the same
                // blob-resolution path as `blob!(name)` — the binary is registered in the output
                // Blobs on first reference and assigned the next BlobIndex.
                let ident = pt.blob_name.to_string();
                let blob_name = match self.bindings.get(&ident) {
                    Some(Binding::Blob(blob_name)) => blob_name.clone(),
                    Some(_) => {
                        return Err(ManifestError::InvalidVariableType(format!(
                            "publish_template!({ident}) requires a blob, but '{ident}' is bound to a value that is \
                             not a blob"
                        )));
                    },
                    None => ident,
                };
                let arg = self.resolve_blob_arg(&blob_name)?;
                let binary = arg.as_blob_index().ok_or_else(|| ManifestError::InvalidInstruction {
                    reason: "publish_template! resolved to a non-Blob arg".to_string(),
                })?;
                let metadata_hash = pt
                    .metadata_hash_hex
                    .map(|hex| tari_ootle_template_metadata::MetadataHash::from_hex(&hex))
                    .transpose()
                    .map_err(|e| ManifestError::InvalidInstruction {
                        reason: format!("Invalid metadata hash hex: {}", e),
                    })?;
                Ok(vec![Instruction::PublishTemplate { binary, metadata_hash }])
            },
            ManifestIntent::CallLocalFunction(ident) => {
                if self.call_depth >= MAX_CALL_DEPTH {
                    return Err(ManifestError::MaxCallDepthExceeded { max: MAX_CALL_DEPTH });
                }
                let name = ident.to_string();
                let body = self
                    .functions
                    .get(&name)
                    .ok_or_else(|| ManifestError::UndefinedFunction { name: name.clone() })?
                    .clone();
                self.call_depth += 1;
                let mut instructions = Vec::with_capacity(body.len());
                for intent in body {
                    // Every visited intent is charged, whether or not it emits anything, so a call tree of empty
                    // bodies cannot expand without bound.
                    self.generated_instructions = self.generated_instructions.saturating_add(1);
                    if self.generated_instructions > MAX_GENERATED_INSTRUCTIONS {
                        return Err(ManifestError::TooManyInstructions {
                            max: MAX_GENERATED_INSTRUCTIONS,
                        });
                    }
                    instructions.extend(self.translate_intent(intent)?);
                }
                self.call_depth -= 1;
                Ok(instructions)
            },
        }
    }

    fn next_workspace_id_raw(&mut self) -> Result<WorkspaceId, ManifestError> {
        let id = self.current_workspace_id;
        self.current_workspace_id = id
            .checked_add(1)
            .ok_or(ManifestError::WorkspaceIdOverflow { max: WorkspaceId::MAX })?;
        Ok(id)
    }

    fn next_workspace_id(&mut self, name: String) -> Result<WorkspaceId, ManifestError> {
        let id = self.next_workspace_id_raw()?;
        self.bindings
            .insert(name, Binding::Workspace(WorkspaceOffsetId::new(id)));
        Ok(id)
    }

    fn register_output_binding(&mut self, binding: OutputBinding, workspace_id: WorkspaceId) {
        match binding {
            OutputBinding::Single(ident) => {
                self.bindings.insert(
                    ident.to_string(),
                    Binding::Workspace(WorkspaceOffsetId::new(workspace_id)),
                );
            },
            OutputBinding::Tuple(idents) => {
                for (i, ident) in idents.into_iter().enumerate() {
                    self.bindings.insert(
                        ident.to_string(),
                        Binding::Workspace(WorkspaceOffsetId::new(workspace_id).with_offset(i)),
                    );
                }
            },
        }
    }

    fn process_args(&mut self, args: Vec<ManifestLiteral>) -> Result<Vec<InstructionArg>, ManifestError> {
        args.into_iter().map(|arg| self.process_arg(arg)).collect()
    }

    fn process_arg(&mut self, arg: ManifestLiteral) -> Result<InstructionArg, ManifestError> {
        match arg {
            ManifestLiteral::Lit(lit) => lit_to_arg(&lit),
            ManifestLiteral::Workspace(ident) => self.get_ident(&ident.to_string()),
            ManifestLiteral::Blob(ident) => self.resolve_blob_arg(&ident.to_string()),
            ManifestLiteral::Global(name) => self.get_global(&name.value())?.to_arg(),
            ManifestLiteral::Array(items) => {
                let items = items
                    .into_iter()
                    .map(|item| self.list_element_value(item))
                    .collect::<Result<_, _>>()?;
                let bytes = tari_bor::encode(&tari_bor::Value::Array(items))?;
                // An element can be a `cbor!` literal or a list-valued variable, so the bracket count
                // alone does not bound the depth of the encoded value. Decoding it applies the same
                // bound as every other value.
                tari_bor::decode::<tari_bor::Value>(&bytes).map_err(|_| ManifestError::InvalidInstruction {
                    reason: format!("list argument nests deeper than {MAX_LIST_DEPTH} levels"),
                })?;
                Ok(InstructionArg::raw_literal_bytes(bytes))
            },
            ManifestLiteral::Special(SpecialLiteral::Null) => {
                Ok(InstructionArg::literal(tari_bor::Value::Null).expect("Null literal serialization should not fail"))
            },
            ManifestLiteral::Special(SpecialLiteral::Amount(amount)) => Ok(call_arg!(amount)),
            ManifestLiteral::Special(SpecialLiteral::NonFungibleId(id)) => Ok(call_arg!(id)),
            ManifestLiteral::Special(SpecialLiteral::Cbor(value)) => Ok(InstructionArg::literal(value)?),
            ManifestLiteral::Special(SpecialLiteral::Metadata(metadata)) => Ok(call_arg!(metadata)),
            ManifestLiteral::Special(SpecialLiteral::SubstateId(id_or_var)) => match id_or_var {
                OrVar::Var(ident) => self.get_ident(&ident.to_string()),
                OrVar::Value(id) => Ok(call_arg!(id)),
            },
            ManifestLiteral::Special(SpecialLiteral::Address(var_or_id)) => match var_or_id {
                OrVar::Var(ident) => self.get_ident(&ident.to_string()),
                OrVar::Value(id) => Ok(InstructionArg::literal(address_value(&id)?)?),
            },
        }
    }

    /// The value `item` holds as an element of a list, which is written into the transaction as
    /// one literal and so cannot hold a workspace value or a blob.
    fn list_element_value(&mut self, item: ManifestLiteral) -> Result<tari_bor::Value, ManifestError> {
        match &item {
            ManifestLiteral::Workspace(ident) |
            ManifestLiteral::Special(
                SpecialLiteral::SubstateId(OrVar::Var(ident)) | SpecialLiteral::Address(OrVar::Var(ident)),
            ) => {
                let name = ident.to_string();
                if let Ok(Resolved::Workspace(_)) = self.resolve(&name) {
                    return Err(ManifestError::InvalidInstruction {
                        reason: format!(
                            "'{name}' is a workspace value, which is only known when the transaction runs, so it \
                             cannot be an element of a list"
                        ),
                    });
                }
            },
            ManifestLiteral::Blob(ident) => {
                return Err(ManifestError::InvalidInstruction {
                    reason: format!("blob!({ident}) refers to a transaction blob and cannot be an element of a list"),
                });
            },
            _ => {},
        }
        arg_to_value(self.process_arg(item)?)
    }

    /// Resolve `blob!(name)` to an `InstructionArg::Blob(idx)`. The name must have a payload
    /// in the `blob_inputs` map (supplied to `parse_manifest`); the blob is appended to the
    /// output `Blobs` on first reference and assigned the next free `BlobIndex`. Repeated
    /// references to the same name reuse the same index.
    fn resolve_blob_arg(&mut self, name: &str) -> Result<InstructionArg, ManifestError> {
        if let Some(idx) = self.blob_indices.get(name) {
            return Ok(InstructionArg::Blob(*idx));
        }
        let blob = self
            .blob_inputs
            .get(name)
            .ok_or_else(|| ManifestError::BlobNotProvided { name: name.to_string() })?
            .clone();
        let idx = self
            .output_blobs
            .push(blob)
            .map_err(|e| ManifestError::BlobIndexOverflow { max: e.max })?;
        self.blob_indices.insert(name.to_string(), idx);
        Ok(InstructionArg::Blob(idx))
    }

    fn get_imported_template(&self, name: &Ident) -> Result<TemplateAddress, ManifestError> {
        self.imported_templates
            .get(name)
            .copied()
            .ok_or_else(|| ManifestError::TemplateNotImported { name: name.to_string() })
    }

    fn get_global(&self, name: &str) -> Result<&ManifestValue, ManifestError> {
        self.globals
            .get(name)
            .ok_or_else(|| ManifestError::UndefinedGlobal { name: name.to_string() })
    }

    fn extract_component_address(value: &ManifestValue) -> Result<ComponentAddress, ManifestError> {
        value
            .as_address()
            .and_then(|addr| addr.as_component_address())
            .ok_or_else(|| {
                ManifestError::InvalidVariableType(format!("Expected component variable but got {:?}", value))
            })
    }

    fn extract_public_key(&self, name: &str) -> Result<RistrettoPublicKeyBytes, ManifestError> {
        let value = self.resolve_value(name)?;

        match value {
            ManifestValue::Value(tari_bor::Value::Bytes(bytes)) => RistrettoPublicKeyBytes::from_bytes(bytes)
                .map_err(|e| ManifestError::InvalidVariableType(e.to_string())),
            _ => Err(ManifestError::InvalidVariableType(format!(
                "Expected public key bytes for variable '{name}' but got {value:?}"
            ))),
        }
    }

    fn extract_from_global<T>(&self, name: &str) -> Result<T, ManifestError>
    where T: tari_bor::DeserializeOwned + for<'b> tari_bor::Decode<'b, ()> {
        let value = self.resolve_value(name)?;

        match value {
            ManifestValue::Value(v) => tari_bor::from_value(v).map_err(|e| {
                ManifestError::InvalidVariableType(format!("Failed to deserialize variable '{name}': {e}"))
            }),
            _ => Err(ManifestError::InvalidVariableType(format!(
                "Expected serialized value for variable '{name}' but got {value:?}"
            ))),
        }
    }

    /// Resolves a bare identifier. Every position an identifier can appear in resolves it here, so a name means the
    /// same thing throughout a manifest.
    fn resolve(&self, name: &str) -> Result<Resolved<'_>, ManifestError> {
        match self.bindings.get(name) {
            Some(Binding::Input(value)) => Ok(Resolved::Value(value)),
            Some(Binding::Workspace(id)) => Ok(Resolved::Workspace(*id)),
            Some(Binding::Blob(_)) => Ok(Resolved::Blob),
            None => self
                .globals
                .get(name)
                .map(Resolved::Value)
                .ok_or_else(|| ManifestError::UndefinedVariable { name: name.to_string() }),
        }
    }

    fn resolve_value(&self, name: &str) -> Result<&ManifestValue, ManifestError> {
        match self.resolve(name)? {
            Resolved::Value(value) => Ok(value),
            Resolved::Workspace(_) => Err(ManifestError::InvalidVariableType(format!(
                "'{name}' is a workspace value, which is only known when the transaction runs, but this position \
                 requires a value known when the manifest is built"
            ))),
            Resolved::Blob => Err(Self::misplaced_blob(name)),
        }
    }

    fn get_workspace_id(&self, name: &str) -> Result<WorkspaceOffsetId, ManifestError> {
        match self.bindings.get(name) {
            Some(Binding::Workspace(id)) => Ok(*id),
            Some(Binding::Input(_)) => Err(ManifestError::InvalidVariableType(format!(
                "'{name}' is an input value, but this position requires a workspace value"
            ))),
            Some(Binding::Blob(_)) => Err(Self::misplaced_blob(name)),
            None => Err(ManifestError::UndefinedVariable { name: name.to_string() }),
        }
    }

    fn get_ident(&self, name: &str) -> Result<InstructionArg, ManifestError> {
        match self.resolve(name)? {
            Resolved::Value(value) => value.to_arg(),
            Resolved::Workspace(id) => Ok(InstructionArg::Workspace(id)),
            Resolved::Blob => Err(Self::misplaced_blob(name)),
        }
    }

    fn misplaced_blob(name: &str) -> ManifestError {
        ManifestError::InvalidVariableType(format!(
            "'{name}' is bound to a blob, which only publish_template!({name}) accepts; pass blob!(..) directly to \
             use a blob as an argument"
        ))
    }
}
