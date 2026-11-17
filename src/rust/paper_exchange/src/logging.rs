use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_BYTES: u64 = 10 * 1024 * 1024;
const BACKUPS: usize = 5;
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
pub fn write(level: &str, message: String) {
    let _guard = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let line = format!(
        "{}.{:03} {level} {message}
",
        now.as_secs(),
        now.subsec_millis()
    );
    let dir = std::env::var_os("SERVICE_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("logs"));
    let name = env!("CARGO_PKG_NAME");
    let path = dir.join(format!("{name}.log"));
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(&dir)?;
        if fs::metadata(&path)
            .map(|m| m.len() + line.len() as u64 > MAX_BYTES)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(dir.join(format!("{name}.log.{BACKUPS}")));
            for i in (1..BACKUPS).rev() {
                let from = dir.join(format!("{name}.log.{i}"));
                if from.exists() {
                    fs::rename(from, dir.join(format!("{name}.log.{}", i + 1)))?;
                }
            }
            if path.exists() {
                fs::rename(&path, dir.join(format!("{name}.log.1")))?;
            }
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?
            .write_all(line.as_bytes())
    })();
    if let Err(error) = result {
        crate::service_log!("ERROR", "Could not write service log: {error}");
    }
    print!("{line}");
}
#[macro_export]
macro_rules! service_log { ($level:expr, $($arg:tt)*) => { $crate::logging::write($level, format!($($arg)*)) } }

#[cfg(test)]
mod tests {
    #[test]
    fn service_log_macro_formats_message_and_writes() {
        let path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("test-logs-{}", std::process::id()));
        unsafe {
            std::env::set_var("SERVICE_LOG_DIR", &path);
        }
        crate::service_log!("TEST", "coverage {}", 42);
        let log =
            std::fs::read_to_string(path.join(format!("{}.log", env!("CARGO_PKG_NAME")))).unwrap();
        assert!(log.contains("TEST coverage 42"));
        std::fs::remove_dir_all(&path).unwrap();
        unsafe {
            std::env::remove_var("SERVICE_LOG_DIR");
        }
    }
}
