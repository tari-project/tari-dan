# Tari Indexer Deployment Guide

## Should You Run Your Own Indexer?

### Reasons to Run Your Own Indexer

**Performance and Latency**
- **Local querying**: Avoid network latency when querying blockchain data
- **Custom caching**: Optimize caching strategies for your specific use patterns
- **Dedicated resources**: No sharing of compute or bandwidth with other users
- **High throughput**: Handle high-frequency queries without rate limiting

**Privacy and Security**
- **Data privacy**: Keep your query patterns and accessed data private
- **Network security**: Reduce attack surface by minimizing external dependencies
- **Audit trail**: Maintain complete logs of all data access and queries
- **Custom filtering**: Implement organization-specific event filtering and data retention

**Control and Customization**
- **Configuration control**: Full control over scanning intervals, storage, and performance tuning
- **Custom APIs**: Extend or modify APIs to meet specific application requirements
- **Data retention**: Implement custom data retention and archival policies
- **Integration**: Deep integration with internal systems and workflows

**Reliability and Availability**
- **Service availability**: Eliminate dependency on third-party indexer services
- **Geographic distribution**: Deploy regionally for better global access
- **Disaster recovery**: Implement custom backup and recovery procedures
- **SLA control**: Meet specific uptime and performance requirements

**Development and Testing**
- **Development environment**: Full control over test data and network state
- **Debugging capabilities**: Access to detailed logs and internal state for troubleshooting
- **Feature development**: Prototype new features and integrations locally
- **Historical analysis**: Access to complete historical data for analytics

### Reasons Not to Run Your Own Indexer

**Operational Overhead**
- **Infrastructure management**: Requires ongoing server maintenance, monitoring, and updates
- **Database administration**: SQLite database maintenance, backup, and recovery procedures
- **Network management**: P2P networking configuration and peer management
- **Security maintenance**: Regular security updates and vulnerability management

**Resource Requirements**
- **Storage costs**: Requires significant disk space for complete blockchain history
- **Bandwidth usage**: Continuous network traffic for syncing with validator nodes
- **Compute resources**: CPU and memory requirements for real-time data processing
- **Monitoring infrastructure**: Additional systems for health monitoring and alerting

**Technical Complexity**
- **Configuration management**: Complex configuration with many interdependent settings
- **Network topology**: Understanding of Tari network architecture and shard management
- **Troubleshooting**: Debugging network sync issues, database corruption, or performance problems
- **Version management**: Keeping up with network upgrades and compatibility requirements

**Cost Considerations**
- **Infrastructure costs**: Server hosting, storage, and bandwidth expenses
- **Operational costs**: Staff time for maintenance, monitoring, and incident response
- **Development costs**: Time investment for setup, customization, and integration
- **Opportunity cost**: Resources that could be used for core application development

## Deployment Architecture Options

### Single Instance Deployment

**Best for**: Development, testing, small applications, or single-team usage

```
┌─────────────────────────────────────────────────────────────┐
│                      Single Server                         │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Tari Indexer                           │   │
│  │                                                     │   │
│  │  ┌─────────────┐  ┌─────────────┐                  │   │
│  │  │   REST API  │  │   Web UI    │                  │   │
│  │  │  Port 18300 │  │ Port 15000  │                  │   │
│  │  └─────────────┘  └─────────────┘                  │   │
│  │                                                     │   │
│  │  ┌─────────────────────────────────────────────────┐ │   │
│  │  │              SQLite Database                    │ │   │
│  │  │            (Complete History)                   │ │   │
│  │  └─────────────────────────────────────────────────┘ │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Pros:**
- Simple setup and configuration
- Lower resource requirements
- Easy to debug and monitor
- Cost-effective for smaller workloads

**Cons:**
- Single point of failure
- Limited scalability
- Performance bottlenecks under high load

### High Availability Deployment

**Best for**: Production applications requiring uptime guarantees

```
┌──────────────────────┐    ┌──────────────────────┐
│    Load Balancer     │    │    Load Balancer     │
│     (Primary)        │    │     (Backup)         │
└──────────┬───────────┘    └──────────┬───────────┘
           │                           │
           ▼                           ▼
┌─────────────────┐              ┌─────────────────┐
│  Indexer Node 1 │              │  Indexer Node 2 │
│                 │              │                 │
│  ┌───────────┐  │              │  ┌───────────┐  │
│  │   APIs    │  │              │  │   APIs    │  │
│  └───────────┘  │              │  └───────────┘  │
│  ┌───────────┐  │              │  ┌───────────┐  │
│  │ Database  │  │◄────────────►│  │ Database  │  │
│  │(Primary)  │  │   Sync       │  │(Replica)  │  │
│  └───────────┘  │              │  └───────────┘  │
└─────────────────┘              └─────────────────┘
```

**Features:**
- Active-passive failover configuration
- Database replication for data consistency
- Health monitoring and automatic failover
- Geographic distribution capabilities

### Microservices Deployment

**Best for**: Large-scale applications with diverse requirements

```
┌─────────────────────────────────────────────────────────────────────┐
│                          Kubernetes Cluster                        │
│                                                                     │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐    │
│  │   API Gateway   │  │   Data Ingress  │  │   Web UI        │    │
│  │   (Multiple     │  │   (Scanner +    │  │   (Static       │    │
│  │   Replicas)     │  │   Processor)    │  │   Serving)      │    │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘    │
│           │                     │                     │            │
│           ▼                     ▼                     ▼            │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐    │
│  │   Database      │  │   Cache Layer   │  │   Storage       │    │
│  │   (PostgreSQL)  │  │   (Redis)       │  │   (Persistent   │    │
│  │                 │  │                 │  │   Volumes)      │    │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

## System Requirements

### Minimum Requirements

**Development/Testing Environment:**
- **CPU**: 2 cores (x86_64)
- **RAM**: 4 GB
- **Storage**: 50 GB SSD (initial), grows over time
- **Network**: 10 Mbps upload/download
- **OS**: Linux (Ubuntu 20.04+), macOS 10.15+, Windows 10+

### Recommended Requirements

**Production Environment:**
- **CPU**: 4+ cores (x86_64)
- **RAM**: 8-16 GB
- **Storage**: 200+ GB NVMe SSD (with growth planning)
- **Network**: 100+ Mbps dedicated bandwidth
- **OS**: Linux (Ubuntu 22.04+ or RHEL 8+)

### Storage Planning

**Database Growth Estimates:**
- **Initial sync**: 1-5 GB (depending on network age)
- **Daily growth**: 100MB - 1GB (depending on network activity)
- **Substate cache**: 5-20 GB (configurable, LRU eviction)
- **Logs**: 100MB - 1GB per day (configurable retention)

**Disk I/O Requirements:**
- **Random reads**: High (for query workloads)
- **Sequential writes**: Medium (for blockchain data ingestion)
- **IOPS**: 1000+ recommended for production

### Network Requirements

**Bandwidth Usage:**
- **Initial sync**: 10-50 GB (one-time)
- **Ongoing sync**: 10-100 MB/hour (varies with network activity)
- **API serving**: Variable (depends on client usage)

**Port Requirements:**
- **P2P listener**: Configurable (default: varies by config)
- **REST API**: Configurable (default: 18300)
- **Web UI**: Configurable (default: 15000)

## Security Considerations

### Network Security

**Firewall Configuration:**
```bash
# Allow P2P networking (adjust port as needed)
ufw allow [P2P_PORT]

# Allow API access (restrict to trusted networks)
ufw allow from [TRUSTED_NETWORK] to any port 18300

# Allow Web UI access (optional, restrict as needed)
ufw allow from [TRUSTED_NETWORK] to any port 15000
```

**P2P Security:**
- Use encrypted connections for all P2P communication
- Implement peer allowlisting for sensitive environments
- Monitor for unusual peer behavior or connection patterns
- Regular rotation of P2P identity keys

### Data Security

**Database Protection:**
- Secure file system permissions on database files
- Regular backup encryption and offsite storage
- Access logging for all database operations
- Database integrity verification procedures

**API Security:**
- Implement rate limiting to prevent abuse
- Use HTTPS for all API endpoints in production
- Consider API authentication for sensitive data
- Monitor for unusual query patterns

### Infrastructure Security

**Server Hardening:**
- Regular security updates for OS and dependencies
- Disable unnecessary services and ports
- Implement intrusion detection systems
- Use configuration management for consistent security

**Monitoring and Alerting:**
- Real-time monitoring of system health
- Alerts for unusual network or database activity  
- Log aggregation and analysis
- Incident response procedures

## Monitoring and Maintenance

### Health Monitoring

**Key Metrics:**
- **Sync status**: Block height lag from network head
- **Database performance**: Query response times and throughput
- **P2P connectivity**: Connected peer count and quality
- **Resource usage**: CPU, memory, disk, and network utilization
- **API performance**: Request rates and error rates

**Recommended Tools:**
- **System monitoring**: Prometheus + Grafana, DataDog, or New Relic
- **Log management**: ELK Stack, Splunk, or Fluentd
- **Database monitoring**: Built-in SQLite monitoring or custom scripts
- **Network monitoring**: libp2p metrics and custom P2P analytics

### Maintenance Procedures

**Regular Tasks:**
- **Database maintenance**: Vacuum, analyze, and integrity checks
- **Log rotation**: Automated cleanup of old log files
- **Cache cleanup**: Periodic cleanup of substate cache
- **Backup verification**: Regular testing of backup and restore procedures

**Upgrade Procedures:**
- **Version testing**: Test new versions in staging environment
- **Database migrations**: Plan and test database schema migrations
- **Configuration updates**: Review and update configuration as needed
- **Rollback planning**: Maintain procedures for reverting problematic upgrades

### Troubleshooting

**Common Issues:**
- **Sync lag**: Network connectivity or validator node issues
- **Database corruption**: Hardware failure or improper shutdown
- **P2P connectivity**: Firewall or NAT traversal problems
- **Performance degradation**: Resource exhaustion or inefficient queries

**Diagnostic Tools:**
- **Built-in logs**: Comprehensive logging at multiple levels
- **Database tools**: SQLite command-line interface for direct inspection
- **Network tools**: P2P connection debugging and peer analysis
- **Performance profiling**: CPU and memory profiling tools

## Cost Analysis

### Infrastructure Costs (Monthly Estimates)

**Cloud Deployment (AWS/GCP/Azure):**
- **Development**: $50-150 (small instance, basic storage)
- **Production**: $200-800 (medium-large instance, SSD storage, bandwidth)
- **High Availability**: $500-1500 (multiple instances, load balancer, backup)

**Self-Hosted:**
- **Hardware**: $200-2000 (one-time, varies by requirements)
- **Hosting**: $50-300 (colo, power, bandwidth)
- **Maintenance**: $500-2000 (staff time, varies by scale)

### Total Cost of Ownership

**First Year Costs:**
- **Setup time**: 20-80 hours (depending on complexity)
- **Ongoing operations**: 2-10 hours/month
- **Infrastructure**: As above
- **Total**: $5,000-25,000+ (highly variable by scale and requirements)

**Break-Even Analysis:**
- **Third-party service costs**: $100-1000/month (typical for API services)
- **Custom requirements**: Often justifies cost for specialized needs
- **Scale factor**: Costs become more favorable at larger scales