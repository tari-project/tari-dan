# Tari Indexer Architecture

## High-Level Architecture

The Tari Indexer is designed as a modular, event-driven system that bridges the distributed Tari DAN network with
applications requiring centralized data access.

```
┌───────────────────────────────────────────────────────────────┐
│                                 Ootle Network                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │ Validator    │  │ Validator    │  │ Validator    │         │
│  │ Node         │  │ Node         │  │ Node         │         │
│  │ (Shard A)    │  │ (Shard B)    │  │ (Shard C)    │         │
│  └──────────────┘  └──────────────┘  └──────────────┘         │
└───────────────────────────────────────────────────────────────┘
                     │                    │                    
                     ▼                    ▼                    
┌───────────────────────────────────────────────────────────────┐
│                          Indexer                              │
│                                                               │
└───────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌────────────────────────────────────────────────────────────────────────┐
│                        Client Applications                             │
│                                                                        │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │   Wallets   │  │    DApps    │  │   Block     │  │  Analytics  │    │
│  │             │  │             │  │  Explorers  │  │    Tools    │    │
│  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘    │
└────────────────────────────────────────────────────────────────────────┘
```

## Core Components

### Network Interface Layer

**P2P Networking Service**

- Manages connections to validator nodes across the network
- Handles peer discovery, connection management, and message routing
- Supports multiple transport protocols (TCP, QUIC)
- Implements relay and rendezvous capabilities for NAT traversal

**Epoch Oracle System**

- **Base Layer Oracle**: Monitors the Minotari base layer for epoch changes
- **Configured Oracle**: Uses predefined epoch configurations
- **Hybrid Oracle**: Combines base layer monitoring with configuration fallbacks
- Provides authoritative source for network epoch information

**Epoch Manager**

- Tracks current and historical network epochs
- Manages validator committee composition changes
- Handles shard assignments and validator node routing
- Coordinates with template manager for epoch-specific templates

### Data Processing Layer

**Block Scanner**

- Continuously polls validator nodes for new blocks and events
- Implements configurable scanning intervals to balance load and freshness
- Handles network partitions and temporary validator node unavailability
- Processes blocks in order to maintain consistency

**Network State Sync**

- Synchronizes state across multiple validator nodes and shards
- Implements intelligent sync strategies to minimize network overhead
- Handles state reconciliation when inconsistencies are detected
- Supports event filtering to focus on relevant data

**Event Manager**

- Processes and categorizes all network events
- Maintains event ordering and relationships
- Supports real-time event streaming to connected clients
- Implements configurable event filtering and routing

**Substate Manager**

- Manages the lifecycle of substates (blockchain state objects)
- Tracks substate versions, ownership, and state transitions
- Implements efficient querying and retrieval mechanisms
- Coordinates with substate cache for performance optimization

**Transaction Manager**

- Processes and stores transaction data and metadata
- Extracts transaction results, logs, and execution details
- Maintains transaction-substate relationships
- Supports transaction dry-run capabilities

**Template Manager**

- Downloads and manages smart contract templates
- Maintains template metadata and deployment history
- Handles template versioning and updates
- Integrates with epoch manager for template availability

### Storage Layer

**SQLite Indexer Store**

- Primary database for all indexed network data
- Optimized schema for fast queries and analytical workloads
- Maintains full historical data with efficient indexing
- Supports atomic transactions and data consistency

Tables include:

- **Transactions**: Complete transaction records with metadata
- **Substates**: Current and historical substate versions
- **Events**: All network events with categorization
- **UTXOs**: Unspent transaction outputs with tracking
- **Key-Value**: Configuration and metadata storage

**Global Database**

- Shared database for epoch and validator information
- Stores network-wide metadata and configuration
- Manages template registry and download queue
- Handles cross-epoch data consistency

**Substate File Cache**

- File-system based cache for frequently accessed substates
- Implements LRU eviction policies for memory management
- Provides fast access to large substate data
- Handles cache invalidation on substate updates

### API & Interface Layer

**REST API Server**

- Traditional HTTP endpoints for programmatic access
- Comprehensive endpoint coverage for all data types
- Built-in rate limiting and request validation
- Swagger/OpenAPI documentation generation
- WebSocket support for real-time UTXO streaming

**Web UI Server**

- Serves the React-based web interface
- Static file serving with optimized caching
- Configuration injection for API endpoints
- Responsive design for multiple device types

**Dry Run Processor**

- Simulates transaction execution without network commitment
- Uses live network state for accurate simulation
- Provides detailed execution traces and gas estimates
- Supports debugging and development workflows

## Data Flow Architecture

### Ingestion Pipeline

1. **Network Discovery**: P2P layer discovers and connects to validator nodes
2. **Block Scanning**: Scanner identifies new blocks and events across shards
3. **Data Extraction**: Raw block data is parsed and validated
4. **Event Processing**: Events are categorized and relationships established
5. **Storage**: Processed data is atomically committed to the database
6. **Indexing**: Database indexes are updated for efficient querying
7. **Cache Updates**: File cache is updated with new substate data
8. **Event Emission**: Real-time events are emitted to connected clients

### Query Pipeline

1. **Request Routing**: API layer routes requests to appropriate handlers
2. **Query Optimization**: Queries are analyzed and optimized for performance
3. **Data Retrieval**: Data is fetched from database with appropriate joins
4. **Cache Integration**: Cached substates are integrated into results
5. **Response Formatting**: Results are formatted according to API specification
6. **Client Delivery**: Formatted response is delivered to the client

### Real-time Pipeline

1. **Event Detection**: New network events are immediately detected
2. **Event Filtering**: Events are filtered based on client subscriptions
3. **Event Transformation**: Events are transformed to client-expected format
4. **WebSocket Delivery**: Events are pushed to connected WebSocket clients
5. **State Synchronization**: Client state is kept synchronized with network

## Scalability Considerations

### Horizontal Scaling

While the indexer is designed as a single-instance service, it can be scaled horizontally by:

- **Read Replicas**: Multiple read-only instances can share the same data
- **Shard Specialization**: Different instances can focus on specific shards
- **API Load Balancing**: Multiple API servers can share the same database
- **Geographic Distribution**: Regional instances can serve local clients

### Performance Optimization

- **Database Indexing**: Comprehensive indexing strategy for all query patterns
- **Connection Pooling**: Efficient management of database and network connections
- **Caching Layers**: Multi-level caching from memory to disk
- **Batch Processing**: Bulk operations for improved throughput
- **Async Processing**: Non-blocking I/O for all network and storage operations

### Resource Management

- **Memory Management**: Careful memory usage with periodic garbage collection
- **Disk Usage**: Automatic cleanup of old cache files and logs
- **Network Bandwidth**: Intelligent peer selection and request batching
- **CPU Utilization**: Multi-threaded processing with work-stealing queues