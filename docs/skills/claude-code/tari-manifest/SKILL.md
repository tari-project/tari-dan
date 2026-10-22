---
name: tari-manifest
description: This skill should be used when the user asks to "write a manifest", "create a transaction manifest", "submit a transaction", "call a component method", "call a template function", "interact with a Tari component", "write Ootle manifest", "authenticate with wallet daemon", or needs to construct Tari Ootle transaction manifests for submitting transactions on the Tari network.
---

# Tari Ootle Transaction Manifests

Tari Ootle manifests are a Rust-like DSL for defining transaction instructions. They are parsed by `tari_transaction_manifest::parse_manifest()` and compiled into `Vec<Instruction>` for execution on the Tari engine.

## CRITICAL: User Confirmation Required Before Submission

**NEVER submit a transaction without explicit user confirmation.** Before calling `transactions.submit_manifest` (with `dry_run: false`), ALWAYS:

1. **Present a clear summary** of what the transaction will do, including:
   - The action in plain language (e.g. "Transfer 100 tTARI from account A to account B")
   - The manifest source code
   - All variables and their resolved values (component addresses, amounts, etc.)
   - The max fee
   - Which account will sign/pay fees
2. **Ask the user to confirm** before proceeding. Wait for explicit approval.
3. **Do not batch or auto-submit** multiple transactions without per-transaction confirmation.

Dry-run submissions (`dry_run: true`) may be performed without confirmation as they do not modify state.

## Manifest Structure

Every manifest has a `fn main()` block for transaction instructions and an optional `fn fee_main()` block for fee payment. Helper functions may be defined and are inlined at call sites.

```rust
// Optional: import templates by hash
use template_<64-char-hex-hash> as TemplateName;

// Optional: fee instructions (executed first)
fn fee_main() {
    let account = arg!["account"];
    account.pay_fee(1000);
}

// Required: main transaction instructions
fn main() {
    let account = var!["account"];
    let component = var!["component"];

    let result = component.some_method(arg1, arg2);
    account.deposit(result);
}
```

## Core Concepts

### Variables and Arguments

Access runtime variables passed to the manifest via globals:

```rust
let my_var = var!["variable_name"];   // workspace variable
let my_var = arg!["argument_name"];   // alias for var!
let my_var = global!["global_name"];  // alias for var!
```

### Template Function Calls

Call static functions on imported templates (constructors, etc.):

```rust
use template_<hash> as MyTemplate;
let component = MyTemplate::new(arg1, arg2);
```

The `Account` template is always pre-imported.

### Component Method Calls

Call methods on component variables (from `var!` or prior call results):

```rust
let result = component.method_name(arg1, arg2);
component.method_name_no_return(arg1);
```

### Chaining Results

Method call results are stored in workspace variables and can be passed to subsequent calls:

```rust
let bucket = account.withdraw(TARI, 1000);
let item = shop.buy(bucket);
account.deposit(item);
```

## Supported Argument Types

| Type | Syntax | Example |
|------|--------|---------|
| String | `"value"` | `"hello"` |
| Integer | `123u64`, `42i32` | `1_000u64` (unsuffixed defaults to i128) |
| Boolean | `true` / `false` | `true` |
| Amount | `amount!(value)` | `amount!(1000)` |
| Address | `address!("prefix_hex...")` or `address!(var)` | `address!("component_ab12...")` |
| SubstateId | `substate_id!("prefix_hex...")` | `substate_id!("vault_ab12...")` |
| NonFungibleId | `non_fungible_id!("str")`, `non_fungible_id!(1u32)`, `non_fungible_id!(1u64)` | `non_fungible_id!("MyNFT")` |
| Metadata | `metadata!({"key": value, ...})` | `metadata!({"name": "Token", "resource": address!("resource_ab12...")})` |
| PublicKey | `public_key!("hex")` | `public_key!("ab12...cd34")` |
| HexBytes | `hex_bytes!("hex")` | `hex_bytes!("deadbeef")` |
| CBOR | `cbor!(value)` | `cbor!({"key": [1, -2], "owner": address!("component_ab12...")})` |
| List | `[elem, ...]` | `gov.set_council(1u16, [var!["k1"], var!["k2"]])` (elements must not be workspace vars or blobs) |
| Tari token | `TARI` | `account.withdraw(TARI, 100)` (when describing transactions to users, call the token **tTARI** on testnet or **$TARI** on mainnet) |
| Workspace var | bare identifier | `bucket` (from prior `let bucket = ...`) |

## Built-in Macros

```rust
// Address allocation
let addr = new_component_addr!();
let addr = new_resource_addr!();

// Proof management
drop_all_proofs!();
```

## Writing Manifests Workflow

1. **Identify the template and component addresses** the transaction will interact with. These are passed as variables.
2. **Write `fee_main()`** if fees are not handled externally. Typically: `account.pay_fee(max_fee)`.
3. **Write `main()`** with the transaction logic: retrieve variables, call methods, deposit results.
4. **Pass variables** when submitting: map variable names to `ManifestValue` instances (component addresses, resource addresses, amounts, etc.).
5. **Present a summary and ask the user to confirm** before submitting (see "User Confirmation Required" above). Only submit after receiving explicit approval.

## Submitting Manifests via Wallet Daemon JSON-RPC

The wallet daemon exposes a JSON-RPC 2.0 API. All requests are POSTed to the `/json_rpc` endpoint.

### Step 1: Check Auth Method

```bash
curl -s -X POST http://localhost:12008/json_rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"auth.method","params":{}}'
```

Response: `{"result":{"method":"none"}}` or `{"result":{"method":"webauthn"}}`.

### Step 2: Authenticate and Get JWT Token

**Available permissions:** `Admin`, `AccountInfo`, `AccountList`, `AccountBalance`, `KeyList`, `TransactionGet`, `TransactionSend`, `SubstatesRead`, `TemplatesRead`, `GetNft`, `NftGetOwnershipProof`. Use `Admin` for full access.

JWT tokens expire after 5 minutes by default. Refresh with `auth.refresh` or request a new one.

#### When `method` is `"none"`

Authenticate directly with `AuthCredentials::None`:

```bash
curl -s -X POST http://localhost:12008/json_rpc \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":2,"method":"auth.request",
    "params":{"permissions":["Admin"],"credentials":"None"}
  }'
```

Response contains `{"result":{"token":"eyJ..."}}`. Store this JWT token.

#### When `method` is `"webauthn"`

WebAuthn requires a browser-based passkey interaction (biometric, hardware key, etc.) that cannot be performed by an agent directly. Ask the user to provide a JWT token. Instruct them to:

1. Open the wallet daemon web UI in their browser
2. Authenticate with their passkey
3. Copy the JWT token from the browser (e.g. from the `Authorization` header in dev tools, or from the UI if it exposes one)
4. Paste the token back to the agent

Once the user provides the token, use it in the `Authorization: Bearer <token>` header for all subsequent requests. If the token expires, ask the user to provide a fresh one.

### Step 3: Use Token for All Subsequent Requests

Include the JWT in the `Authorization` header:

```bash
curl -s -X POST http://localhost:12008/json_rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer eyJ..." \
  -d '{"jsonrpc":"2.0","id":3,"method":"accounts.list","params":{"offset":0,"limit":10}}'
```

### Step 4: Summarize and Confirm with User

**Before submitting**, present a human-readable summary to the user and wait for explicit confirmation. Example:

> **Transaction Summary**
> - **Action:** Transfer 100 tTARI from source account to destination account
> - **Source:** `component_e61f...bf68` (account "dsfgdfg")
> - **Destination:** `component_1642...c0c1` (account "sdfdf")
> - **Max fee:** 1000
> - **Signing key:** default
>
> Proceed? (yes/no)

Only after the user confirms, submit the manifest.

### Step 5: Submit the Manifest

```bash
curl -s -X POST http://localhost:12008/json_rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer eyJ..." \
  -d '{
    "jsonrpc":"2.0","id":4,"method":"transactions.submit_manifest",
    "params":{
      "manifest": "fn fee_main() {\n  let account = var![\"account\"];\n  account.pay_fee(1000);\n}\n\nfn main() {\n  let account = var![\"account\"];\n  let dest = var![\"dest\"];\n  let bucket = account.withdraw(TARI, 100);\n  dest.deposit(bucket);\n}",
      "variables": {
        "account": "component_e61f...bf68",
        "dest": "component_1642...c0c1"
      },
      "max_fee": 1000,
      "dry_run": false
    }
  }'
```

**Request fields:**
- `manifest` (string): The manifest source code. Newlines as `\n`, inner quotes escaped.
- `variables` (object): Map of variable names to address/value strings (e.g. `component_<hex>`, `resource_<hex>`, `"1000u64"`).
- `signing_key_id` (optional): Override the signing key. Omit to use default account key.
- `max_fee` (u64): Maximum fee in microtari.
- `dry_run` (bool): If `true`, executes without submitting to network. Returns result in response.

**Response:**
```json
{
  "result": {
    "transaction_id": "tx_<hex>",
    "result": null
  }
}
```

`result` is `null` for live submissions (use `transactions.wait_result` to poll). For `dry_run: true`, it contains the `ExecuteResult`.

### Step 6: Wait for Result (optional)

```bash
curl -s -X POST http://localhost:12008/json_rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer eyJ..." \
  -d '{
    "jsonrpc":"2.0","id":5,"method":"transactions.wait_result",
    "params":{"transaction_id":"tx_<hex>","timeout_secs":120}
  }'
```

### Useful Endpoints

| Method | Description |
|--------|-------------|
| `auth.method` | Check auth type (no auth required) |
| `auth.request` | Get JWT token |
| `auth.refresh` | Refresh expired JWT token |
| `accounts.list` | List accounts (`offset`, `limit`) |
| `accounts.get_default` | Get default account |
| `accounts.get_balances` | Get account balances |
| `transactions.submit_manifest` | Submit a manifest transaction |
| `transactions.wait_result` | Wait for transaction finalization |
| `transactions.get_result` | Poll transaction result (non-blocking) |
| `substates.get` | Fetch a substate by address |
| `templates.get` | Fetch template function definitions |

### Via Wallet CLI

```bash
tari_wallet_cli transaction submit-manifest manifest.tm \
  -g account=component_<hex> \
  -g resource=resource_<hex> \
  --max-fee 1000
```

### Programmatically (Rust)

```rust
use tari_transaction_manifest::{parse_manifest, ManifestValue};

let mut globals = HashMap::new();
globals.insert("account".to_string(), ManifestValue::from(account_address));

let instructions = parse_manifest(&manifest_str, globals, HashMap::new())?;
// instructions.instructions     - main transaction instructions
// instructions.fee_instructions - fee payment instructions
```

## Token Naming

The native Tari token is referred to as `TARI` within manifest code. When communicating with users, always use the proper token name:
- **Testnet:** tTARI
- **Mainnet:** $TARI

Never refer to the token as "XTR" in user-facing summaries or descriptions.

## Common Patterns

### Withdraw and Deposit

```rust
fn main() {
    let source = var!["source_account"];
    let dest = var!["dest_account"];
    let resource = var!["resource"];

    let bucket = source.withdraw(resource, 1000);
    dest.deposit(bucket);
}
```

### Create Proof and Call Protected Method

```rust
fn main() {
    let account = var!["account"];
    let admin_badge = var!["admin_badge"];
    let component = var!["component"];

    let proof = account.create_proof_by_amount(Address(admin_badge), 1);
    component.admin_action();
    drop_all_proofs!();
}
```

### Instantiate a Component

```rust
use template_<hash> as MyTemplate;

fn main() {
    let new_component = MyTemplate::new("config_value", 42u64);
}
```

## Additional Resources

### Reference Files

- **`references/manifest-syntax.md`** - Complete syntax reference with all supported constructs and edge cases
- **`references/wallet-daemon-api.md`** - Full wallet daemon JSON-RPC API reference including all auth, transaction, account, and substate endpoints

### Example Files

- **`examples/token_swap.rs`** - Token swap between accounts via a DEX component
- **`examples/initialize_component.rs`** - Component initialization from a template
- **`examples/nft_mint_and_deposit.rs`** - Mint an NFT and deposit to account
