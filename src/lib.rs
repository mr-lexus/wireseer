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

pub const APP_NAME: &str = "Wireseer";
pub const APP_NAME_UPPER: &str = "WIRESEER";
pub const TAGLINE: &str = "LOCAL SIGNAL INTELLIGENCE";
pub const BRAND_TRACE_UNICODE: &str = "●──┬────●──╼";
pub const BRAND_TRACE_ASCII: &str = "o--+----o-->";
pub const BRAND_PULSE_UNICODE: &str = "●─╼";
pub const BRAND_PULSE_ASCII: &str = "o->";
pub const CLI_BANNER: &str =
    "WIRESEER  ●──┬────●──╼\n          local signal intelligence for your LAN";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
