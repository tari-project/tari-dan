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


