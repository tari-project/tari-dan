# Tari Indexer

The Tari Indexer is a service that indexes and tracks substates, transactions, and events on the Tari DAN (Digital Assets Network). It provides a REST API for querying blockchain data and includes a web-based user interface for exploring the network.

## Documentation

Comprehensive documentation is available in the [`docs/`](docs/) directory:

- **[Overview](docs/overview.md)**: What the indexer is and why you need it
- **[Architecture](docs/architecture.md)**: High-level system design and component breakdown  
- **[Deployment Guide](docs/deployment.md)**: Should you run your own indexer? Deployment options and requirements
- **[Configuration](docs/configuration.md)**: Complete configuration reference and examples

## Overview

The indexer continuously scans the DAN layer for new blocks, transactions, and substates, storing this information in a local SQLite database. It provides multiple interfaces for accessing this data:

- **REST API**: RESTful endpoints for programmatic access
- **Web UI**: Browser-based interface for exploring network data
- **UTXO Streaming**: Real-time streaming of UTXO updates

## Features

- Real-time blockchain data indexing
- REST API
- Web-based user interface
- UTXO streaming capabilities
- P2P networking for validator node communication
- Configurable scanning intervals
- Template and NFT tracking
- Transaction and event monitoring

## Building

```bash
cargo build --release
```

### Features

- `web_ui` (default): Includes the web-based user interface
- Without web UI: `cargo build --release --no-default-features`

## Configuration

The indexer uses configuration files and environment variables. Key configuration sections:

- **API Endpoints**: Configure the REST server address
- **P2P Networking**: Set peer seeds, ports, and reachability modes
- **Database**: SQLite storage configuration
- **Scanning**: Block scanning intervals and filters
- **Web UI**: Public API URLs for the web interface

## Running

### Basic Usage

```bash
./target/release/tari_indexer
```

### Command Line Options

The following options are available (use `tari_indexer --help` for the most up-to-date list):

#### Core Options
- `-h, --help`: Print help information
- `-V, --version`: Print version information

#### Configuration
- `-c, --config <CONFIG>`: Path to configuration file (default: `config/config.toml`)
- `-b, --base-path <BASE_PATH>`: Directory to store files [env: `TARI_BASE_DIR`] (default: `~/.tari`)
- `--network <NETWORK>`: Override network configuration [env: `TARI_NETWORK`]
- `-p <CONFIG_PROPERTY_OVERRIDES>`: Override config properties using fully qualified keys

#### Logging
- `-l, --log-config <LOG_CONFIG>`: Path to log configuration file
- `--log-path <LOG_PATH>`: Directory for log storage

#### API Server
- `-r, --api-listen-address <API_LISTEN_ADDRESS>`: Bind address for REST API server
- `--web-ui-public-api-url <WEB_UI_PUBLIC_API_URL>`: Public API URL for web UI [env: `TARI_INDEXER_WEB_UI_PUBLIC_API_URL`]

#### Blockchain Scanning
- `-a, --address <ADDRESS>...`: Substate addresses to track (can be specified multiple times)
- `-i, --scanning-interval <SCANNING_INTERVAL>`: Block scanning interval in seconds

#### P2P Networking
- `-s, --peer-seeds <PEER_SEEDS>`: P2P peer seeds (can be specified multiple times)
- `--listener-port <LISTENER_PORT>`: Port to listen on for P2P connections
- `--reachability <REACHABILITY>`: P2P reachability mode (reachable/unreachable)
- `--disable-mdns`: Disable mDNS peer discovery

#### Epoch Oracle
- `-g, --epoch-oracle-minotari-node-grpc-url <URL>`: Minotari node gRPC URL [env: `TARI_INDEXER_MINOTARI_NODE_GRPC_URL`]
- `--epoch-oracle-config <EPOCH_ORACLE_CONFIG>`: Path to epoch oracle configuration file

### Example Usage

```bash
# Run with custom API address and peer seeds
./target/release/tari_indexer \
    --api-listen-address 0.0.0.0:8080 \
    --peer-seeds "peer1::address1" \
    --peer-seeds "peer2::address2" \
    --scanning-interval 10

# Run with environment variables
TARI_INDEXER_WEB_UI_PUBLIC_API_URL=http://localhost:8080 \
TARI_INDEXER_MINOTARI_NODE_GRPC_URL=http://localhost:18142 \
./target/release/tari_indexer

# Run with custom configuration
./target/release/tari_indexer \
    --config /path/to/custom/config.toml \
    --base-path /custom/data/directory
```

## APIs

### REST API

When running, the REST API is available at the configured address (default: `http://localhost:8080`). Key endpoints include:

- `/templates` - Template information
- `/transactions` - Transaction data
- `/substates` - Substate queries
- `/nfts` - NFT tracking
- `/utxos/stream` - UTXO streaming
- `/network` - Network information

Swagger documentation is available at `/swagger-ui/` when the server is running.

### Web UI

The web interface (when enabled) provides:
- Network overview and statistics
- Transaction browsing and search
- Substate exploration
- Event monitoring
- Resource and NFT tracking

## Database

The indexer uses SQLite for local storage, automatically creating and migrating the database schema on startup. The database location is configurable through the data directory setting.

## Development

### Web UI Development

The web UI is a React application located in the `web_ui/` directory:

```bash
cd web_ui/
npm install
npm run dev  # Development server
npm run build  # Production build
```

### Testing

Run the test suite:

```bash
cargo test
```

## Architecture

The indexer consists of several key components:

- **Block Scanner**: Continuously scans for new blocks and events
- **Storage Layer**: SQLite-based data persistence
- **REST Server**: HTTP API endpoints
- **P2P Networking**: Communication with validator nodes
- **Web UI Server**: Static file serving for the web interface

## License

Copyright 2023, The Tari Project. Licensed under BSD 3-Clause.