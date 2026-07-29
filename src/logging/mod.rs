use std::{fs::OpenOptions, panic::AssertUnwindSafe};

use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::config::AppPaths;

#[derive(Debug)]
pub enum LoggingGuard {
    File(WorkerGuard),
    Stderr,
}

pub fn init(paths: &AppPaths, configured_level: &str, verbosity: u8) -> Result<LoggingGuard> {
    let level = match verbosity {
        0 => configured_level,
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .context("configure log filter")?;
    let log_path = paths.data_dir.join("wireseer.log");
    let file_is_writable = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .is_ok();
    if file_is_writable {
        let appender = std::panic::catch_unwind(AssertUnwindSafe(|| {
            tracing_appender::rolling::never(&paths.data_dir, "wireseer.log")
        }))
        .ok();
        if let Some(appender) = appender {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(writer)
                .try_init()
                .map_err(|error| anyhow::anyhow!("initialize structured logging: {error}"))?;
            return Ok(LoggingGuard::File(guard));
        }
    }

    // `doctor` and read-only inventory commands remain useful in restricted
    // environments. Do not turn a logging-path problem into a panic.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|error| anyhow::anyhow!("initialize stderr logging: {error}"))?;
    Ok(LoggingGuard::Stderr)
}
