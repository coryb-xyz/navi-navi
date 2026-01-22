use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, info_span, warn, Instrument};

use crate::config::Config;

/// Shared state for tracking active connections
#[derive(Clone)]
pub struct ProxyMetrics {
    active_connections: Arc<AtomicUsize>,
}

impl ProxyMetrics {
    pub fn new() -> Self {
        Self {
            active_connections: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }
}

/// RAII guard for connection counting - decrements on drop
struct ConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl ConnectionGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Exponential backoff for error recovery
struct Backoff {
    current: Duration,
    min: Duration,
    max: Duration,
}

impl Backoff {
    fn new() -> Self {
        Self {
            current: Duration::from_millis(10),
            min: Duration::from_millis(10),
            max: Duration::from_secs(5),
        }
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * 2).min(self.max);
        delay
    }

    fn reset(&mut self) {
        self.current = self.min;
    }
}

/// Trace ID generator using timestamp + counter
struct TraceIdGenerator {
    counter: AtomicU64,
    epoch: u64,
}

impl TraceIdGenerator {
    fn new() -> Self {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            counter: AtomicU64::new(0),
            epoch,
        }
    }

    fn next(&self) -> String {
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{:x}-{:04x}", self.epoch, count & 0xFFFF)
    }
}

pub struct Proxy {
    config: Arc<Config>,
    cancel_token: CancellationToken,
    metrics: ProxyMetrics,
    trace_gen: Arc<TraceIdGenerator>,
}

struct PendingConnection {
    stream: TcpStream,
    addr: SocketAddr,
    port_label: &'static str,
    arrived_at: Instant,
}

impl Proxy {
    pub fn new(config: Config, cancel_token: CancellationToken) -> Self {
        Self {
            config: Arc::new(config),
            cancel_token,
            metrics: ProxyMetrics::new(),
            trace_gen: Arc::new(TraceIdGenerator::new()),
        }
    }

    pub fn metrics(&self) -> ProxyMetrics {
        self.metrics.clone()
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener_a = TcpListener::bind(self.config.port_a).await?;
        let listener_b = TcpListener::bind(self.config.port_b).await?;

        info!(port_a = %self.config.port_a, port_b = %self.config.port_b, "Proxy listening");

        let (tx_a, mut rx_a) = mpsc::channel::<PendingConnection>(self.config.channel_capacity);
        let (tx_b, mut rx_b) = mpsc::channel::<PendingConnection>(self.config.channel_capacity);

        let config = self.config.clone();
        let metrics = self.metrics.clone();
        let trace_gen = self.trace_gen.clone();

        let cancel_matcher = self.cancel_token.clone();

        // Spawn listeners for both ports
        let accept_a = spawn_listener(listener_a, tx_a, "A", self.cancel_token.clone());
        let accept_b = spawn_listener(listener_b, tx_b, "B", self.cancel_token.clone());

        // Match connections and bridge them
        let connection_timeout = config.connection_timeout;
        let max_connections = config.max_connections;
        let matcher = tokio::spawn(async move {
            loop {
                // Wait for a connection on port A, respecting cancellation
                let conn_a = tokio::select! {
                    _ = cancel_matcher.cancelled() => {
                        info!("Matcher shutting down");
                        break;
                    }
                    result = rx_a.recv() => {
                        match result {
                            Some(c) => c,
                            None => break,
                        }
                    }
                };

                // Check max connections limit
                if max_connections > 0 {
                    let current = metrics.active_connections();
                    if current >= max_connections {
                        warn!(
                            current = current,
                            max = max_connections,
                            "Max connections reached, rejecting"
                        );
                        continue;
                    }
                }

                info!(
                    addr_a = %conn_a.addr,
                    "Connection from A waiting for B"
                );

                // Wait for a connection on port B, with optional timeout
                let conn_b = if let Some(timeout_duration) = connection_timeout {
                    let elapsed = conn_a.arrived_at.elapsed();
                    let remaining = timeout_duration.saturating_sub(elapsed);

                    if remaining.is_zero() {
                        warn!(
                            addr = %conn_a.addr,
                            port = conn_a.port_label,
                            "Connection timed out waiting for partner"
                        );
                        continue;
                    }

                    tokio::select! {
                        _ = cancel_matcher.cancelled() => {
                            info!("Matcher shutting down");
                            break;
                        }
                        result = timeout(remaining, rx_b.recv()) => {
                            match result {
                                Ok(Some(c)) => c,
                                Ok(None) => break,
                                Err(_) => {
                                    warn!(
                                        addr = %conn_a.addr,
                                        port = conn_a.port_label,
                                        timeout_secs = timeout_duration.as_secs(),
                                        "Connection timed out waiting for partner"
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                } else {
                    tokio::select! {
                        _ = cancel_matcher.cancelled() => {
                            info!("Matcher shutting down");
                            break;
                        }
                        result = rx_b.recv() => {
                            match result {
                                Some(c) => c,
                                None => break,
                            }
                        }
                    }
                };

                let trace_id = trace_gen.next();
                let span = info_span!("bridge", trace_id = %trace_id);

                info!(
                    parent: &span,
                    addr_a = %conn_a.addr,
                    addr_b = %conn_b.addr,
                    wait_ms = conn_a.arrived_at.elapsed().as_millis() as u64,
                    "Bridging connections"
                );

                let buffer_size = config.buffer_size;
                let conn_counter = metrics.active_connections.clone();

                // Spawn the bridge task with trace ID span
                tokio::spawn(
                    async move {
                        let _guard = ConnectionGuard::new(conn_counter);

                        if let Err(e) =
                            bridge_streams(conn_a.stream, conn_b.stream, buffer_size).await
                        {
                            debug!(error = %e, "Bridge closed");
                        }
                        info!(
                            addr_a = %conn_a.addr,
                            addr_b = %conn_b.addr,
                            "Bridge terminated"
                        );
                    }
                    .instrument(span),
                );
            }
        });

        tokio::select! {
            _ = accept_a => warn!("Listener A task ended"),
            _ = accept_b => warn!("Listener B task ended"),
            _ = matcher => warn!("Matcher task ended"),
        }

        Ok(())
    }

    /// Gracefully shutdown the proxy
    pub async fn shutdown_gracefully(&self) {
        info!("Initiating graceful shutdown");

        // Signal all tasks to stop accepting new connections
        self.cancel_token.cancel();

        let drain_timeout = self.config.drain_timeout;
        let start = Instant::now();

        // Wait for active connections to drain
        loop {
            let active = self.metrics.active_connections();
            if active == 0 {
                info!("All connections drained");
                break;
            }

            if start.elapsed() >= drain_timeout {
                warn!(
                    remaining_connections = active,
                    "Drain timeout reached, forcing shutdown"
                );
                break;
            }

            info!(
                active_connections = active,
                elapsed_secs = start.elapsed().as_secs(),
                drain_timeout_secs = drain_timeout.as_secs(),
                "Waiting for connections to drain"
            );

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// Spawn a listener task that accepts connections and sends them to the channel
fn spawn_listener(
    listener: TcpListener,
    tx: mpsc::Sender<PendingConnection>,
    port_label: &'static str,
    cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Backoff::new();
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!(port = port_label, "Listener shutting down");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            backoff.reset();
                            info!(addr = %addr, port = port_label, "New connection");
                            let pending = PendingConnection {
                                stream,
                                addr,
                                port_label,
                                arrived_at: Instant::now(),
                            };
                            if tx.send(pending).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let delay = backoff.next_delay();
                            let retry_ms = delay.as_millis() as u64;
                            if is_transient_error(&e) {
                                warn!(error = %e, port = port_label, retry_ms, "Transient accept error, backing off");
                            } else {
                                error!(error = %e, port = port_label, retry_ms, "Accept failed, backing off");
                            }
                            tokio::time::sleep(delay).await;
                        }
                    }
                }
            }
        }
    })
}

/// Categorize errors as transient (recoverable) vs fatal
fn is_transient_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
    )
}

async fn bridge_streams(
    stream_a: TcpStream,
    stream_b: TcpStream,
    buffer_size: usize,
) -> Result<(), std::io::Error> {
    stream_a.set_nodelay(true)?;
    stream_b.set_nodelay(true)?;

    let (read_a, write_a) = stream_a.into_split();
    let (read_b, write_b) = stream_b.into_split();

    let (result_a, result_b) = tokio::join!(
        copy_stream(read_a, write_b, buffer_size, "A->B"),
        copy_stream(read_b, write_a, buffer_size, "B->A"),
    );

    match (result_a, result_b) {
        (Ok(a), Ok(b)) => info!(bytes_a_to_b = a, bytes_b_to_a = b, "Bridge complete"),
        (Err(e), _) | (_, Err(e)) => debug!(error = %e, "Bridge error"),
    }

    Ok(())
}

async fn copy_stream(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    buffer_size: usize,
    direction: &str,
) -> Result<u64, std::io::Error> {
    let mut buf = vec![0u8; buffer_size];
    let mut total_bytes: u64 = 0;

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            debug!(direction, bytes = total_bytes, "Stream closed");
            break;
        }
        writer.write_all(&buf[..n]).await?;
        total_bytes += n as u64;
    }

    writer.shutdown().await?;
    Ok(total_bytes)
}
