//! Journal minimal sur stderr (stdout est réservé au protocole MCP). Niveau réglé par LOG_LEVEL.

use std::sync::atomic::{AtomicU8, Ordering};

static LEVEL: AtomicU8 = AtomicU8::new(2);

fn rank(name: &str) -> u8 {
    match name.to_ascii_uppercase().as_str() {
        "DEBUG" => 1,
        "INFO" => 2,
        "WARNING" | "WARN" => 3,
        "ERROR" | "CRITICAL" => 4,
        _ => 2,
    }
}

pub fn set_level(name: &str) {
    LEVEL.store(rank(name), Ordering::Relaxed);
}

fn emit(level: u8, label: &str, msg: &str) {
    if level >= LEVEL.load(Ordering::Relaxed) {
        eprintln!("[arrmcp] {label}: {msg}");
    }
}

pub fn debug(msg: &str) {
    emit(1, "debug", msg)
}
pub fn info(msg: &str) {
    emit(2, "info", msg)
}
pub fn warn(msg: &str) {
    emit(3, "warning", msg)
}
pub fn error(msg: &str) {
    emit(4, "error", msg)
}
