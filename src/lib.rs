//! arrmcp : serveur MCP pour Sonarr, Radarr, Lidarr, Prowlarr et Tracearr (lecture, écriture gardée, mode test).
#![allow(clippy::result_large_err)] // ArrError porte un message, un conseil et des détails : il est rare et gros.

pub mod client;
pub mod config;
pub mod error;
pub mod http;
pub mod log;
pub mod policy;
pub mod redact;
pub mod runtime;
pub mod schema;
pub mod server;
pub mod services;
pub mod stats;
pub mod testmode;
pub mod tool;
pub mod tools_read;
pub mod tools_write;
