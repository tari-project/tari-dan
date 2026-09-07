---
name: tari-ootle
description: Tari Ootle development instructions for OpenCode. Templates (smart contracts) in Rust, compiled to WASM, published on the Tari L2 network.
---

## Overview

Tari Ootle is a decentralized application platform built on the Tari Layer 2 network. You build **templates** (smart contracts) in Rust, compile them to WASM (`wasm32-unknown-unknown`), publish them to the network, and interact with deployed **components** (instances of templates) via transactions.

**Key concepts:**
- **Template** — A Rust module annotated with `#[template]` that defines the logic and state structure. Compiled to WASM and deployed to the network.
- **Component** — A live instance of a template on-chain. Holds state (struct fields) and exposes public methods.
- **Resource** — A native digital asset (fungible token or non-fungible NFT). Created with `ResourceBuilder`. Cannot be copied or accidentally destroyed.
- **Vault** — An on-chain container that holds exactly one type of resource. Must be stored in a component before the function returns.
- **Bucket** — A temporary container for resources during a transaction. Used to move assets between vaults or to/from method calls.
- **Transaction** — A set of instructions (CallFunction, CallMethod, etc.) that are signed, submitted, and executed atomically.

**Crate ecosystem:**

| Crate | Purpose | Used In |
|-------|---------|---------|
| `tari_template_lib` | Core template library: prelude, `ResourceBuilder`, `Vault`, `Bucket`, `ComponentManager`, `CallerContext`, `emit_event`, `rand`, macros (`args!`, `rule!`, `metadata!`) | Templates (WASM) |
| `tari_template_lib_types` | Shared types: `Amount`, `ComponentAddress`, `ResourceAddress`, `NonFungibleId`, `AccessRule`, `OwnerRule`, `Metadata` | Templates & client |
| `tari_ootle_transaction` | `TransactionBuilder` and `args!` macro for constructing transactions | Client & tests |
| `ootle-rs` | Client wallet and Indexer provider: sign, submit, watch transactions; builtin template helpers (faucet) | Client apps |
| `tari_template_test_tooling` | Local test harness (dev-dependency): compile templates to WASM and run against the engine in-process | Tests only |

**Important macro distinction:**
- `args!` (from `tari_template_lib::prelude`) — used **inside templates** for cross-template calls (alias for `invoke_args!`)
- `args!` (from `tari_ootle_transaction`) — used **in test code and client code** for `TransactionBuilder`, `TemplateTest::call_function`, and `call_method`. Produces `Vec<NamedArg>`.

---

## Getting Started Workflow

The typical development workflow for building on Tari Ootle:

1. **Generate a template project** from the official starter repo:
   ```bash
   cargo generate https://github.com/tari-project/wasm-template
   ```
   This repo contains:
   - `wasm_templates/` — Blank template starters (e.g., `wasm_templates/empty`)
   - `examples/` — Complete working examples with templates and client apps (e.g., `examples/guessing_game/template`, `examples/guessing_game/cli`)

   When prompted, select the subfolder matching your needs.

2. **Write your template** in `src/lib.rs` inside the `#[template]` module.

3. **Build to WASM:**
   ```bash
   cargo build --target wasm32-unknown-unknown --release
   ```

4. **Test locally** using `tari_template_test_tooling` (see [Testing Templates](#testing-templates)).

5. **Publish to the network** via the Wallet Web UI (see [Publishing Templates](#publishing-templates)).

6. **Interact with your component** using a client app built with `ootle-rs`, the Wallet CLI, or a pre-built example CLI from `cargo generate`.

> **Tip:** For the guessing game and other example templates, pre-published template addresses are available on the Esmeralda testnet. Check the [Tari Ootle guides](https://ootle.tari.com/guides/) for current addresses — you can skip publishing and go straight to interacting.

### CLI App Workflow Order (IMPORTANT)

When running a CLI client app (e.g., the guessing game CLI), operations MUST happen in this order:

1. **Initialize wallet** — Create keypairs, select network, connect to indexer
2. **Fund admin account** — Get tTARI from the faucet
3. **Publish the template** — Deploy the WASM to the network. **Direct the user to the Wallet Web UI** at `http://127.0.0.1:5100` to publish. Do NOT write custom publish code — the Web UI handles fee estimation, upload, and provides the template address.
4. **Create/deploy the game component** — Instantiate the template on-chain (requires the template address from step 3)
5. **Register players** — Add player accounts AFTER the game component exists
6. **Play** — Start rounds, make guesses, end games

> **CRITICAL:** Never register players or add users before the template is published and the game component is deployed. Players need a component to interact with. Never try to publish a template programmatically unless the CLI already has a publish command — always direct users to the Wallet Web UI for publishing.

### Tooling Requirements

The only tools needed for Ootle development are:
- `rustup` with the `wasm32-unknown-unknown` target
- `cargo-generate` (for scaffolding)
- Standard Rust toolchain (`cargo build`, `cargo test`)
- `wasm-opt` (for minifying the WASM before publishing — see [Minifying the WASM](#minifying-the-wasm))

> **Do NOT install** rust-analyzer extensions, cargo-expand, wasm-pack, wasm-bindgen, or other WASM/Rust analysis tools. They are unnecessary for Ootle development and add bloat. The `wasm32-unknown-unknown` target and standard `cargo build` are sufficient.

### Non-Interactive CLI Usage

The generated CLI examples use `dialoguer` for interactive prompts (Select, Input), which requires a real TTY. **OpenCode runs commands in a non-interactive shell without a TTY.**

When running CLI commands that have interactive prompts:
- **Tell the user to run the command in their terminal** rather than trying to run it through OpenCode
- Do NOT try to pipe input, use `expect`, or wrap with `script` — these are fragile workarounds
- Do NOT modify the CLI to add non-interactive flags unless the user specifically asks for it
- If the CLI already supports `--flag` style arguments that bypass prompts, use those

---

## Writing a Template

### Project Setup

```bash
# Install cargo-generate if not already installed
cargo install cargo-generate

# Generate a new template project from the official starter
cargo generate https://github.com/tari-project/wasm-template
```

The [wasm-template](https://github.com/tari-project/wasm-template) repository offers multiple starting points:
- **`wasm_templates/empty`** — A minimal blank template to start from scratch
- **`examples/guessing_game/template`** — A complete guessing game template with tests
- **`examples/guessing_game/cli`** — A ready-to-use CLI client for the guessing game

When you run `cargo generate`, select the subfolder that matches your goal. For a blank slate, choose a `wasm_templates/` entry. For a working example to learn from, choose from `examples/`.

A generated template project looks like:
```
your_template/
├── Cargo.toml
├── src/
│   └── lib.rs      # Template source code
└── tests/
    └── test.rs     # Unit tests
```

The generated `Cargo.toml` must include:
```toml
[package]
name = "your_template"
version = "0.1.0"
edition = "2024"

[dependencies]
tari_template_lib = "0.20"

[lib]
crate-type = ["cdylib"]

[profile.release]
opt-level = 's'     # Optimize for size.
lto = true          # Enable Link Time Optimization.
codegen-units = 1   # Reduce number of codegen units to increase optimizations.
panic = 'abort'     # Abort on panic.
strip = true          # Strip symbols and debug info.
```

> **CRITICAL:** The `crate-type = ["cdylib"]` is required for WASM compilation. Without it, the build will not produce a `.wasm` file.

> **CRITICAL:** Keep `crate-type = ["cdylib"]` and nothing else. Requesting both `cdylib` and `rlib` (a common attempt to make template code unit-testable) makes rustc silently drop `-C lto`, so the LTO setting below never applies — measured ~76 KB of avoidable bloat. If you want pure logic (tallies, state machines) unit-testable, extract it into a standalone crate that both the template and the tests depend on, and leave the template a pure `cdylib`.

> **Tip:** The `[profile.release]` section in `Cargo.toml` significantly reduces the size of the compiled WASM file, which lowers the fees required for on-chain storage and publishing.

> **Versions:** These crate versions are updated as new releases are published to crates.io. Use the minor version (e.g. `"0.31"` not `"0.31.1"`) to automatically get the latest patch. Before starting a new template, check [crates.io](https://crates.io/crates/tari_template_lib) for a newer minor version (e.g. `"0.32"`, `"0.33"`).

### Compilation

```bash
# Add the WASM target (one-time setup)
rustup target add wasm32-unknown-unknown

# Build the template
cargo build --target wasm32-unknown-unknown --release
```

Output: `target/wasm32-unknown-unknown/release/your_template_name.wasm`

### Minifying the WASM

Minify with `wasm-opt` before publishing (the same tool `tari-ootle-cli` and the wallet daemon use). This typically cuts the binary ~20% and proportionally reduces publish fees:

```bash
wasm-opt -Oz --enable-bulk-memory \
  target/wasm32-unknown-unknown/release/your_template_name.wasm \
  -o target/wasm32-unknown-unknown/release/your_template_name.min.wasm
```

> `--enable-bulk-memory` is required: rustc's `wasm32-unknown-unknown` output uses bulk-memory operations, and `wasm-opt` needs the flag to handle them.
>
> Check the size before publishing: `ls -la target/wasm32-unknown-unknown/release/*.wasm`. A properly configured template is roughly 250 KB after minification; anything much larger will cost proportionally more to publish.

### Template Structure

Every template follows this pattern:

```rust
use tari_template_lib::prelude::*;

#[template]
mod my_template {
    use super::*;

    // Component state — all fields must be serde-serializable
    pub struct MyComponent {
        my_vault: Vault,
        counter: u64,
    }

    impl MyComponent {
        // CONSTRUCTOR: any function returning Self or Component<Self>
        // creates a new component instance on-chain when called via CallFunction
        pub fn new() -> Component<Self> {
            let token = ResourceBuilder::public_fungible()
                .with_token_symbol("TOK")
                .build(); // → ResourceAddress

            Component::new(Self {
                my_vault: Vault::new_empty(token),
                counter: 0,
            })
            .with_access_rules(
                ComponentAccessRules::new()
                    .method("public_method", rule!(allow_all))
                    .default(rule!(deny_all))
            )
            .create()
        }

        // PUBLIC METHOD: takes &self or &mut self, called via CallMethod
        pub fn public_method(&mut self, value: u64) {
            self.counter += value;
        }

        // READ-ONLY METHOD: takes &self, cannot modify state
        pub fn get_counter(&self) -> u64 {
            self.counter
        }

        // FUNCTION (no &self): called via CallFunction on the template, not a method on a component
        pub fn greet(name: String) -> String {
            format!("Hello, {}!", name)
        }
    }

    // Private helper: no &self, not in impl block's public interface
    fn internal_helper() -> u64 {
        42
    }
}
```

### The `#[template]` Macro

- Generates the WASM ABI so the validator engine can call your code
- Exposes public functions in `impl` blocks as callable methods/functions
- Handles state serialization/deserialization automatically
- Structs defined inside the `#[template]` module automatically get serde derives
- Only one `#[template]` module per crate

### Component State Rules

- All struct fields MUST be serde-serializable (types from `tari_template_lib` already are)
- Use `&mut self` methods to modify state
- Use `&self` methods for read-only access
- A component can also be an `enum`
- State size affects transaction costs
- Supported field types: all Rust primitives, `String`, `Vec<T>`, `HashMap<K,V>`, `BTreeMap<K,V>`, `Option<T>`, `Vault`, `Amount`, `ResourceAddress`, `ComponentAddress`, `RistrettoPublicKeyBytes`, `NonFungibleId`, `ComponentManager`, and any struct inside the `#[template]` module

### Constructors

```rust
// Simple constructor — returning Self creates the component with default rules
pub fn new_simple() -> Self {
    Self { counter: 0 }
}

// Explicit constructor — returns Component<Self> for full control
pub fn new_explicit() -> Component<Self> {
    Component::new(Self { counter: 0 })
        .with_access_rules(ComponentAccessRules::new()
            .method("do_something", rule!(allow_all))
            .default(rule!(deny_all))
        )
        .with_owner_rule(OwnerRule::OwnedBySigner)
        .create()
}

// Constructor with address allocation — allows creating and calling in one transaction
pub fn new_with_allocation(addr: ComponentAddressAllocation) -> Component<Self> {
    Component::new(Self { counter: 0 })
        .with_address_allocation(addr)
        .with_access_rules(ComponentAccessRules::new()
            .method("do_something", rule!(allow_all))
            .default(rule!(deny_all))
        )
        .create()
}

// Constructor with public key address — deterministic address from a public key
pub fn new_with_public_key() -> Component<Self> {
    let pk = CallerContext::transaction_signer_public_key();
    Component::new(Self { counter: 0 })
        .with_public_key_address(pk)
        .with_access_rules(ComponentAccessRules::allow_all())
        .create()
}
```

### Error Handling

Errors in templates are handled by panicking. When a panic occurs, the transaction fails atomically and no state changes are committed:

```rust
pub fn do_something(&mut self, value: u64) {
    assert_ne!(value, 0, "Value cannot be zero");
    assert!(value <= 1024, "Value too large");
    if !value.is_power_of_two() {
        panic!("Value must be a power of two");
    }
    self.counter += value;
}
```

> There are no Result-based error flows in templates. Panics are the error mechanism.

---

## Resources

There are 4 resource types:
- **Public Fungible** — Interchangeable tokens (like ERC-20), amounts visible on-chain
- **Public Non-Fungible** — Unique tokens (like ERC-721), metadata visible on-chain
- **Confidential** — Fungible tokens with hidden amounts (Pedersen commitments)
- **Stealth** — Confidential UTXOs with hidden owners (TARI/tTARI is a stealth resource)

### Creating Resources

```rust
// ─── Public Fungible Token ───
// Without initial supply → returns ResourceAddress
let token_addr: ResourceAddress = ResourceBuilder::public_fungible()
    .with_token_symbol("TOK")
    .metadata("name", "My Token")
    .build();

// With initial supply → returns Bucket containing tokens
let token_bucket: Bucket = ResourceBuilder::public_fungible()
    .with_token_symbol("TOK")
    .metadata("name", "My Token")
    .initial_supply(Amount::from(1000))
    .build();

// ─── Non-Fungible (NFT) ───
let nft_addr: ResourceAddress = ResourceBuilder::non_fungible()
    .with_token_symbol("NFT")
    .metadata("name", "My NFT Collection")
    .build();    // Mint later via ResourceManager

// With initial supply of NFTs
let nft_bucket: Bucket = ResourceBuilder::non_fungible()
    .with_token_symbol("NFT")
    .initial_supply_with_data(vec![
        (NonFungibleId::from_u64(1), &metadata!["name" => "First"], &()),
        (NonFungibleId::from_u64(2), &metadata!["name" => "Second"], &()),
    ])
    .build();

// ─── Confidential Fungible ───
let conf_addr: ResourceAddress = ResourceBuilder::confidential()
    .with_token_symbol("cTOK")
    .with_view_key(CallerContext::transaction_signer_public_key())
    .build();

// ─── Stealth (like TARI) ───
let stealth_addr: ResourceAddress = ResourceBuilder::stealth()
    .with_token_symbol("sTOK")
    .build();
```

### Resource Builder Options (All Types)

```rust
ResourceBuilder::public_fungible()  // or non_fungible(), confidential(), stealth()
    // Metadata
    .with_token_symbol("SYM")              // Token symbol (displayed by explorers)
    .metadata("name", "Token Name")        // Add metadata key-value pair
    .add_metadata("key", "value")          // Same as .metadata()

    // Access rules on the resource itself
    .mintable(rule!(resource(admin_badge))) // Who can mint new tokens (default: deny_all)
    .burnable(rule!(allow_all))            // Who can burn tokens (default: deny_all)
    .recallable(rule!(deny_all))           // Who can forcefully recall from vaults
    .freezable(rule!(deny_all))            // Who can freeze vaults holding this resource
    .withdrawable(rule!(allow_all))        // Who can withdraw (default: allow_all)
    .depositable(rule!(allow_all))         // Who can deposit (default: allow_all)
    .update_non_fungible_data(rule!(...))  // Who can update NFT mutable data
    .update_access_rules(rule!(...))       // Who can change these rules later

    // Ownership
    .with_owner_rule(OwnerRule::OwnedBySigner)  // Owner of the resource definition

    // Advanced
    .with_divisibility(2)                  // Fungible only: decimal places (default: 0)
    .disable_total_supply_tracking()       // Don't track total supply on-chain
    .with_address_allocation(alloc)        // Pre-allocated resource address

    // Finalize
    .build()                               // Create the resource (returns ResourceAddress)
    .initial_supply(Amount::from(1000))    // Mint initial tokens (changes return to Bucket)
```

### Minting NFTs After Creation

```rust
let manager: ResourceManager = vault.get_resource_manager();

// Mint a single NFT
let nft_bucket: Bucket = manager.mint_non_fungible(
    NonFungibleId::from_string("unique-id"),   // Unique ID within this resource
    &metadata!["name" => "My NFT"],            // Immutable data (cannot change after mint)
    &(),                                        // Mutable data (can be updated later)
);

// NonFungibleId variants:
NonFungibleId::from_string("my-id")       // String (max 64 chars)
NonFungibleId::from_u64(42)               // u64
NonFungibleId::from_u32(1)                // u32
NonFungibleId::from_u256([0u8; 32])       // 32-byte array
NonFungibleId::random()                   // Random UUID-style
```

### Vaults — Complete API

```rust
// ─── Creation ───
let vault = Vault::new_empty(resource_address);      // Empty vault for a resource type
let vault = Vault::from_bucket(bucket);              // Create vault containing bucket's tokens

// ─── Deposits ───
vault.deposit(bucket);                               // Add tokens from bucket into vault

// ─── Withdrawals ───
let bucket = vault.withdraw(amount);                 // Withdraw fungible amount → Bucket
let bucket = vault.withdraw(1u64);                   // Can pass u64 directly
let bucket = vault.withdraw_non_fungible(nft_id);    // Withdraw one NFT by ID → Bucket
let bucket = vault.withdraw_non_fungibles(id_set);   // Withdraw multiple NFTs → Bucket
let bucket = vault.withdraw_all();                   // Withdraw everything → Bucket

// ─── Queries ───
let balance: Amount = vault.balance();               // Current balance
let locked: Amount = vault.locked_balance();         // Locked/frozen balance
let addr: ResourceAddress = vault.resource_address();// Resource type held by this vault
let ids: BTreeSet<NonFungibleId> = vault.get_non_fungible_ids(); // All NFT IDs in vault

// ─── Resource Management ───
let manager: ResourceManager = vault.get_resource_manager(); // For minting etc.

// ─── Fee Payment ───
vault.pay_fee(amount);                               // Pay transaction fee from this vault

// ─── Authorization ───
vault.authorize();                                   // Create auth proof from vault contents (RAII)
let proof = vault.create_proof_by_amount(amount);    // Create proof for a specific amount
```

> **CRITICAL:** A `Vault` MUST be stored in a component struct field before the function returns. An orphaned vault (created but not stored) will cause the transaction to fail.

### Buckets — Complete API

```rust
// ─── Queries ───
let addr: ResourceAddress = bucket.resource_address(); // What resource this holds
let rtype: ResourceType = bucket.resource_type();       // Fungible, NonFungible, etc.
let amt: Amount = bucket.amount();                      // How many tokens
let empty: bool = bucket.is_empty();                    // Whether empty
let ids = bucket.get_non_fungible_ids();                // NFT IDs in bucket
let nfts = bucket.get_non_fungibles();                  // Full NFT data

// ─── Splitting ───
let new_bucket = bucket.take(Amount::from(50));         // Split off some tokens

// ─── Combining ───
let combined = bucket.join(other_bucket);               // Merge two same-resource buckets

// ─── Destruction ───
bucket.burn();                                          // Permanently destroy tokens
bucket.drop_empty();                                    // Assert empty and drop (panics if not)

// ─── Proofs ───
let proof = bucket.create_proof();                      // Create ownership proof
```

> **CRITICAL:** A `Bucket` MUST be consumed before the function returns. Consume it by: depositing into a vault, burning, returning from a function, or passing to another component. An orphaned bucket will cause the transaction to fail.

---

## Access Rules and Authorization

### Component Access Rules

```rust
Component::new(Self { ... })
    .with_access_rules(
        ComponentAccessRules::new()      // Default: deny_all for unlisted methods
            .method("guess", rule!(allow_all))
            .method("admin_action", rule!(resource(admin_badge)))
            .default(rule!(deny_all))
    )
    // OR use the convenience constructor:
    // ComponentAccessRules::allow_all()  // Default: allow_all for unlisted methods
    .with_owner_rule(OwnerRule::OwnedBySigner)
    .create()
```

### The `rule!` Macro — Complete Reference

```rust
// ─── Basic Rules ───
rule!(allow_all)                                        // No restrictions
rule!(deny_all)                                         // Nobody can call

// ─── Resource-Based Rules ───
rule!(resource(resource_address))                       // Must hold this resource in a proof
rule!(non_fungible(NonFungibleAddress::new(res, id)))   // Must hold specific NFT
rule!(public_key(ristretto_public_key_bytes))           // Must be signed by this key

// ─── Scope Rules ───
rule!(component(component_address))                     // Only callable from this component
rule!(template(template_address))                       // Only callable from this template

// ─── Composite Rules ───
rule!(any_of(resource(a), resource(b)))                 // Any one condition met (OR)
rule!(all_of(resource(a), resource(b)))                 // All conditions met (AND)
rule!(m_of_n(2, resource(a), resource(b), resource(c)))// M of N conditions met
```

### Owner Rules

```rust
OwnerRule::OwnedBySigner           // Default: transaction signer is owner
OwnerRule::None                    // No owner (nobody can update access rules)
OwnerRule::ByAccessRule(rule)      // Custom rule determines ownership
OwnerRule::ByPublicKey(pk)         // Specific public key is owner
```

### Caller Context — Complete API

```rust
// Get the authenticated signer (ALWAYS use this for identity, never accept as argument)
let signer: RistrettoPublicKeyBytes = CallerContext::transaction_signer_public_key();

// Get current component address (only in CallMethod context)
let addr: ComponentAddress = CallerContext::current_component_address();

// Get signer proof (for passing as authorization)
let proof: Proof = CallerContext::get_main_signer_proof();
let proof: Proof = CallerContext::get_signer_proof_for_public_key(pk);

// Address allocation (for creating components/resources with deterministic addresses)
let alloc: ComponentAddressAllocation = CallerContext::allocate_component_address(None);
let alloc: ComponentAddressAllocation = CallerContext::allocate_component_address(Some(pk));
let alloc: ResourceAddressAllocation = CallerContext::allocate_resource_address();
```

> **NEVER** accept a public key as a method argument for authentication. Always use `CallerContext::transaction_signer_public_key()` — it cannot be spoofed.

---

## Cross-Component Calls

```rust
// Get a reference to another component
let other: ComponentManager = ComponentManager::get(component_address);

// Call a method that returns a value
let value: u64 = other.call("method_name", args![arg1, arg2]);

// Call a method that returns unit (fire-and-forget)
other.invoke("method_name", args![arg1, arg2]);

// Common pattern: deposit a bucket into another component (e.g., Account)
other.invoke("deposit", args![prize_bucket]);

// Get template address of a component
let tmpl: TemplateAddress = other.get_template_address();

// Get the address
let addr: ComponentAddress = other.component_address();
```

---

## Events

```rust
// Emit an event (permanently recorded in the transaction receipt)
emit_event("GameEnded", metadata![
    "winner_account" => winner_address.to_string(),
    "number" => winning_number.to_string(),
    "round" => round.to_string(),
]);
```

Events are indexed by the Indexer and can be queried by explorers and dApps. The topic is formatted as `"TemplateName.EventTopic"` in receipts.

---

## Randomness

```rust
use tari_template_lib::rand::random_bytes;

// Get N pseudorandom bytes
let bytes: Vec<u8> = random_bytes(4);

// Convenience: get a random u32
use tari_template_lib::rand::random_u32;
let n: u32 = random_u32();

// Common pattern: random number in range
fn generate_number() -> u8 {
    random_bytes(1)[0] % 11  // 0..=10
}
```

> **WARNING:** `random_bytes` is deterministic — entropy comes from the transaction itself to ensure all validators produce the same result. Do NOT use for cryptographic security. You cannot use the `rand` crate in templates (no entropy source on `wasm32-unknown-unknown`).

---

## Publishing Templates

> **Publishing a template is the most expensive operation you will do** — every validator stores the WASM forever, so publish fees scale with WASM size. Optimize the binary first (see [Minifying the WASM](#minifying-the-wasm)) and always estimate the fee (dry-run or the Web UI's "Estimate Fee" button) instead of guessing. For one-off publishing the Wallet Web UI is the easiest path. Do NOT add a `publish` subcommand to example CLI apps — but a client app may publish programmatically as long as it uses dry-run fee estimation, never a hardcoded fee.

### Publish via Wallet Web UI (Easiest for One-Off Publishing)

1. Open the Tari Ootle Wallet web UI (default: `http://127.0.0.1:5100`)
2. Click "Publish Template" on the Home page
3. Select fee account with tTARI (testnet Tari)
4. Upload the `.wasm` file from `target/wasm32-unknown-unknown/release/`
5. Click "Estimate Fee" then "Publish Template"
6. Find the template address under "Templates" in the sidebar
7. Paste the template address into the CLI's state file or `--template-address` flag

### Publish Programmatically (ootle-rs)

> Programmatic publishing is fine for client apps **when the fee comes from a dry-run estimate** (see [Fee Estimation (Dry-Run)](#fee-estimation-dry-run)) — never from a hardcoded value. The example below uses a placeholder fee; in real code replace it with an estimated fee plus a margin multiplier.

```rust
use tari_ootle_transaction::TransactionBuilder;
use ootle_rs::TransactionRequest;

let wasm_binary: Vec<u8> = std::fs::read("target/wasm32-unknown-unknown/release/your_template.wasm")?;
let unsigned = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()
    .pay_fee_from_component(account_addr, 250_000u64) // See fee note below
    .publish_template(wasm_binary.try_into().unwrap())
    .build_unsigned();
let tx = TransactionRequest::default()
    .with_transaction(unsigned)
    .build(provider.wallet())
    .await?;
let receipt = provider.send_transaction(tx).await?.watch().await?;

// Get the new template address from the receipt
let template_addr = receipt.diff_summary.upped
    .iter()
    .find_map(|s| s.substate_id.as_template())
    .expect("template address in receipt");
```

> **Fee guidance for publishing:** Template publishing fees are proportional to WASM binary size, and a naive (unoptimized) template can cost millions of fee units — a ~300 KB template published at ~7.8 million units in practice. Fee amounts are integer "sats-like" units, not TARI. Reduce the cost by shrinking the WASM (see [Minifying the WASM](#minifying-the-wasm)); a properly configured template should be ~250 KB after minification. Always estimate the fee with a dry-run (or the Wallet Web UI's "Estimate Fee" button) and add a margin multiplier instead of guessing.

### Fee Estimation (Dry-Run)

> **Never hardcode fee amounts in client code.** Hardcoded fees are either several times too high (wasted funds) or too low (failed transactions). Estimate every transaction with a dry-run, then resubmit with the estimated fee plus a margin (2x is a proven safe default):

```rust
let unsigned = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()
    .pay_fee_from_component(account_addr, 0u64) // placeholder
    .call_method(component_addr, "method", args![...])
    .build_unsigned();

// 1. Dry-run to learn the true cost
let dry_run = provider.dry_run(&unsigned).await?;
let estimated: u64 = dry_run.fee_estimate(); // exact accessor varies by ootle-rs version

// 2. Rebuild with the estimated fee (plus margin for estimation variance)
let unsigned = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()
    .pay_fee_from_component(account_addr, estimated * 2)
    .call_method(component_addr, "method", args![...])
    .build_unsigned();
```

The dry-run pattern applies to every transaction type: faucet claims, publishes, method calls, and stealth spends.

---

## Interacting with Deployed Components (Client-Side Rust)

### Setup Wallet and Provider

```rust
use ootle_rs::{
    key_provider::PrivateKeyProvider,
    provider::ProviderBuilder,
    wallet::OotleWallet,
    default_indexer_url,
};
use tari_ootle_common_types::Network;

const NETWORK: Network = Network::Esmeralda;  // Testnet (default_indexer_url is configured)

// Create a random wallet (for testing) or load from seed
let secret = PrivateKeyProvider::random(NETWORK);
let wallet = OotleWallet::from(secret);

let mut provider = ProviderBuilder::new()
    .wallet(wallet)
    .connect(default_indexer_url(NETWORK))
    .await?;

// With custom transaction timeout (default is 32 seconds — too short for testnet):
use std::time::Duration;
let mut provider = ProviderBuilder::new()
    .wallet(wallet)
    .connect_with_transaction_timeout(default_indexer_url(NETWORK), Duration::from_secs(120))
    .await?;
```

> **Timeout guidance:** The default transaction timeout is 32 seconds, which is often too short for the Esmeralda testnet. Use `connect_with_transaction_timeout()` with 120 seconds for testnet usage. LocalNet is faster and the default is usually fine.

**Available networks:**
- `Network::Esmeralda` — Public testnet (indexer: `http://217.182.93.35:50124`)
- `Network::LocalNet` — Local development (indexer: `http://localhost:12500`)
- Other networks (MainNet, StageNet, NextNet, Igor) are not yet configured with default indexer URLs

### Fund Account (Testnet Faucet)

```rust
use ootle_rs::{
    TransactionRequest,
    builtin_templates::{UnsignedTransactionBuilder, faucet::IFaucet},
};
use tari_template_lib_types::constants::TARI;

let unsigned_tx = IFaucet::new(&provider)
    .take_faucet_funds(10 * TARI)    // Request 10 TARI
    .pay_fee(500u64)                     // Fee for the transaction
    .prepare()
    .await?;

let tx = TransactionRequest::default()
    .with_transaction(unsigned_tx)
    .build(provider.wallet())
    .await?;

let pending = provider.send_transaction(tx).await?;
let outcome = pending.watch().await?;
```

### Transaction Pattern (Sign → Send → Watch)

Every on-chain interaction follows this pattern:

```rust
use tari_ootle_transaction::{TransactionBuilder, args};
use ootle_rs::TransactionRequest;

// 1. Build an unsigned transaction
let unsigned_tx = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()                              // Auto-detect input substates
    .pay_fee_from_component(account_addr, 2000u64)        // Placeholder — see Fee Estimation
    .call_function(template_addr, "new", args![])         // Or call_method(...)
    .build_unsigned();

// 2. Sign it
let tx = TransactionRequest::default()
    .with_transaction(unsigned_tx)
    .build(provider.wallet())
    .await?;

// 3. Send and wait for finalization
let pending = provider.send_transaction(tx).await?;
let receipt = pending.watch().await?;
```

> **Fees in these examples are placeholders.** Real client code must estimate fees with a dry-run (see [Fee Estimation (Dry-Run)](#fee-estimation-dry-run)) — never ship hardcoded fee amounts.

### Call a Template Function (Create Component)

```rust
let unsigned_tx = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()
    .pay_fee_from_component(account_addr, 2000u64)
    .call_function(template_addr, "new", args![])
    .build_unsigned();
```

### Call a Component Method

```rust
let unsigned_tx = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()
    .pay_fee_from_component(account_addr, 2000u64)
    .call_method(component_addr, "start_game", args![nft_id])
    .build_unsigned();
```

### TransactionBuilder — Key Methods

```rust
TransactionBuilder::new(network)
    // Input handling
    .with_auto_fill_inputs()                    // Auto-detect required substates
    .add_input(substate_address)                // Add specific input
    .with_inputs(iter_of_inputs)                // Add multiple inputs
    .with_unversioned_inputs(iter)              // Add unversioned inputs

    // Fee payment
    .pay_fee_from_component(account, amount)    // Pay fee from an account component
    .pay_fee_from_bucket(bucket_label, amount)  // Pay fee from a workspace bucket

    // Instructions
    .call_function(template, "fn_name", args![...])   // Call template function
    .call_method(component, "method", args![...])     // Call component method
    .create_account(public_key)                        // Create an account component
    .create_account_with_bucket(pk, bucket_label)      // Create account with initial funds
    .publish_template(wasm_binary)                     // Deploy a template

    // Workspace (chain instruction outputs)
    .put_last_instruction_output_on_workspace("label") // Store output for later use
    .take_from_bucket("label", amount)                 // Take from workspace bucket

    // Address allocation
    .allocate_component_address()                      // Pre-allocate component address
    .allocate_resource_address()                       // Pre-allocate resource address

    // Build
    .build_unsigned()                                  // Produce unsigned transaction
```

### Read Addresses from Receipts

```rust
// Find the new component address
let component_addr = receipt.diff_summary.upped
    .iter()
    .find_map(|s| s.substate_id.as_component_address())
    .expect("component address in receipt");

// Find a resource address (excluding native TARI)
use tari_template_lib_types::constants::TARI_TOKEN;
let resource_addr = receipt.diff_summary.upped
    .iter()
    .find_map(|s| s.substate_id.as_resource_address().filter(|a| *a != TARI_TOKEN))
    .expect("resource address in receipt");

// Find a template address (returns PublishedTemplateAddress)
// IMPORTANT: Use as_template() on SubstateId — NOT as_template_address()
// as_template_address() does NOT exist on SubstateId
let template_addr = receipt.diff_summary.upped
    .iter()
    .find_map(|s| s.substate_id.as_template())
    .expect("template address in receipt");
```

> **IMPORTANT API note:** On `SubstateId`, the method is `as_template()` — it returns `Option<PublishedTemplateAddress>`. There is NO `as_template_address()` method on `SubstateId`. If you need the underlying `TemplateAddress` (a `Hash32`), call `.as_template_address()` on the `PublishedTemplateAddress` result, not on the `SubstateId`.

### Read Events from Receipts

```rust
let event = receipt.events
    .iter()
    .find(|e| e.topic() == "GuessingGame.GameEnded")
    .expect("event in receipt");

let value = event.get_payload("field_name");
```

### Manual Inputs

When the transaction touches vaults/components that auto-fill can't detect, add them manually:

```rust
let unsigned_tx = TransactionBuilder::new(provider.network())
    .with_auto_fill_inputs()
    .add_input(specific_substate_address)
    .with_inputs(addresses.iter().copied().map(Into::into))
    .pay_fee_from_component(account_addr, 2000u64)
    .call_method(component_addr, "end_game", args![])
    .build_unsigned();
```

---

## Testing Templates

Use `tari_template_test_tooling` as a **dev-dependency**. It compiles your template to WASM and runs transactions against it locally using the same execution engine as the network.

### Test Setup

Add to your test crate's `Cargo.toml`:
```toml
[dev-dependencies]
tari_template_test_tooling = "0.40"
```

> `tari_template_test_tooling` re-exports `tari_ootle_transaction` — use `tari_template_test_tooling::transaction::args` (and the other transaction items) in tests instead of adding `tari_ootle_transaction` as a separate dev-dependency.

> **Versions:** These versions may be updated as new crates are published. Use the minor version (e.g. `"0.40"` not `"0.40.0"`) to get the latest patch. Check [crates.io](https://crates.io/crates/tari_template_test_tooling) for newer versions before starting.

### Standard Test Pattern

Most template interactions require multiple instructions in a single transaction (e.g. creating a component then calling a method on it, or paying fees from an account). Use `test.transaction()` to build multi-instruction transactions — this is the standard way to write tests.

```rust
use tari_template_test_tooling::TemplateTest;
use tari_template_test_tooling::transaction::args;

#[test]
fn test_my_template() {
    let mut test = TemplateTest::new(".", ["."]);
    let (account, owner_proof, secret_key) = test.create_funded_account();
    let template_addr = test.get_template_address("MyTemplate");

    // Build a transaction with multiple instructions
    let transaction = test.transaction()
        .call_function(template_addr, "new", args![])
        .put_last_instruction_output_on_workspace("component")
        .call_method("component", "some_method", args![42u64])
        .build_and_seal(&secret_key);

    let result = test.execute_expect_success(transaction, vec![owner_proof]);
}
```

### TemplateTest — Key Methods

```rust
// ─── Construction ───
TemplateTest::my_crate()                    // Test the template in the current crate
TemplateTest::new(base_path, [paths])       // Compile templates from given paths
TemplateTest::new_builtin_only()            // Only built-in templates (Account, etc.)

// ─── Building Transactions ───
let tx = test.transaction()                 // Returns a transaction builder (recommended)
    .call_function(template_addr, "fn", args![...])
    .put_last_instruction_output_on_workspace("name")
    .call_method("name", "method", args![...])
    .build_and_seal(&secret_key);

// ─── Single-Call Convenience Methods ───
// Each creates a transaction with a single call. Useful for simple cases but limited —
// Most template interactions require multiple instructions to be useful (e.g. deposit a bucket
// returned from a previous call). Use test.transaction() for this.
let result: T = test.call_function("TemplateName", "function", args![...], proofs);
let result: T = test.call_method(component_addr, "method", args![...], proofs);

// ─── Account Management ───
let (account, proof, secret) = test.create_funded_account();  // 1B micro-TARI balance
let (account, proof, secret) = test.create_empty_account();

// ─── Execution ───
let result = test.execute_expect_success(transaction, proofs);  // Panics on failure
let result = test.execute_expect_failure(transaction, proofs);  // Panics on success
let result = test.execute_expect_commit(transaction, proofs);   // Panics if not finalized

// ─── State Inspection ───
let value: T = test.extract_component_value(component_addr, "field_path");
let addr = test.get_template_address("TemplateName");

// ─── Configuration ───
test.enable_fees();                          // Enable fee tracking
test.disable_fees();                         // Disable fee tracking (default)
```

---

## Tari Wallet CLI

The `tari_ootle_wallet_cli` is a simple CLI for interacting with the Wallet Daemon (`tari_ootle_walletd`) via its JSON-RPC interface. The wallet daemon is what connects to the network via the indexer.

**Repository:** This CLI is part of the [tari-ootle](https://github.com/tari-project/tari-ootle) repository at `applications/tari_wallet_cli/`.

**Building from source:**
```bash
cargo build --release --bin tari_ootle_wallet_cli
```

**Pre-built binaries** are available on the [releases page](https://github.com/tari-project/tari-ootle/releases).

### Connection

```bash
# Connect to a wallet daemon (default endpoint)
tari_ootle_wallet_cli -d /ip4/127.0.0.1/tcp/12009 <command>

# Or via environment variable
export JRPC_ENDPOINT="/ip4/127.0.0.1/tcp/12009"
tari_ootle_wallet_cli <command>
```

### Account Commands

```bash
# Create a new account
tari_ootle_wallet_cli accounts create --name "my-account"

# List all accounts
tari_ootle_wallet_cli accounts list

# Get account details
tari_ootle_wallet_cli accounts get my-account

# Check balances
tari_ootle_wallet_cli accounts get-balance my-account

# Get free testnet tokens (faucet)
tari_ootle_wallet_cli accounts faucet my-account --amount 1000000

# Set default account
tari_ootle_wallet_cli accounts default my-account
```

### Transaction Commands

```bash
# Call a template function (e.g., create a component)
tari_ootle_wallet_cli transactions submit call-function \
    <template_address> new \
    --fee-account my-account \
    --wait-timeout 30

# Call a component method with arguments
tari_ootle_wallet_cli transactions submit call-method \
    <component_address> guess \
    -a 5 -a <payout_component_address> \
    --fee-account my-account

# Submit a transaction manifest (advanced)
tari_ootle_wallet_cli transactions submit-manifest manifest.tari \
    --fee-account my-account

# Get transaction result
tari_ootle_wallet_cli transactions get <transaction_id>

# Send tokens to another account
tari_ootle_wallet_cli transactions send \
    1000 <resource_address> <destination_pubkey> \
    --fee-account my-account

# Confidential transfer
tari_ootle_wallet_cli transactions confidential-transfer \
    1000 <destination_ootle_address> \
    --account my-account
```

### Key Management

```bash
tari_ootle_wallet_cli keys list
tari_ootle_wallet_cli keys create
```

---

## Complete Examples

### Example 1: Simple Counter Template

```rust
use tari_template_lib::prelude::*;

#[template]
mod counter {
    use super::*;

    pub struct Counter {
        value: u64,
    }

    impl Counter {
        pub fn new(initial: u64) -> Component<Self> {
            Component::new(Self { value: initial })
                .with_access_rules(ComponentAccessRules::allow_all())
                .create()
        }

        pub fn increment(&mut self) {
            self.value += 1;
        }

        pub fn get(&self) -> u64 {
            self.value
        }
    }
}
```

### Example 2: Fungible Token with Admin Badge

```rust
use tari_template_lib::prelude::*;

#[template]
mod token {
    use super::*;

    pub struct MyToken {
        token_vault: Vault,
        admin_badge_vault: Vault,
    }

    impl MyToken {
        pub fn new() -> Component<Self> {
            // Create an admin badge NFT
            let admin_badge = ResourceBuilder::non_fungible()
                .with_token_symbol("ADMIN")
                .initial_supply_with_data(vec![
                    (NonFungibleId::from_u64(0), &metadata!["role" => "admin"], &()),
                ])
                .build();
            let admin_resource = admin_badge.resource_address();

            // Create the token, mintable only by admin badge holder
            let initial_tokens = ResourceBuilder::public_fungible()
                .with_token_symbol("MYTKN")
                .metadata("name", "My Token")
                .mintable(rule!(resource(admin_resource)))
                .burnable(rule!(allow_all))
                .initial_supply(Amount::from(1_000_000))
                .build();
            let token_resource = initial_tokens.resource_address();

            Component::new(Self {
                token_vault: Vault::from_bucket(initial_tokens),
                admin_badge_vault: Vault::from_bucket(admin_badge),
            })
            .with_access_rules(ComponentAccessRules::new()
                .method("withdraw", rule!(allow_all))
                .method("get_balance", rule!(allow_all))
                .default(rule!(resource(admin_resource)))
            )
            .create()
        }

        pub fn get_balance(&self) -> Amount {
            self.token_vault.balance()
        }

        pub fn withdraw(&mut self, amount: Amount) -> Bucket {
            self.token_vault.withdraw(amount)
        }

        pub fn mint_more(&mut self, amount: Amount) {
            // Authorize with admin badge, then mint
            self.admin_badge_vault.authorize();
            let manager = self.token_vault.get_resource_manager();
            let new_tokens = manager.mint_fungible(amount);
            self.token_vault.deposit(new_tokens);
        }
    }
}
```

### Example 3: Guessing Game (Full Featured)

```rust
use tari_template_lib::prelude::*;

#[template]
mod guessing_game {
    use std::{collections::HashMap, mem};
    use super::*;

    const MAXIMUM_GUESSES_PER_ROUND: usize = 5;

    pub struct GuessingGame {
        prize_vault: Vault,
        guesses: HashMap<RistrettoPublicKeyBytes, Guess>,
        round_number: u32,
    }

    pub struct Guess {
        pub payout_to: ComponentManager,
        pub guess: u8,
    }

    impl GuessingGame {
        pub fn new(address: ComponentAddressAllocation) -> Component<Self> {
            let prize_resource = ResourceBuilder::non_fungible()
                .metadata("name", "Guessing Game Prize")
                .with_token_symbol("DICE")
                .build();

            let access_rules = ComponentAccessRules::new()
                .method("guess", rule!(allow_all));

            Component::new(Self {
                prize_vault: Vault::new_empty(prize_resource),
                guesses: HashMap::new(),
                round_number: 0,
            })
            .with_address_allocation(address)
            .with_access_rules(access_rules)
            .create()
        }

        pub fn start_game(&mut self, prize: NonFungibleId) {
            assert!(!self.is_game_in_progress(), "Game already in progress!");
            self.round_number += 1;
            let manager = self.prize_vault.get_resource_manager();
            let prize = manager.mint_non_fungible(
                prize,
                &metadata!["round" => self.round_number.to_string()],
                &(),
            );
            self.prize_vault.deposit(prize);
        }

        pub fn guess(&mut self, guess: u8, payout_to: ComponentAddress) {
            assert!(guess <= 10, "Guess must be from 0 to 10");
            assert!(self.guesses.len() < MAXIMUM_GUESSES_PER_ROUND, "No more guesses allowed");
            assert!(self.is_game_in_progress(), "No game has been started");

            let player = CallerContext::transaction_signer_public_key();
            let payout_to = ComponentManager::get(payout_to);
            let prev = self.guesses.insert(player, Guess { payout_to, guess });
            assert!(prev.is_none(), "You already guessed in this round");
        }

        pub fn end_game_and_payout(&mut self) {
            let prize = self.prize_vault.withdraw(1u64);
            let number = generate_number();
            let guesses = mem::take(&mut self.guesses);
            let num_participants = guesses.len();

            for (player, guess) in guesses {
                if guess.guess == number {
                    guess.payout_to.invoke("deposit", args![prize]);
                    emit_event("GameEnded", metadata![
                        "winner" => player.to_string(),
                        "winner_account" => guess.payout_to.component_address().to_string(),
                        "number" => number.to_string(),
                        "num_participants" => num_participants.to_string(),
                    ]);
                    return;
                }
            }

            emit_event("GameEnded", metadata![
                "number" => number.to_string(),
                "num_participants" => num_participants.to_string(),
            ]);
            prize.burn();
        }

        fn is_game_in_progress(&self) -> bool {
            !self.prize_vault.balance().is_zero()
        }
    }

    fn generate_number() -> u8 {
        use tari_template_lib::rand::random_bytes;
        random_bytes(1)[0] % 11
    }
}
```

---

## Common Mistakes to Avoid

1. **Orphaned Vault** — Creating a `Vault` but not storing it in a component field → transaction fails.
2. **Orphaned Bucket** — Not consuming a `Bucket` (deposit, burn, or return it) → transaction fails.
3. **Spoofable Auth** — Accepting a public key as a function argument for identity → use `CallerContext::transaction_signer_public_key()`.
4. **Wrong rand** — Using the `rand` crate → use `tari_template_lib::rand::random_bytes` (no entropy on wasm32).
5. **No Access Rules** — Forgetting `.with_access_rules()` → default is `deny_all`, only the component creator/owner can call methods.
6. **Wrong Resource in Vault** — Depositing a different resource type into a vault → transaction fails.
7. **Missing `cdylib`** — Forgetting `crate-type = ["cdylib"]` in Cargo.toml → no WASM output produced.
8. **Using the wrong `args!` macro** — Ensure you use `args!` from `tari_ootle_transaction` for client/test code (produces `Vec<NamedArg>`) and `args!` from `tari_template_lib::prelude` for cross-template calls inside templates.
9. **Returning mutable bucket** — Forgetting to actually deposit/burn a bucket in all code paths → transaction fails if the bucket isn't consumed.
10. **Large state** — Storing unbounded data structures → high transaction costs, potential DoS.
11. **Hallucinated APIs** — These methods/types do NOT exist. Never use them:
    - `SubstateId::as_template_address()` — use `as_template()` instead
    - `IAccount::publish_template()` — no such method; publish via `TransactionBuilder::publish_template()` or the Web UI
    - `provider.publish_template()` — no such method on the provider
    - `ProviderBuilder::with_timeout()` — use `connect_with_transaction_timeout()` instead
12. **Hardcoded publish fees** — Publishing is expensive (proportional to WASM size; a naive ~300 KB template cost ~7.8M fee units in practice). Optimize and minify the WASM first, then estimate the fee via dry-run or the Web UI's "Estimate Fee" button. Do NOT add a `publish` subcommand to example CLI apps — the generated CLI examples do not include one by design; for one-off publishing use the Wallet Web UI.
13. **Wrong operation order** — Always: init wallet → fund → publish template → create component → register players. Never register players before the game component exists.
14. **Struct placement in template module** — The `#[template]` macro requires the main component struct to appear first in the template module. Placing other structs above it causes the macro to treat the wrong struct as the component, leading to compilation errors like *"a template must have associated functions and/or methods"*. Fix: define ancillary structs in their own module and `use` them, or place them below the component `impl` block. Note: ancillary structs defined outside the template module must derive `#[derive(serde::Serialize, serde::Deserialize)]` and require `serde = "1"` as a dependency.
15. **Git dependencies** — Never use git dependencies in `Cargo.toml`. All Tari crates are published on [crates.io](https://crates.io). Always use the latest minor version (e.g. `"0.20"` not a git URL). Check crates.io if unsure.
16. **Duplicate test dependency** — `tari_template_test_tooling` re-exports the `tari_ootle_transaction` crate. Use the re-export (`tari_template_test_tooling::transaction`) in tests rather than adding `tari_ootle_transaction` as a separate `[dev-dependencies]` entry.
17. **Missing standard imports** — Import standard library types (e.g. `HashMap`, `BTreeMap`) as normal in Rust. You can import them outside the template module and bring them in with `use super::*;` (which all template modules should include), or import directly inside the template module.

---

## Quick Reference: Prelude Exports

The `tari_template_lib::prelude::*` import gives you:

| Category | Types/Items |
|----------|-------------|
| **Core** | `Component`, `ComponentManager`, `CallerContext`, `Consensus` |
| **Resources** | `ResourceBuilder`, `ResourceManager`, `Vault`, `Bucket`, `Proof`, `NonFungible` |
| **Addresses** | `ComponentAddress`, `ResourceAddress`, `TemplateAddress`, `NonFungibleAddress`, `NonFungibleId`, `VaultId` |
| **Allocations** | `ComponentAddressAllocation`, `ResourceAddressAllocation` |
| **Access Control** | `AccessRule`, `ComponentAccessRules` (aliased as `AccessRules`), `OwnerRule` |
| **Amounts** | `Amount` |
| **Crypto** | `RistrettoPublicKeyBytes`, `PublicKey`, `Signature` |
| **Metadata** | `Metadata` |
| **Constants** | `TARI`, `PUBLIC_IDENTITY_RESOURCE_ADDRESS`, `STEALTH_TARI_RESOURCE_ADDRESS` |
| **Macros** | `template`, `args!`, `rule!`, `metadata!`, `debug!`, `info!`, `warn!`, `error!` |
| **Functions** | `emit_event` |
| **Modules** | `rand` (for `random_bytes`, `random_u32`) |
| **Templates** | `BuiltinTemplate`, `TemplateManager` |
| **Auth** | `Account`, `SignatureVerifier`, `Verifiable` |
