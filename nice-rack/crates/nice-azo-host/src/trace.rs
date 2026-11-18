//! The device's own voice while it lives inside somebody else's process.
//!
//! `niceroute.dll` runs hosted — REAPER, nice-rack, any ASIO client —
//! and a handshake that fails in a host is invisible from every probe.
//! Startup-path decisions (election, hardware hosting, refusals, panic
//! payloads) are appended to `%TEMP%\niceroute.log`, one timestamped
//! line each, so the answer to "REAPER says Error starting ASIO" is a
//! file read rather than a guess. Delete the file any time; the device
//! recreates it. Nothing on the audio path calls this — logging lives
//! one level up, in the calls hosts make between the callbacks.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Where `log` writes. Per process: a sandboxed or elevated run has its
/// own temporary directory, so a probe's lines and a host's lines can
/// live in two different files with the same name.
pub fn path() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("niceroute.log");
    path
}

/// One appended line, flushed. Best-effort forever: a device that
/// cannot write its diary must still make sound.
pub(crate) fn log(msg: &str) {
    static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    let Ok(mut slot) = FILE.get_or_init(|| Mutex::new(None)).lock() else {
        return;
    };
    if slot.is_none() {
        *slot = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path())
            .ok();
    }
    if let Some(file) = slot.as_mut() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        if writeln!(file, "{now:.3} pid {:>6} {msg}", std::process::id()).is_ok() {
            let _ = file.flush();
        }
    }
}
