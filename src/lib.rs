pub mod alerts;
pub mod app;
pub mod cli;
pub mod config;
pub mod devices;
pub mod export;
pub mod history;
pub mod logging;
pub mod network;
pub mod tui;

pub const APP_NAME: &str = "Lantern";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
