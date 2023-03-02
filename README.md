# Tari DAN implementation

This is where you can find the cutting edge development of the Tari smart contract layer - the Digital Assets 
Network, or DAN.

You can read about the technical specifications of the DAN in the [RFCs](https://rfc.tari.com).

If you're looking for the core Tari base layer code, it's an [this repository](https://github.com/tari-project/tari).

If this is the first time you are executing `cargo build`, you should first run `npm install && npm run build` in the
`tari_indexer_web_ui` repo, as explained in `./applications/tari_indexer_web_ui.README.md`.

## Tari DAN Validator node

See the dedicated [README](./applications/tari_validator_node/README.md) for installation and running guides.

## Tari DAN CLI

A CLI tool to help manage accounts, templates, VNs and transactions on the DAN.

## Tari DAN web-gui

A very basic web-gui for interacting with VNS.

See the dedicated [README](./applications/tari_validator_node_web_ui/README.md) for installation and running guides.

## Running and testing a validator

NOTE: This repo is heavily under development, so these instructions may change without notice.

First thing you need is to run a Tari base node, Tari console wallet and most likely `tor`. You will also need to run a Tari miner to mine some 
blocks. You should use the `feature-dan` branch and `igor` network for now.

A validator node can be started using:

```
cargo run --bin tari_validator_node -- --network igor
``` 

You may find it useful to run multiple validator nodes, in which case, create a subfolder for each one and add the `-b <folder>` to run it in that folder

Example
```
cd vn1 
cargo run --bin tari_validator_node -- -b . --network igor

// other terminal 
cd vn2
cargo run --bin tari_validator_node -- -b . --network igor
```

#### Registering the VN
The validator node web ui can be found at `http://localhost:5000`. When you open it, you can click on `Register VN` to register it. Alternatively, register using the cli:
```
cargo run --bin tari_validator_node_cli -- vn register
```

You'll need to mine a number of blocks, but after that the vn should have a shard key and show up as registered on the web ui

#### Creating a template 

The easiest way to create a template is with the template at https://github.com/tari-project/wasm-template 

```
cargo generate https://github.com/tari-project/wasm-template.git
```

Install the `wasm32-unknown-unknown` target with rustup

```
rustup target add wasm32-unknown-unknown
```

In the directory you created run:

```
cd package
cargo wasm-build
```

Upload the WASM file created in `package/target/wasm32-unknown-unknown/release` to an HTTP server (or IPFS).

After editing the project, you can deploy the project using the validator_node_cli:

```
cargo run --bin tari_validator_node_cli -- templates publish 
```

(See help on tari_validator_node_cli for more details)

Once the template is registered on the base layer and sufficiently mined, you should see it in the `templates` table of the `global_storage.sqlite` file  under `data/validator`. The `compiled_code` column should contain binary data and the `status` column should be `Active`.

### Calling a function
With the templated registered we can invoke a function by using the `validator_node_cli`.

Next we can get a list of templates

```
cargo run --bin tari_validator_node_cli -- templates list
```

Finally, call the function (In this case we'll be calling the `new` function on the example `Counter` template)

```
cargo run --bin tari_validator_node_cli -- transactions submit --wait-for-result call-function <template_address> new 
```

### Debugging Hotstuff 
The first thing that you may want to check is the ingoing and outgoing messages from your node. These messages are logged to a sqlite database under `data/peer_db/message_log.sqlite` to confirm that messages are being sent and received.

If messages are being received, the Hotstuff data can be viewed in the `state.db` under the `data/validator_node` folder.

Each submitted transaction should create a `payload`, so the payload_id is useful to filter the other tables.

If the transaction was successful, there should be some data in the `substates` table with the `created_by_payload_id` or the `deleted_by_payload_id` equal to the `payload_id`.






