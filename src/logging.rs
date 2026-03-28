use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;

use color_eyre::eyre::Result;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initialize structured logging to a file.
///
/// Returns a `WorkerGuard` that must be held alive for the duration of the
/// program to ensure all log messages are flushed.
pub fn init_logging(debug: bool) -> Result<WorkerGuard> {
    let log_file = open_log_file()?;
    let (non_blocking, guard) = tracing_appender::non_blocking(log_file);

    let default_filter = if debug { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter))
        .unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true),
        )
        .init();

    Ok(guard)
}

fn open_log_file() -> io::Result<File> {
    let mut last_error = None;

    for log_dir in log_directories() {
        match open_log_file_in_directory(&log_dir) {
            Ok(file) => return Ok(file),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no writable log directory candidates were available",
        )
    }))
}

fn open_log_file_in_directory(log_dir: &PathBuf) -> io::Result<File> {
    fs::create_dir_all(log_dir)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("hurl.log"))
}

fn log_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    if let Some(data_local_dir) = dirs::data_local_dir() {
        directories.push(data_local_dir.join("hurl").join("logs"));
    }

    directories.push(std::env::temp_dir().join("hurl").join("logs"));

    if let Ok(current_dir) = std::env::current_dir() {
        directories.push(current_dir.join(".hurl").join("logs"));
    }

    directories
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hurl-logging-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn open_log_file_uses_next_candidate_when_first_is_a_file() {
        let blocked = unique_path("blocked");
        let fallback = unique_path("fallback");

        let _ = fs::remove_file(&blocked);
        let _ = fs::remove_dir_all(&fallback);

        fs::write(&blocked, b"not a directory").unwrap();

        let file = open_log_file_in_candidates([blocked.clone(), fallback.clone()]).unwrap();
        let expected_path = fallback.join("hurl.log");

        assert!(expected_path.exists());
        drop(file);

        let _ = fs::remove_file(blocked);
        let _ = fs::remove_dir_all(fallback);
    }

    #[test]
    fn open_log_file_returns_error_when_no_candidate_is_writable() {
        let blocked = unique_path("all-blocked");

        let _ = fs::remove_file(&blocked);
        fs::write(&blocked, b"not a directory").unwrap();

        let error = open_log_file_in_candidates([blocked.clone()]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

        let _ = fs::remove_file(blocked);
    }

    fn open_log_file_in_candidates<I>(candidates: I) -> io::Result<File>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let mut last_error = None;

        for log_dir in candidates {
            match open_log_file_in_directory(&log_dir) {
                Ok(file) => return Ok(file),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error
            .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no candidates provided")))
    }
}
