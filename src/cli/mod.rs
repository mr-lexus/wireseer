use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{IconMode, ScanMode};

#[derive(Debug, Parser)]
#[command(
    name = "lantern",
    version,
    about = "Discover, inspect, and export devices on your local network",
    long_about = "Lantern is a local-first LAN inventory tool. Run it without a command for the interactive TUI, or use a subcommand for scripts, automation, and data export.\n\nHuman-readable output uses tables. JSON, XML, and CSV modes write only serialized data to the selected destination so other programs can consume it safely.",
    propagate_version = true
)]
#[command(
    after_help = "Examples:\n  lantern                         Open the interactive TUI\n  lantern devices --format json  Read inventory as JSON\n  lantern export --kind devices --format xml --output devices.xml\n  lantern scan --mode quick --format csv > scan.csv\n\nLantern performs conservative discovery only on networks you are authorized to inspect. CLI help is currently available in English."
)]
pub struct Cli {
    /// Use an alternate TOML configuration file.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// Increase diagnostic logging (`-v`, `-vv`).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Override icons for this run. Use `nerd` only with an active Nerd Font.
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "MODE",
        env = "LANTERN_ICONS"
    )]
    pub icons: Option<IconMode>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scan once and print or export discovered devices.
    Scan(ScanArgs),
    /// Open the TUI in watch mode.
    Watch(NetworkArgs),
    /// Export a complete stored dataset in a machine-readable format.
    Export(ExportArgs),
    /// Read devices from the local inventory.
    Devices(DevicesArgs),
    /// Read the stored network event timeline.
    History(HistoryArgs),
    /// Read alerts from the local database.
    Alerts(AlertsArgs),
    /// Create or compare a known-network baseline.
    Baseline {
        #[command(subcommand)]
        command: BaselineCommand,
    },
    /// Inspect configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Diagnose local capabilities without scanning.
    Doctor,
    /// Import or inspect the offline MAC vendor database.
    Vendor {
        #[command(subcommand)]
        command: VendorCommand,
    },
}

#[derive(Debug, Clone, clap::Args, Default)]
pub struct NetworkArgs {
    /// Select an interface by name instead of using automatic selection.
    #[arg(long, value_name = "NAME")]
    pub interface: Option<String>,
    /// Scan this authorized IPv4 CIDR instead of the selected interface subnet.
    #[arg(long, value_name = "CIDR")]
    pub subnet: Option<String>,
    /// Override the configured discovery mode.
    #[arg(long, value_enum, value_name = "MODE")]
    pub mode: Option<ScanMode>,
}

#[derive(Debug, Clone, clap::Args, Default)]
#[command(
    after_help = "Examples:\n  lantern scan\n  lantern scan --mode quick --format json\n  lantern scan --subnet 192.168.1.0/24 --format xml --output scan.xml"
)]
pub struct ScanArgs {
    #[command(flatten)]
    pub network: NetworkArgs,
    /// Output format. Defaults to table for terminals.
    #[arg(
        short,
        long,
        value_enum,
        default_value = "table",
        value_name = "FORMAT"
    )]
    pub format: OutputFormat,
    /// Write output to PATH. Use '-' or omit this option for stdout.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Xml,
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    Json,
    Xml,
    Csv,
}

#[derive(Debug, clap::Args)]
#[command(
    after_help = "Examples:\n  lantern export --kind devices --format json\n  lantern export --kind alerts --format xml --output alerts.xml\n  lantern export --kind comparison --format csv -o comparison.csv"
)]
pub struct ExportArgs {
    /// Serialization format.
    #[arg(short, long, value_enum, default_value = "json", value_name = "FORMAT")]
    pub format: ExportFormat,
    /// Write output to PATH. Use '-' or omit this option for stdout.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// Stored dataset to export.
    #[arg(
        short,
        long,
        value_enum,
        default_value = "devices",
        value_name = "KIND"
    )]
    pub kind: ExportKind,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExportKind {
    Devices,
    History,
    Alerts,
    Comparison,
    Baseline,
}

#[derive(Debug, clap::Args)]
#[command(
    after_help = "Examples:\n  lantern devices\n  lantern devices --online --format json\n  lantern devices --format xml --output devices.xml"
)]
pub struct DevicesArgs {
    /// Include only devices currently marked online.
    #[arg(long)]
    pub online: bool,
    /// Output format.
    #[arg(
        short,
        long,
        value_enum,
        default_value = "table",
        value_name = "FORMAT"
    )]
    pub format: OutputFormat,
    /// Write output to PATH. Use '-' or omit this option for stdout.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
#[command(
    after_help = "Examples:\n  lantern history --limit 100\n  lantern history --format json\n  lantern history --format csv --output history.csv"
)]
pub struct HistoryArgs {
    /// Maximum number of newest events to return.
    #[arg(long, default_value_t = 50, value_name = "COUNT")]
    pub limit: usize,
    /// Output format.
    #[arg(
        short,
        long,
        value_enum,
        default_value = "table",
        value_name = "FORMAT"
    )]
    pub format: OutputFormat,
    /// Write output to PATH. Use '-' or omit this option for stdout.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
#[command(
    after_help = "Examples:\n  lantern alerts\n  lantern alerts --open --format json\n  lantern alerts --format xml --output alerts.xml"
)]
pub struct AlertsArgs {
    /// Include only unresolved alerts.
    #[arg(long)]
    pub open: bool,
    /// Output format.
    #[arg(
        short,
        long,
        value_enum,
        default_value = "table",
        value_name = "FORMAT"
    )]
    pub format: OutputFormat,
    /// Write output to PATH. Use '-' or omit this option for stdout.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum BaselineCommand {
    Create {
        #[arg(long, default_value = "Known network")]
        name: String,
    },
    Compare,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Path,
    Show,
    Init {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum VendorCommand {
    /// Validate and copy an IEEE CSV or OUI text file into Lantern's data directory.
    Import { source: PathBuf },
    /// Download the current public IEEE MA-L, MA-M, and MA-S registries.
    Update,
    /// Show the configured database and number of loaded prefixes.
    Status,
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    #[test]
    fn icon_override_is_global_and_explicit() {
        let before = Cli::try_parse_from(["lantern", "--icons", "nerd"]).expect("before command");
        assert_eq!(before.icons, Some(IconMode::Nerd));

        let after =
            Cli::try_parse_from(["lantern", "doctor", "--icons", "ascii"]).expect("after command");
        assert_eq!(after.icons, Some(IconMode::Ascii));
        assert!(matches!(after.command, Some(Command::Doctor)));
    }

    #[test]
    fn structured_formats_and_output_paths_parse_consistently() {
        let scan =
            Cli::try_parse_from(["lantern", "scan", "--format", "xml", "--output", "scan.xml"])
                .expect("scan arguments");
        let Some(Command::Scan(scan)) = scan.command else {
            panic!("expected scan command");
        };
        assert_eq!(scan.format, OutputFormat::Xml);
        assert_eq!(scan.output, Some(PathBuf::from("scan.xml")));

        let devices = Cli::try_parse_from([
            "lantern", "devices", "--online", "--format", "json", "-o", "-",
        ])
        .expect("device arguments");
        let Some(Command::Devices(devices)) = devices.command else {
            panic!("expected devices command");
        };
        assert!(devices.online);
        assert_eq!(devices.format, OutputFormat::Json);
        assert_eq!(devices.output, Some(PathBuf::from("-")));
    }

    #[test]
    fn long_help_documents_human_and_machine_workflows() {
        let mut command = Cli::command();
        let mut output = Vec::new();
        command.write_long_help(&mut output).expect("render help");
        let help = String::from_utf8(output).expect("UTF-8 help");
        for expected in [
            "interactive TUI",
            "scripts, automation, and data export",
            "JSON, XML, and CSV",
            "lantern devices --format json",
            "alerts",
        ] {
            assert!(help.contains(expected), "missing help text: {expected}");
        }
    }
}
