mod cli;
mod config;
mod health;
mod proxy;

use clap::Parser;
use cli::Cli;
use config::{Config, LogFormat};
use health::HealthServer;
use proxy::Proxy;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// Windows service support
#[cfg(windows)]
mod service;

const SERVICE_NAME: &str = "navi-navi";

fn init_logging(format: LogFormat, level: &str) {
    let filter = EnvFilter::try_from_env("NAVI_LOG").unwrap_or_else(|_| EnvFilter::new(level));
    let registry = tracing_subscriber::registry().with(filter);

    match format {
        LogFormat::Json => registry.with(fmt::layer().json()).init(),
        LogFormat::Pretty => registry.with(fmt::layer()).init(),
    }
}

fn main() {
    // Parse CLI arguments (which also reads env vars via clap's env feature)
    let cli = Cli::parse();

    // Check if running as Windows service
    #[cfg(windows)]
    if cli.service {
        if let Err(e) = service::run_service() {
            eprintln!("Service error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // Normal CLI mode
    run_cli(cli);
}

fn run_cli(cli: Cli) {
    // Build layered config: CLI > Env > TOML > Defaults
    let config = match Config::build(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize logging with configured format and level
    init_logging(config.log_format, &config.log_level);

    // Build and run tokio runtime
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    runtime.block_on(run_proxy(config, cli_shutdown_signal));
}

/// Core proxy runner - shared between CLI and service modes
async fn run_proxy<F, Fut>(config: Config, shutdown_signal: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    info!(
        port_a = %config.port_a,
        port_b = %config.port_b,
        buffer_size = config.buffer_size,
        channel_capacity = config.channel_capacity,
        max_connections = config.max_connections,
        drain_timeout_secs = config.drain_timeout.as_secs(),
        connection_timeout_secs = config.connection_timeout.map(|d| d.as_secs()),
        health_port = ?config.health_port,
        "Starting navi-navi proxy"
    );

    // Create shared cancellation token
    let cancel_token = CancellationToken::new();

    // Create proxy
    let proxy = Proxy::new(config.clone(), cancel_token.clone());
    let metrics = proxy.metrics();

    // Spawn health server if configured
    let health_handle = if let Some(health_port) = config.health_port {
        let health_server = HealthServer::new(health_port, metrics, cancel_token.clone());
        Some(tokio::spawn(async move {
            if let Err(e) = health_server.run().await {
                error!(error = %e, "Health server failed");
            }
        }))
    } else {
        None
    };

    // Run proxy with signal handling
    tokio::select! {
        result = proxy.run() => {
            if let Err(e) = result {
                error!(error = %e, "Proxy failed");
                std::process::exit(1);
            }
        }
        _ = shutdown_signal() => {
            info!("Received shutdown signal");
        }
    }

    // Graceful shutdown
    proxy.shutdown_gracefully().await;

    // Wait for health server to stop
    if let Some(handle) = health_handle {
        let _ = handle.await;
    }

    info!("Proxy shutdown complete");
}

/// CLI mode shutdown signal (SIGINT/SIGTERM)
async fn cli_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.expect("Failed to install Ctrl+C handler");
}
