//! Windows Service integration
//!
//! This module is only compiled on Windows targets and provides
//! integration with the Windows Service Control Manager (SCM).

use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

use crate::cli::Cli;
use crate::config::Config;
use crate::SERVICE_NAME;

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

fn set_status(
    handle: &ServiceStatusHandle,
    state: ServiceState,
    controls: ServiceControlAccept,
    wait_hint: Duration,
) -> Result<(), windows_service::Error> {
    handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: state,
        controls_accepted: controls,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint,
        process_id: None,
    })
}

// Generate the Windows service boilerplate
define_windows_service!(ffi_service_main, service_main);

/// Entry point called by the Windows service dispatcher
pub fn run_service() -> Result<(), windows_service::Error> {
    // Register the service with the system and start the dispatcher
    // This call blocks until the service is stopped
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

/// Main service entry point called by the Windows service infrastructure
fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service_inner() {
        // Log error - but logging may not be initialized yet
        eprintln!("Service failed: {}", e);
    }
}

fn run_service_inner() -> Result<(), Box<dyn std::error::Error>> {
    // Channel for receiving stop signal from SCM
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    // Register the service control handler
    let status_handle = service_control_handler::register(
        SERVICE_NAME,
        move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    // Signal the service to stop
                    let _ = shutdown_tx.send(());
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        },
    )?;

    // Report that we're starting
    set_status(
        &status_handle,
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        Duration::from_secs(10),
    )?;

    // Build config from environment/TOML (CLI args not available in service mode)
    let cli = Cli {
        service: true,
        ..Default::default()
    };

    let config = Config::build(cli).map_err(|e| format!("Configuration error: {}", e))?;

    // Initialize logging
    super::init_logging(config.log_format, &config.log_level);

    // Report that we're running
    set_status(
        &status_handle,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        Duration::default(),
    )?;

    // Create tokio runtime and run the proxy
    let runtime = tokio::runtime::Runtime::new()?;

    // Create a shutdown signal that waits for SCM stop command
    let shutdown_signal = move || async move {
        // Convert sync channel to async wait
        tokio::task::spawn_blocking(move || {
            let _ = shutdown_rx.recv();
        })
        .await
        .ok();
    };

    runtime.block_on(super::run_proxy(config, shutdown_signal));

    // Report that we're stopping
    set_status(
        &status_handle,
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        Duration::from_secs(30),
    )?;

    // Report that we've stopped
    set_status(
        &status_handle,
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        Duration::default(),
    )?;

    Ok(())
}
