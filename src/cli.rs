use clap::Parser;

/// A listen-listen TCP proxy for bridging two client applications
#[derive(Parser, Debug, Default)]
#[command(name = "navi-navi")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Address for port A (first listener)
    #[arg(long, env = "NAVI_PORT_A")]
    pub port_a: Option<String>,

    /// Address for port B (second listener)
    #[arg(long, env = "NAVI_PORT_B")]
    pub port_b: Option<String>,

    /// Buffer size for data transfer in bytes
    #[arg(long, env = "NAVI_BUFFER_SIZE")]
    pub buffer_size: Option<usize>,

    /// Connection timeout in seconds (0 = no timeout)
    #[arg(long, env = "NAVI_CONNECTION_TIMEOUT")]
    pub connection_timeout: Option<u64>,

    /// Log format: "json" or "pretty"
    #[arg(long, env = "NAVI_LOG_FORMAT")]
    pub log_format: Option<String>,

    /// Log level: "trace", "debug", "info", "warn", "error"
    #[arg(long, env = "NAVI_LOG_LEVEL")]
    pub log_level: Option<String>,

    /// Maximum concurrent connections (0 = unlimited)
    #[arg(long, env = "NAVI_MAX_CONNECTIONS")]
    pub max_connections: Option<usize>,

    /// Channel capacity for pending connections
    #[arg(long, env = "NAVI_CHANNEL_CAPACITY")]
    pub channel_capacity: Option<usize>,

    /// Drain timeout in seconds for graceful shutdown
    #[arg(long, env = "NAVI_DRAIN_TIMEOUT")]
    pub drain_timeout: Option<u64>,

    /// Health/metrics server port (disabled if not set)
    #[arg(long, env = "NAVI_HEALTH_PORT")]
    pub health_port: Option<u16>,

    /// Path to configuration file
    #[arg(short, long, env = "NAVI_CONFIG")]
    pub config: Option<String>,

    /// Run as Windows service (used by Service Control Manager)
    #[arg(long, hide = true)]
    pub service: bool,
}
