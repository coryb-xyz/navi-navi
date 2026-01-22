# navi-navi

A listen-listen TCP proxy for bridging two client applications. Unlike traditional proxies that connect to upstream servers, navi-navi listens on two ports and bridges the first connection from each port together.

## Use Cases

- Bridging two applications that can only act as clients
- NAT traversal rendezvous points
- Testing and development of client applications
- Protocol adapters where both endpoints initiate connections

## Installation

```bash
cargo build --release
```

The binary will be at `target/release/navi-navi` (or `navi-navi.exe` on Windows).

## Quick Start

```bash
# Start the proxy with default settings (ports 8080 and 8081)
navi-navi

# Specify custom ports
navi-navi --port-a 0.0.0.0:3000 --port-b 0.0.0.0:3001

# With health endpoint enabled
navi-navi --health-port 9090
```

## Configuration

navi-navi supports three configuration methods with the following priority:

**CLI arguments > Environment variables > TOML config file > Defaults**

### Configuration Options

| Option | CLI Flag | Environment Variable | TOML Key | Default | Description |
|--------|----------|---------------------|----------|---------|-------------|
| Port A | `--port-a` | `NAVI_PORT_A` | `port_a` | `0.0.0.0:8080` | Listen address for first endpoint |
| Port B | `--port-b` | `NAVI_PORT_B` | `port_b` | `0.0.0.0:8081` | Listen address for second endpoint |
| Buffer Size | `--buffer-size` | `NAVI_BUFFER_SIZE` | `buffer_size` | `65536` | Buffer size in bytes for data transfer |
| Connection Timeout | `--connection-timeout` | `NAVI_CONNECTION_TIMEOUT` | `connection_timeout` | `0` (none) | Seconds to wait for partner connection (0 = unlimited) |
| Max Connections | `--max-connections` | `NAVI_MAX_CONNECTIONS` | `max_connections` | `0` (unlimited) | Maximum concurrent bridged connections |
| Channel Capacity | `--channel-capacity` | `NAVI_CHANNEL_CAPACITY` | `channel_capacity` | `32` | Internal queue size for pending connections |
| Drain Timeout | `--drain-timeout` | `NAVI_DRAIN_TIMEOUT` | `drain_timeout` | `30` | Seconds to wait for connections to close on shutdown |
| Health Port | `--health-port` | `NAVI_HEALTH_PORT` | `health_port` | disabled | Port for health/metrics HTTP server |
| Log Format | `--log-format` | `NAVI_LOG_FORMAT` | `log_format` | `pretty` | Log output format: `pretty` or `json` |
| Log Level | `--log-level` | `NAVI_LOG_LEVEL` | `log_level` | `info` | Log level: `trace`, `debug`, `info`, `warn`, `error` |
| Config File | `-c, --config` | `NAVI_CONFIG` | - | - | Path to TOML configuration file |

### CLI Examples

```bash
# Basic usage with custom ports
navi-navi --port-a 127.0.0.1:5000 --port-b 127.0.0.1:5001

# Production settings with connection limits and health endpoint
navi-navi \
  --port-a 0.0.0.0:8080 \
  --port-b 0.0.0.0:8081 \
  --max-connections 1000 \
  --connection-timeout 60 \
  --health-port 9090 \
  --log-format json

# Use a specific config file
navi-navi --config /etc/navi-navi/config.toml

# Override config file settings with CLI flags
navi-navi --config /etc/navi-navi/config.toml --port-a 0.0.0.0:9000
```

### Environment Variables

```bash
# Set configuration via environment
export NAVI_PORT_A="0.0.0.0:8080"
export NAVI_PORT_B="0.0.0.0:8081"
export NAVI_MAX_CONNECTIONS="500"
export NAVI_LOG_FORMAT="json"
export NAVI_HEALTH_PORT="9090"

navi-navi
```

### TOML Configuration File

navi-navi searches for configuration files in this order:
1. Path specified by `--config` or `NAVI_CONFIG`
2. `./navi-navi.toml` (current directory)
3. `~/.config/navi-navi/config.toml` (Linux/macOS) or `%APPDATA%\navi-navi\config.toml` (Windows)

Example `navi-navi.toml`:

```toml
port_a = "0.0.0.0:8080"
port_b = "0.0.0.0:8081"
buffer_size = 65536
channel_capacity = 64
connection_timeout = 60
drain_timeout = 30
max_connections = 1000
log_format = "json"
log_level = "info"
health_port = 9090
```

## Health & Metrics

When `health_port` is configured, navi-navi exposes HTTP endpoints:

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Liveness probe - returns `200 OK` |
| `GET /ready` | Readiness probe - returns `200 READY` |
| `GET /metrics` | Prometheus-format metrics |

### Metrics

```
# HELP navi_active_connections Current number of active bridged connections
# TYPE navi_active_connections gauge
navi_active_connections 42
```

### Example Usage

```bash
# Liveness check
curl http://localhost:9090/health

# Prometheus scrape
curl http://localhost:9090/metrics
```

## Graceful Shutdown

navi-navi handles shutdown signals gracefully:

1. Stops accepting new connections
2. Waits up to `drain_timeout` seconds for existing connections to complete
3. Logs remaining connections if timeout is reached
4. Exits cleanly

Supported signals:
- `SIGINT` (Ctrl+C)
- `SIGTERM` (Unix)
- Service stop command (Windows)

## Running as a Windows Service

navi-navi can run as a native Windows service:

### Install the Service

```cmd
sc create navi-navi binPath= "C:\path\to\navi-navi.exe --service" start= auto
```

### Configure the Service

Since CLI arguments aren't available in service mode, configure via:

1. **TOML file** at `C:\ProgramData\navi-navi\config.toml` or next to the executable
2. **Environment variables** set in the service properties

To set environment variables for the service:

```cmd
reg add "HKLM\SYSTEM\CurrentControlSet\Services\navi-navi" /v Environment /t REG_MULTI_SZ /d "NAVI_PORT_A=0.0.0.0:8080\0NAVI_PORT_B=0.0.0.0:8081\0NAVI_HEALTH_PORT=9090"
```

### Manage the Service

```cmd
# Start
sc start navi-navi

# Stop (triggers graceful shutdown)
sc stop navi-navi

# Check status
sc query navi-navi

# Remove
sc delete navi-navi
```

## Running with Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /usr/src/navi-navi
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /usr/src/navi-navi/target/release/navi-navi /usr/local/bin/
EXPOSE 8080 8081 9090
CMD ["navi-navi"]
```

```bash
docker build -t navi-navi .
docker run -p 8080:8080 -p 8081:8081 -p 9090:9090 \
  -e NAVI_HEALTH_PORT=9090 \
  -e NAVI_LOG_FORMAT=json \
  navi-navi
```

## Running with systemd

Example `/etc/systemd/system/navi-navi.service`:

```ini
[Unit]
Description=navi-navi TCP proxy
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/navi-navi
Restart=on-failure
RestartSec=5

Environment=NAVI_PORT_A=0.0.0.0:8080
Environment=NAVI_PORT_B=0.0.0.0:8081
Environment=NAVI_HEALTH_PORT=9090
Environment=NAVI_LOG_FORMAT=json
Environment=NAVI_MAX_CONNECTIONS=1000

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable navi-navi
sudo systemctl start navi-navi
```

## Logging

navi-navi uses structured logging with trace IDs for request correlation.

### Pretty Format (default)

```
2024-01-15T10:30:00.000Z  INFO navi_navi::proxy: Bridging connections addr_a=192.168.1.10:54321 addr_b=192.168.1.20:54322 trace_id=678abc-0001
2024-01-15T10:30:05.000Z  INFO navi_navi::proxy: Bridge complete bytes_a_to_b=1024 bytes_b_to_a=2048 trace_id=678abc-0001
```

### JSON Format

```json
{"timestamp":"2024-01-15T10:30:00.000Z","level":"INFO","target":"navi_navi::proxy","message":"Bridging connections","addr_a":"192.168.1.10:54321","addr_b":"192.168.1.20:54322","trace_id":"678abc-0001"}
```

Control log level at runtime with `NAVI_LOG` environment variable:

```bash
NAVI_LOG=debug navi-navi
NAVI_LOG=navi_navi::proxy=trace navi-navi
```

## How It Works

```
┌──────────┐                                      ┌──────────┐
│ Client A │──connect──►┌────────────┐◄──connect──│ Client B │
└──────────┘            │            │            └──────────┘
                        │ navi-navi  │
                        │            │
                        │  Port A    │
                        │  Port B    │
                        │            │
                        │   Bridge   │
                        │    ◄─►     │
                        └────────────┘
```

1. Client A connects to Port A
2. Client B connects to Port B
3. navi-navi bridges the two connections
4. Data flows bidirectionally until either side disconnects

## License

MIT
