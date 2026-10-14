# Tari Indexer Overview

## What is the Tari Indexer?

The Tari Indexer is a specialized service that monitors, indexes, and provides access to data from the Tari Digital Assets Network (DAN). It serves as a bridge between the decentralized network and applications that need to query blockchain data efficiently.

## Purpose and Role

The Tari DAN is a distributed network where data is sharded across multiple validator nodes. While this provides scalability and decentralization, it makes it challenging for applications to:

- **Query data across shards**: Different pieces of information may exist on different validator nodes
- **Track historical changes**: The network focuses on current state, not historical data
- **Perform complex queries**: Validator nodes are optimized for consensus, not data queries
- **Access data reliably**: Applications need consistent, fast access to blockchain data

The Tari Indexer solves these problems by:

1. **Aggregating data** from across the sharded network into a single queryable database
2. **Maintaining history** of all transactions, substates, and events
3. **Providing multiple APIs** (REST, Web UI) for different use cases
4. **Caching substates** locally for fast retrieval
5. **Streaming real-time updates** to connected applications

## What Does the Indexer Index?

The indexer continuously scans and stores several types of data:

### Transactions
- Transaction details and metadata
- Transaction results and outcomes  
- Fee information
- Execution logs and events

### Substates
- Current and historical substate versions
- Substate changes and diffs
- Resource and component states
- UTXO tracking

### Templates
- Smart contract templates and their metadata
- Template instantiations and usage
- NFT collections and individual tokens

### Network Events
- Block processing events
- Consensus events
- P2P network events
- System events and logs

### Resources
- Fungible and non-fungible tokens
- Resource metadata and properties
- Ownership and transfer history
- Vault states

## Key Features

### Data Access APIs

- **REST API**: Traditional HTTP endpoints for programmatic access
- **Web UI**: Browser-based interface for exploring network data
- **UTXO Streaming**: Real-time WebSocket streaming of UTXO changes

### Advanced Capabilities

- **Cross-shard querying**: Aggregate data from multiple network shards
- **Historical analysis**: Access to complete transaction and state history  
- **Event filtering**: Configurable filters to track specific types of events
- **Caching**: Local caching of frequently accessed substates for performance
- **Dry run execution**: Test transactions without committing to the network

### Network Integration

- **P2P networking**: Direct communication with validator nodes
- **Epoch management**: Tracks network epochs and validator committee changes
- **Template management**: Automatic downloading and management of smart contract templates
- **Multi-oracle support**: Flexible epoch oracle configuration (base layer, configured, hybrid)

## Use Cases

The Tari Indexer enables various applications and use cases:

### Wallets and DApps
- Query account balances and transaction history
- Track NFT ownership and metadata
- Monitor smart contract interactions
- Stream real-time balance updates

### Analytics and Monitoring
- Analyze network usage patterns
- Track token transfers and trading activity
- Monitor template deployment and usage
- Generate reports on network health

### Development Tools
- Debug smart contract interactions
- Test transaction execution with dry runs
- Explore network state for development
- Integration testing with historical data

### Block Explorers
- Browse transactions and blocks
- Search for specific addresses or transactions  
- Visualize network activity and statistics
- Provide public access to network data

## Data Flow

1. **Network Scanning**: The indexer continuously scans validator nodes for new blocks and events
2. **Data Processing**: Raw network data is processed, validated, and structured
3. **Storage**: Processed data is stored in a local SQLite database with full history
4. **API Serving**: Multiple APIs provide access to the indexed data
5. **Real-time Updates**: WebSocket connections stream live updates to connected clients

The indexer acts as a specialized caching and query layer that makes the distributed Tari network accessible to traditional applications while preserving the decentralized nature of the underlying blockchain.