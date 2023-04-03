# Tari DAN implementation

This is where you can find the cutting edge development of the Tari smart contract layer - the Digital Assets 
Network, or DAN.

You can read about the technical specifications of the DAN in the [RFCs](https://rfc.tari.com).

If you're looking for the core Tari base layer code, it's an [this repository](https://github.com/tari-project/tari)

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

#### Running the Tari Dan Wallet Daemon
To be able to use the Tari Dan Wallet CLI communicate with the running validator node, one needs to set up a tari dan wallet daemon,
as follows:
```
cargo run --bin tari_dan_wallet_daemon -- -b .
```

The wallet daemon will listen to wallet requests and submit it to the running VN. Notice that, each VN running should have
its own wallet daemon. 

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

### Claiming L1 burn Tari on the DAN

The user will need to claim burn tari (from the L1) before submitting transactions, otherwise, it will not be
able to pay for network fees. To be able to claim burn tari, it is necessary to first burn L1 tari, on the base layer. After
burning tari on the Tari base layer, the client is prompted with the following data

```
{
    "transaction_id": <transaction_id>,
    "is_success": <IS_SUCCESS>,
    "failure_message": <FAILURE_MESSAGE>,
    "commitment": <commitment>,
    "ownership_proof": <ownership_proof>,
    "rangeproof": <rangeproof>
}
```

Now, it is possible to claim burn Tari on the DAN layer, as follows. Create a new `.json` file, with path
`<json_file_to_retrieve_burn_tari>`

```
{
    "claim_public_key": <claim_public_key>,
    "transaction_id": <transaction_id>,
    "commitment": <commitment>,
    "ownership_proof": <ownership_proof>,
    "rangeproof": <rangeproof>
}
```

where `<claim_public_key>` is the claim public key that it was provided to the tari console wallet to burn base layer Tari,
in the first place.

```
cargo run --bin tari_dan_wallet_cli --claim-burn --account <account_name> --input <json_file_to_retrieve_burn_tari> --fee 1
```

where `<account_name>` is the user account name, `<json_file_to_retrieve_burn_tari>` is the json generate previously.


### Calling a function
With the templated registered we can invoke a function by using the `tari_dan_wallet_cli`.

Next we can get a list of templates

```
cargo run --bin tari_validator_node_cli -- templates list
```

To be entitled to pay for network fees, the user will have to claim burn Tari, see the previous section.
Finally, call the function (In this case we'll be calling the `new` function on the example `Counter` template)

```
cargo run --bin tari_dan_wallet_cli -- transactions submit --wait-for-result call-function <template_address> new 
```

### Debugging Hotstuff 
The first thing that you may want to check is the ingoing and outgoing messages from your node. These messages are logged to a sqlite database under `data/peer_db/message_log.sqlite` to confirm that messages are being sent and received.

If messages are being received, the Hotstuff data can be viewed in the `state.db` under the `data/validator_node` folder.

Each submitted transaction should create a `payload`, so the payload_id is useful to filter the other tables.

If the transaction was successful, there should be some data in the `substates` table with the `created_by_payload_id` or the `deleted_by_payload_id` equal to the `payload_id`.






