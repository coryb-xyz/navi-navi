use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::cli::Cli;

/// Configuration loaded from TOML file
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FileConfig {
    pub port_a: Option<String>,
    pub port_b: Option<String>,
    pub buffer_size: Option<usize>,
    pub connection_timeout: Option<u64>,
    pub log_format: Option<String>,
    pub log_level: Option<String>,
    pub max_connections: Option<usize>,
    pub channel_capacity: Option<usize>,
    pub drain_timeout: Option<u64>,
    pub health_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port_a: SocketAddr,
    pub port_b: SocketAddr,
    pub buffer_size: usize,
    pub connection_timeout: Option<Duration>,
    pub log_format: LogFormat,
    pub log_level: String,
    pub max_connections: usize,
    pub channel_capacity: usize,
    pub drain_timeout: Duration,
    pub health_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogFormat {
    Json,
    Pretty,
}

impl Config {
    /// Build config with priority: CLI > Environment > TOML file > Defaults
    /// Note: CLI args with `#[arg(env = "...")]` already handle CLI > Env priority via clap
    pub fn build(cli: Cli) -> Result<Self, ConfigError> {
        // Load TOML config if available
        let file_config = Self::load_file_config(cli.config.as_deref());

        // Helper macro to resolve value with priority: CLI/Env > TOML > Default
        macro_rules! resolve {
            ($cli_val:expr, $file_val:expr, $default:expr) => {
                $cli_val.or($file_val).unwrap_or($default)
            };
        }

        // Resolve addresses
        let port_a_str = resolve!(cli.port_a, file_config.port_a, "0.0.0.0:8080".to_string());
        let port_b_str = resolve!(cli.port_b, file_config.port_b, "0.0.0.0:8081".to_string());

        let port_a = port_a_str
            .parse()
            .map_err(|_| ConfigError::InvalidAddress("port_a".to_string(), port_a_str))?;
        let port_b = port_b_str
            .parse()
            .map_err(|_| ConfigError::InvalidAddress("port_b".to_string(), port_b_str))?;

        // Resolve other fields
        let buffer_size = resolve!(cli.buffer_size, file_config.buffer_size, 64 * 1024);
        let connection_timeout_secs =
            resolve!(cli.connection_timeout, file_config.connection_timeout, 0);
        let connection_timeout = if connection_timeout_secs > 0 {
            Some(Duration::from_secs(connection_timeout_secs))
        } else {
            None
        };

        let log_format_str = resolve!(
            cli.log_format,
            file_config.log_format,
            "pretty".to_string()
        );
        let log_format = match log_format_str.to_lowercase().as_str() {
            "json" => LogFormat::Json,
            _ => LogFormat::Pretty,
        };

        let log_level = resolve!(cli.log_level, file_config.log_level, "info".to_string());
        let max_connections = resolve!(cli.max_connections, file_config.max_connections, 0);
        let channel_capacity = resolve!(cli.channel_capacity, file_config.channel_capacity, 32);
        let drain_timeout_secs = resolve!(cli.drain_timeout, file_config.drain_timeout, 30);
        let drain_timeout = Duration::from_secs(drain_timeout_secs);

        // Health port: CLI/Env > TOML > None (disabled by default)
        let health_port = cli.health_port.or(file_config.health_port);

        Ok(Config {
            port_a,
            port_b,
            buffer_size,
            connection_timeout,
            log_format,
            log_level,
            max_connections,
            channel_capacity,
            drain_timeout,
            health_port,
        })
    }

    /// Load configuration from TOML file
    /// Search order: explicit path > ./navi-navi.toml > ~/.config/navi-navi/config.toml
    fn load_file_config(explicit_path: Option<&str>) -> FileConfig {
        let paths_to_try: Vec<PathBuf> = match explicit_path {
            Some(path) => vec![PathBuf::from(path)],
            None => {
                let mut paths = vec![PathBuf::from("navi-navi.toml")];
                if let Some(config_dir) = directories::ProjectDirs::from("", "", "navi-navi") {
                    paths.push(config_dir.config_dir().join("config.toml"));
                }
                paths
            }
        };

        for path in paths_to_try.iter().filter(|p| p.exists()) {
            let contents = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to read config file");
                    continue;
                }
            };

            match toml::from_str(&contents) {
                Ok(config) => {
                    tracing::debug!(path = %path.display(), "Loaded config file");
                    return config;
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to parse config file");
                }
            }
        }

        FileConfig::default()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidAddress(String, String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::InvalidAddress(field, val) => {
                write!(f, "Invalid address for {}: '{}'", field, val)
            }
        }
    }
}

impl std::error::Error for ConfigError {}
