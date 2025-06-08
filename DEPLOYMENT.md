# CivicJournal-Time Deployment Guide

**Version**: 0.4.0

**Last Updated**: 2025-06-08

## Table of Contents

1. [System Requirements](#system-requirements)
2. [Installation](#installation)
3. [Configuration](#configuration)
4. [Building from Source](#building-from-source)
5. [Docker Deployment](#docker-deployment)
6. [Kubernetes Deployment](#kubernetes-deployment)
7. [Backup and Restore](#backup-and-restore)
8. [Monitoring and Logging](#monitoring-and-logging)
9. [Performance Tuning](#performance-tuning)
10. [Troubleshooting](#troubleshooting)
11. [Upgrading](#upgrading)

## System Requirements

### Minimum Requirements
Validated with Demo Mode and benchmark runs:
- **CPU**: 2 cores (4+ recommended for production)
- **RAM**: 4GB (8GB+ recommended for production)
- **Storage**: SSD strongly recommended
  - At least 2x expected data size for optimal performance
  - Consider compression for long-term storage
- **OS**: Linux, macOS, or Windows (Linux recommended for production)
- **Rust**: 1.70+ (for building from source)

### Recommended Production Setup
- **CPU**: 4+ cores
- **RAM**: 16GB+
- **Storage**: NVMe SSD with at least 100GB free space
- **Filesystem**: XFS or ext4 (for Linux)
- **Network**: 1Gbps+ network interface

## Installation

### Pre-built Binaries

1. Download the latest release for your platform from the [GitHub releases page](https://github.com/COGSLLC/civicjournal-time/releases)

2. Extract the archive:
   ```bash
   tar -xzf civicjournal-time-x86_64-linux-0.4.0.tar.gz
   cd civicjournal-time-0.4.0/
   ```

3. Make the binary executable:
   ```bash
   chmod +x civicjournal-time
   ```

### Cargo Install

```bash
cargo install --git https://github.com/COGSLLC/civicjournal-time.git --tag v0.4.0
```

## Configuration

### Configuration File

Create a `config.toml` file with your settings. Here's a complete example:

```toml
[time_hierarchy]
levels = [
    { name = "minute", duration_seconds = 60 },
    { name = "hour", duration_seconds = 3600 },
    { name = "day", duration_seconds = 86400 },
    { name = "month", duration_seconds = 2592000 },
    { name = "year", duration_seconds = 31536000 },
    { name = "decade", duration_seconds = 315360000 },
    { name = "century", duration_seconds = 3153600000 },
]

[rollup]
# Configuration for each level (index corresponds to time_hierarchy.levels - 1)
[[rollup.levels]]
max_items_per_page = 1000
max_page_age_seconds = 3600  # 1 hour
content_type = "ChildHashes"  # or "NetPatches"

[[rollup.levels]]
max_items_per_page = 500
max_page_age_seconds = 86400  # 1 day
content_type = "ChildHashes"

# Add more levels as needed...

[storage]
type = "file"  # or "memory" for testing
base_path = "/var/lib/civicjournal"
max_open_files = 1000

[compression]
enabled = true
algorithm = "zstd"  # "zstd", "lz4", "snappy", or "none"
level = 3  # 1-22 for zstd, higher = better compression but slower

[retention]
enabled = true
period_seconds = 2592000  # 30 days
cleanup_interval_seconds = 86400  # 1 day

[logging]
level = "info"  # "trace", "debug", "info", "warn", "error"
console = true
file = true
file_path = "/var/log/civicjournal/civicjournal.log"
file_rotation = "daily"  # "hourly", "daily", or "never"
```

### Environment Variables

You can override any configuration value using environment variables:

```bash
CIVICJOURNAL_STORAGE_BASE_PATH=/data/civicjournal \
CIVICJOURNAL_LOGGING_LEVEL=info \
CIVICJOURNAL_ROLLUP_LEVELS_0_MAX_ITEMS_PER_PAGE=2000 \
civicjournal-time
```

## Building from Source

1. Clone the repository:
   ```bash
   git clone https://github.com/COGSLLC/civicjournal-time.git
   cd civicjournal-time
   git checkout v0.4.0  # or the desired version
   ```

2. Build in release mode:
   ```bash
   cargo build --release
   ```

3. The binary will be available at `target/release/civicjournal-time`

## Docker Deployment

### Quick Start

```bash
docker run -d \
  --name civicjournal \
  -p 8080:8080 \
  -v /path/to/config.toml:/etc/civicjournal/config.toml \
  -v /path/to/data:/data \
  ghcr.io/cogsllc/civicjournal-time:0.4.0
```

### Docker Compose

Create a `docker-compose.yml` file:

```yaml
version: '3.8'

services:
  civicjournal:
    image: ghcr.io/cogsllc/civicjournal-time:0.4.0
    container_name: civicjournal
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - ./config.toml:/etc/civicjournal/config.toml
      - ./data:/data
    environment:
      - RUST_LOG=info
    deploy:
      resources:
        limits:
          cpus: '2'
          memory: 4G
```

Start the service:
```bash
docker-compose up -d
```

## Kubernetes Deployment

### Helm Chart

1. Add the Helm repository:
   ```bash
   helm repo add civicjournal https://cogsllc.github.io/civicjournal-time/charts
   helm repo update
   ```

2. Install the chart:
   ```bash
   helm install civicjournal civicjournal/civicjournal-time \
     --namespace civicjournal \
     --create-namespace \
     --set storage.persistentVolume.enabled=true \
     --set storage.persistentVolume.size=100Gi \
     --set resources.requests.cpu=2 \
     --set resources.requests.memory=4Gi \
     --set resources.limits.cpu=4 \
     --set resources.limits.memory=8Gi
   ```

### Custom Configuration

Create a `values.yaml` file:

```yaml
replicaCount: 3

image:
  repository: ghcr.io/cogsllc/civicjournal-time
  tag: 0.4.0
  pullPolicy: IfNotPresent

service:
  type: ClusterIP
  port: 8080

resources:
  requests:
    cpu: 2
    memory: 4Gi
  limits:
    cpu: 4
    memory: 8Gi

storage:
  persistentVolume:
    enabled: true
    size: 100Gi
    storageClass: ""
    accessModes: [ "ReadWriteOnce" ]

config: |
  [time_hierarchy]
  levels = [
      { name = "minute", duration_seconds = 60 },
      { name = "hour", duration_seconds = 3600 },
      { name = "day", duration_seconds = 86400 },
      { name = "month", duration_seconds = 2592000 },
      { name = "year", duration_seconds = 31536000 },
      { name = "decade", duration_seconds = 315360000 },
      { name = "century", duration_seconds = 3153600000 },
  ]
  
  [rollup]
  [[rollup.levels]]
  max_items_per_page = 1000
  max_page_age_seconds = 3600
  content_type = "ChildHashes"
  
  # Additional configuration...
```

Install with custom values:
```bash
helm install civicjournal civicjournal/civicjournal-time -f values.yaml
```

## Backup and Restore

### Creating Backups

```bash
# Using the CLI
civicjournal-time backup /path/to/backup.zip

# Using the API
curl -X POST http://localhost:8080/api/v1/backup -d '{"path":"/path/to/backup.zip"}'
```

### Restoring from Backup

```bash
# Using the CLI
civicjournal-time restore /path/to/backup.zip

# Using the API
curl -X POST http://localhost:8080/api/v1/restore -d '{"path":"/path/to/backup.zip"}'
```

### Automated Backups

Example cron job for daily backups:

```bash
# Daily backup at 2 AM
0 2 * * * /usr/local/bin/civicjournal-time backup /backups/civicjournal-$(date +\%Y\%m\%d).zip
```

## Monitoring and Logging

### Logging

Logs are written to the console and optionally to a file. Configure in `config.toml`:

```toml
[logging]
level = "info"  # "trace", "debug", "info", "warn", "error"
console = true
file = true
file_path = "/var/log/civicjournal/civicjournal.log"
file_rotation = "daily"  # "hourly", "daily", or "never"
```

### Metrics

CivicJournal-Time exposes Prometheus metrics at `/metrics`:

```bash
# Scrape config for Prometheus
scrape_configs:
  - job_name: 'civicjournal'
    static_configs:
      - targets: ['localhost:8080']
```

Key metrics:
- `civicjournal_pages_total{level="0"}` - Total pages per level
- `civicjournal_leaves_total` - Total number of leaves
- `civicjournal_rollups_total{level="0"}` - Number of rollups per level
- `civicjournal_storage_bytes` - Storage usage in bytes

### Health Checks

```bash
# Liveness probe
GET /healthz

# Readiness probe
GET /readyz

# Response format
{
  "status": "ok",
  "version": "0.4.0",
  "uptime_seconds": 12345.67,
  "storage_usage_bytes": 1073741824,
  "pages_by_level": {"0": 42, "1": 7, "2": 1}
}
```

## Performance Tuning

### Storage Optimization

1. **Compression**: Enable and tune compression in `config.toml`
   ```toml
   [compression]
   enabled = true
   algorithm = "zstd"  # Best compression
   level = 3          # Balance between speed and ratio
   ```

2. **File System**: Use XFS or ext4 with appropriate mount options:
   ```
   /dev/sdb1 /data xfs defaults,noatime,nodiratime 0 2
   ```

3. **I/O Scheduler**: Use deadline or noop for SSDs:
   ```bash
   echo deadline > /sys/block/sdX/queue/scheduler
   ```

### Memory Management

1. **Page Cache**: The system benefits from available memory for page caching
2. **Jemalloc**: For better memory allocation performance, build with Jemalloc:
   ```bash
   cargo build --release --features jemalloc
   ```

### Network

1. **Keep-Alive**: Configure keep-alive for API clients
2. **gRPC**: For high-throughput deployments, enable gRPC in the configuration

## Troubleshooting

### Common Issues

1. **Permission Denied**
   - Ensure the service user has write access to storage directories
   - Check AppArmor/SELinux policies if running in a container

2. **High CPU Usage**
   - Check rollup operations with `journalctl -u civicjournal`
   - Consider increasing `max_items_per_page` or `max_page_age_seconds`

3. **Disk Full**
   - Implement retention policies
   - Monitor disk usage with Prometheus alerts

4. **Startup Failures**
   - Check config file syntax with `civicjournal-time check-config /path/to/config.toml`
   - Verify storage permissions

### Log Analysis

```bash
# Follow logs
journalctl -u civicjournal -f

# Search for errors
journalctl -u civicjournal --since "1 hour ago" | grep -i error

# Check disk I/O
iotop -oP
```

## Upgrading

### Version 0.3.x to 0.4.0

1. Backup your data
2. Stop the service
3. Update the binary or container image
4. Review configuration changes in the new version
5. Start the new version
6. Verify data integrity

### Data Migration

For major version upgrades, a migration tool will be provided if needed. Check the release notes for specific instructions.

## Support

For support, please open an issue on the [GitHub repository](https://github.com/COGSLLC/civicjournal-time/issues).

## License

CivicJournal-Time is licensed under the [AGPL-3.0-only](LICENSE).
