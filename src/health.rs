use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::proxy::ProxyMetrics;

/// Health and metrics HTTP server
pub struct HealthServer {
    port: u16,
    metrics: ProxyMetrics,
    cancel_token: CancellationToken,
}

impl HealthServer {
    pub fn new(port: u16, metrics: ProxyMetrics, cancel_token: CancellationToken) -> Self {
        Self {
            port,
            metrics,
            cancel_token,
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        let listener = TcpListener::bind(addr).await?;

        info!(port = self.port, "Health server listening");

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!("Health server shutting down");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let io = TokioIo::new(stream);
                            let metrics = self.metrics.clone();

                            tokio::spawn(async move {
                                let service = service_fn(move |req| {
                                    let metrics = metrics.clone();
                                    async move { handle_request(req, metrics).await }
                                });

                                if let Err(e) = http1::Builder::new()
                                    .serve_connection(io, service)
                                    .await
                                {
                                    error!(error = %e, "Health server connection error");
                                }
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "Health server accept error");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    metrics: ProxyMetrics,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let response = match (req.method(), req.uri().path()) {
        (&Method::GET, "/health") => text_response(StatusCode::OK, "OK"),
        (&Method::GET, "/ready") => text_response(StatusCode::OK, "READY"),
        (&Method::GET, "/metrics") => {
            let body = format!(
                "# HELP navi_active_connections Current number of active bridged connections\n\
                 # TYPE navi_active_connections gauge\n\
                 navi_active_connections {}\n",
                metrics.active_connections()
            );
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; version=0.0.4")
                .body(Full::new(Bytes::from(body)))
                .unwrap()
        }
        _ => text_response(StatusCode::NOT_FOUND, "Not Found"),
    };

    Ok(response)
}

fn text_response(status: StatusCode, body: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}
