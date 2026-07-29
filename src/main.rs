use std::{
    fs::File,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::mpsc as std_mpsc,
    thread,
};

use anyhow::{Context, Result};
use clap::Parser;
use wireseer_tui::{
    APP_NAME, BRAND_TRACE_UNICODE, VERSION,
    app::{AppRuntime, AppState, PersistenceEvent, demo_state},
    cli::{
        AlertsArgs, BaselineCommand, Cli, Command, ConfigCommand, DevicesArgs, ExportFormat,
        ExportKind, HistoryArgs, NetworkArgs, OutputFormat, VendorCommand,
    },
    config::{AppPaths, Config, IconMode, ScanMode, ThemeName},
    devices::Device,
    export::{self, Format},
    history::{HistoryStore, compare_baseline},
    logging,
    network::{
        DiscoveryEvent, HealthStatus, default_route, interfaces, provider_capabilities,
        select_interface, vendor::VendorDatabase,
    },
    tui::{self, PersistenceSink},
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::discover();
    paths.ensure()?;
    let mut config = Config::load(&paths, cli.config.as_ref())?;
    let _log_guard = logging::init(&paths, &config.log_level, cli.verbose)?;
    tracing::info!(version = VERSION, "Wireseer starting");
    match cli.command {
        None => {
            let persisted_icons = apply_icon_override(&mut config, cli.icons);
            open_tui(config, &paths, persisted_icons).await
        }
        Some(Command::Watch(args)) => {
            apply_network_args(&mut config, &args);
            config.scan_mode = ScanMode::Watch;
            let persisted_icons = apply_icon_override(&mut config, cli.icons);
            open_tui(config, &paths, persisted_icons).await
        }
        Some(Command::Scan(args)) => {
            apply_network_args(&mut config, &args.network);
            scan_once(config, &paths, args.format, args.output).await
        }
        Some(Command::Devices(args)) => list_devices(&paths, args),
        Some(Command::History(args)) => list_history(&paths, args),
        Some(Command::Alerts(args)) => list_alerts(&paths, args),
        Some(Command::Export(args)) => run_export(&paths, args.kind, args.format, args.output),
        Some(Command::Baseline { command }) => run_baseline(&paths, command),
        Some(Command::Config { command }) => run_config(&paths, &config, command),
        Some(Command::Doctor) => {
            apply_icon_override(&mut config, cli.icons);
            doctor(&paths, &config)
        }
        Some(Command::Vendor { command }) => run_vendor(&paths, &mut config, command).await,
    }
}

async fn open_tui(
    config: Config,
    paths: &AppPaths,
    persisted_icon_mode: Option<IconMode>,
) -> Result<()> {
    anyhow::ensure!(
        io::stdout().is_terminal(),
        "Wireseer's interactive view needs a terminal. Use `wireseer scan` for non-interactive discovery."
    );
    let store = HistoryStore::open(&paths.database_file)?;
    let pruned = store.prune(config.retention_days)?;
    if pruned > 0 {
        tracing::info!(records = pruned, "expired history records pruned");
    }
    let stored_devices = store.load_devices()?;
    let vendor_database = load_configured_vendor_database(&config).await;
    let stored_events = store.load_events(500)?;
    let stored_alerts = store.load_alerts()?;
    let available = interfaces().unwrap_or_else(|error| {
        tracing::warn!(%error, "interface enumeration failed");
        Vec::new()
    });
    let demo = std::env::var_os("WIRESEER_DEMO").is_some();
    let mut state = if demo {
        demo_state(&config)
    } else {
        AppState::new(&config, available, stored_devices)
    };
    if !demo {
        state.events = stored_events.into();
        state.alerts = stored_alerts;
        if let Some(baseline) = store.active_baseline()? {
            state.baseline_active = true;
            state.baseline_diff = compare_baseline(&baseline, &state.devices);
            let unknown_keys = state
                .baseline_diff
                .unknown
                .iter()
                .map(|device| device.stable_key.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            for device in &mut state.devices {
                device.baseline_unknown = unknown_keys.contains(device.stable_key.as_str());
            }
        }
    }
    if let Some(database) = vendor_database {
        let enriched = state.set_vendor_database(database);
        tracing::info!(enriched, "vendor database loaded");
    }
    state.provider_health = provider_capabilities(&config);
    let mut runtime = AppRuntime::new(state, config);
    if demo {
        // Recorded/demo rendering must never probe the network represented by mock data.
        runtime.auto_scan = false;
    }
    let mut persistence = HistorySink::new(store);
    let result = tui::run(&mut runtime, &mut persistence).await;
    runtime.config.theme = runtime.state.theme;
    runtime.config.icons = persisted_icon_mode.unwrap_or(runtime.state.icons);
    runtime.config.animations = runtime.state.animations;
    runtime.config.compact_rows = runtime.state.compact_rows;
    runtime.config.scan_mode = runtime.state.scan_mode;
    if let Err(error) = runtime.config.save(&paths.config_file) {
        tracing::warn!(%error, "could not persist TUI settings");
    }
    result
}

async fn load_configured_vendor_database(config: &Config) -> Option<VendorDatabase> {
    let path = config.vendor_database.clone()?;
    match VendorDatabase::load_non_blocking(path).await {
        Ok(database) => Some(database),
        Err(error) => {
            tracing::warn!(%error, "vendor database unavailable");
            None
        }
    }
}

fn apply_icon_override(config: &mut Config, icon_override: Option<IconMode>) -> Option<IconMode> {
    icon_override.map(|mode| {
        let persisted = config.icons;
        config.icons = mode;
        persisted
    })
}

fn apply_network_args(config: &mut Config, args: &NetworkArgs) {
    if let Some(interface) = &args.interface {
        config.interface = Some(interface.clone());
    }
    if let Some(subnet) = &args.subnet {
        config.subnet = Some(subnet.clone());
    }
    if let Some(mode) = args.mode {
        config.scan_mode = mode;
    }
}

async fn scan_once(
    config: Config,
    paths: &AppPaths,
    format: OutputFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    let available = interfaces()?;
    let selected = select_interface(&available, config.interface.as_deref())?;
    let vendor_database = load_configured_vendor_database(&config).await;
    let mut store = HistoryStore::open(&paths.database_file)?;
    let mut state = AppState::new(&config, available, store.load_devices()?);
    if let Some(database) = vendor_database {
        state.set_vendor_database(database);
    }
    let mut runtime = AppRuntime::new(state, config);
    runtime.state.active_interface = Some(selected);
    runtime.start_scan()?;
    loop {
        let event = runtime
            .discovery_rx
            .recv()
            .await
            .context("discovery worker ended unexpectedly")?;
        let finished = matches!(
            event,
            DiscoveryEvent::Finished { .. } | DiscoveryEvent::Failed { .. }
        );
        if let Some(persistence) = runtime.state.apply_discovery_event(event) {
            persist_now(&mut store, persistence)?;
        }
        for persistence in runtime.state.pending_persistence.drain(..) {
            persist_now(&mut store, persistence)?;
        }
        if finished {
            break;
        }
    }
    if let Some(error) = &runtime.state.scan.last_error {
        anyhow::bail!("scan failed: {error}");
    }
    let mut writer = output_writer(output)?;
    write_devices(&runtime.state.devices, format, true, &mut writer)?;
    writer.flush().context("flush scan output")
}

fn load_devices_with_current_identity(store: &HistoryStore) -> Result<Vec<Device>> {
    let mut devices = store.load_devices()?;
    for device in &mut devices {
        device.recalculate_fingerprint();
    }
    Ok(devices)
}

fn list_devices(paths: &AppPaths, args: DevicesArgs) -> Result<()> {
    let store = HistoryStore::open(&paths.database_file)?;
    let devices = load_devices_with_current_identity(&store)?
        .into_iter()
        .filter(|device| {
            !args.online || device.status == wireseer_tui::devices::DeviceStatus::Online
        })
        .collect::<Vec<_>>();
    let mut writer = output_writer(args.output)?;
    write_devices(&devices, args.format, false, &mut writer)?;
    writer.flush().context("flush device output")
}

fn list_history(paths: &AppPaths, args: HistoryArgs) -> Result<()> {
    let store = HistoryStore::open(&paths.database_file)?;
    let events = store.load_events(args.limit)?;
    let mut writer = output_writer(args.output)?;
    match args.format {
        OutputFormat::Table => export::history_table(&events, &mut writer)?,
        format => export::history(&events, structured_format(format)?, &mut writer)?,
    }
    writer.flush().context("flush history output")
}

fn list_alerts(paths: &AppPaths, args: AlertsArgs) -> Result<()> {
    let store = HistoryStore::open(&paths.database_file)?;
    let alerts = store
        .load_alerts()?
        .into_iter()
        .filter(|alert| !args.open || alert.resolved_at.is_none())
        .collect::<Vec<_>>();
    let mut writer = output_writer(args.output)?;
    match args.format {
        OutputFormat::Table => export::alerts_table(&alerts, &mut writer)?,
        format => export::alerts(&alerts, structured_format(format)?, &mut writer)?,
    }
    writer.flush().context("flush alert output")
}

fn output_writer(output: Option<PathBuf>) -> Result<Box<dyn Write>> {
    match output {
        Some(path) if path.as_os_str() != "-" => {
            Ok(Box::new(File::create(&path).with_context(|| {
                format!("create output at {}", path.display())
            })?))
        }
        Some(_) | None => Ok(Box::new(io::stdout().lock())),
    }
}

fn structured_format(format: OutputFormat) -> Result<Format> {
    match format {
        OutputFormat::Json => Ok(Format::Json),
        OutputFormat::Xml => Ok(Format::Xml),
        OutputFormat::Csv => Ok(Format::Csv),
        OutputFormat::Table => anyhow::bail!("table is a human-readable format"),
    }
}

const fn export_format(format: ExportFormat) -> Format {
    match format {
        ExportFormat::Json => Format::Json,
        ExportFormat::Xml => Format::Xml,
        ExportFormat::Csv => Format::Csv,
    }
}

fn write_devices(
    devices: &[Device],
    format: OutputFormat,
    scan: bool,
    writer: &mut dyn Write,
) -> Result<()> {
    match format {
        OutputFormat::Table if scan => export::scan_table(devices, writer),
        OutputFormat::Table => export::device_table(devices, writer),
        format => export::devices(devices, structured_format(format)?, writer),
    }
}

fn run_export(
    paths: &AppPaths,
    kind: ExportKind,
    format: ExportFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    let store = HistoryStore::open(&paths.database_file)?;
    let mut writer = output_writer(output)?;
    let format = export_format(format);
    match kind {
        ExportKind::Devices => {
            export::devices(
                &load_devices_with_current_identity(&store)?,
                format,
                &mut writer,
            )?;
        }
        ExportKind::History => export::history(&store.load_events(100_000)?, format, &mut writer)?,
        ExportKind::Alerts => export::alerts(&store.load_alerts()?, format, &mut writer)?,
        ExportKind::Baseline => {
            let baseline = store
                .active_baseline()?
                .context("no active baseline; run `wireseer baseline create`")?;
            export::baseline(&baseline, format, &mut writer)?;
        }
        ExportKind::Comparison => {
            let baseline = store
                .active_baseline()?
                .context("no active baseline; run `wireseer baseline create`")?;
            let devices = load_devices_with_current_identity(&store)?;
            export::comparison(&compare_baseline(&baseline, &devices), format, &mut writer)?;
        }
    }
    writer.flush().context("flush export output")
}

fn run_baseline(paths: &AppPaths, command: BaselineCommand) -> Result<()> {
    let store = HistoryStore::open(&paths.database_file)?;
    let devices = load_devices_with_current_identity(&store)?;
    match command {
        BaselineCommand::Create { name } => {
            anyhow::ensure!(
                !devices.is_empty(),
                "no devices are stored; run a scan before creating a baseline"
            );
            let subnet = devices
                .first()
                .map_or("unknown", |device| device.subnet.as_str());
            let baseline = store.save_baseline(&name, subnet, &devices)?;
            println!(
                "Created baseline '{}' with {} devices for {}",
                baseline.name,
                baseline.devices.len(),
                baseline.subnet
            );
        }
        BaselineCommand::Compare => {
            let baseline = store
                .active_baseline()?
                .context("no active baseline; run `wireseer baseline create`")?;
            let diff = compare_baseline(&baseline, &devices);
            println!(
                "Baseline '{}' · {} unknown · {} missing · {} changed",
                baseline.name,
                diff.unknown.len(),
                diff.missing.len(),
                diff.changed.len()
            );
            for device in diff.unknown {
                println!("+ UNKNOWN  {}  {}", device.name, device.ip);
            }
            for device in diff.missing {
                println!("- MISSING  {}  {}", device.name, device.ip);
            }
            for device in diff.changed {
                println!(
                    "~ CHANGED  {}  {}",
                    device.after.name,
                    device.changes.join("; ")
                );
            }
        }
    }
    Ok(())
}

fn run_config(paths: &AppPaths, config: &Config, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Path => println!("{}", paths.config_file.display()),
        ConfigCommand::Show => print!("{}", toml::to_string_pretty(config)?),
        ConfigCommand::Init { force } => {
            anyhow::ensure!(
                force || !paths.config_file.exists(),
                "configuration already exists at {}; pass --force to replace it",
                paths.config_file.display()
            );
            config.save(&paths.config_file)?;
            println!("Wrote {}", paths.config_file.display());
        }
    }
    Ok(())
}

fn doctor(paths: &AppPaths, config: &Config) -> Result<()> {
    println!("{APP_NAME} doctor {VERSION}  {BRAND_TRACE_UNICODE}\n");
    config.validate()?;
    check(
        "Configuration",
        DoctorStatus::Pass,
        &format!("valid · {}", paths.config_file.display()),
    );
    let available = match interfaces() {
        Ok(values) => {
            check(
                "IPv4 interfaces",
                if values.is_empty() {
                    DoctorStatus::Warning
                } else {
                    DoctorStatus::Pass
                },
                &format!("{} detected", values.len()),
            );
            for interface in &values {
                println!(
                    "    {:<16} {:<15} {}{}",
                    interface.name,
                    interface.address,
                    interface.subnet,
                    if interface.likely_active {
                        " · likely active"
                    } else {
                        ""
                    }
                );
            }
            values
        }
        Err(error) => {
            check(
                "IPv4 interfaces",
                DoctorStatus::Warning,
                &format!("{error:#}"),
            );
            Vec::new()
        }
    };
    if let Ok(selected) = select_interface(&available, config.interface.as_deref()) {
        check(
            "Selected subnet",
            DoctorStatus::Pass,
            &format!(
                "{} via {}",
                config
                    .subnet
                    .as_deref()
                    .unwrap_or(&selected.subnet.to_string()),
                selected.name
            ),
        );
        match default_route() {
            Some(route) if route.interface == selected.name => check(
                "Default gateway",
                DoctorStatus::Pass,
                &format!("{} via {}", route.gateway, route.interface),
            ),
            Some(route) => check(
                "Default gateway",
                DoctorStatus::Info,
                &format!(
                    "not defined for {}; system default is {} via {}",
                    selected.name, route.gateway, route.interface
                ),
            ),
            None => check(
                "Default gateway",
                DoctorStatus::Info,
                "not exposed by this platform; .1 remains only a device-role inference",
            ),
        }
    } else {
        check(
            "Selected subnet",
            DoctorStatus::Warning,
            "no usable IPv4 interface",
        );
    }
    for health in provider_capabilities(config) {
        check(
            health.provider,
            match health.status {
                HealthStatus::Available => DoctorStatus::Pass,
                HealthStatus::Disabled => DoctorStatus::Info,
                HealthStatus::Degraded | HealthStatus::Unavailable => DoctorStatus::Warning,
            },
            &health.detail,
        );
    }
    match HistoryStore::open(&paths.database_file) {
        Ok(_) => check(
            "SQLite history",
            DoctorStatus::Pass,
            &format!("available · {}", paths.database_file.display()),
        ),
        Err(error) => check("SQLite history", DoctorStatus::Warning, &error.to_string()),
    }
    match config.vendor_database.as_ref() {
        Some(path) if path.is_file() => check(
            "Vendor database",
            DoctorStatus::Pass,
            &path.display().to_string(),
        ),
        Some(path) => check(
            "Vendor database",
            DoctorStatus::Warning,
            &format!("configured file is unavailable · {}", path.display()),
        ),
        None => check(
            "Vendor database",
            DoctorStatus::Info,
            "not configured; vendor names remain unknown",
        ),
    }
    let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".into());
    let no_color = std::env::var("NO_COLOR").is_ok_and(|value| !value.is_empty());
    let (color_status, color_detail) = if config.theme == ThemeName::Monochrome {
        (
            DoctorStatus::Info,
            format!("Monochrome theme selected · TERM={term}"),
        )
    } else if no_color {
        (
            DoctorStatus::Warning,
            format!(
                "NO_COLOR is set but the explicit {} theme overrides it inside the TUI · TERM={term}",
                config.theme
            ),
        )
    } else if term == "dumb" {
        (
            DoctorStatus::Warning,
            format!(
                "{} requests color but TERM=dumb may not display ANSI colors",
                config.theme
            ),
        )
    } else {
        (DoctorStatus::Pass, format!("TERM={term}"))
    };
    check("Color support", color_status, &color_detail);
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "unknown".into());
    let unicode_supported = locale.to_ascii_uppercase().contains("UTF-8")
        || locale.to_ascii_uppercase().contains("UTF8");
    let (unicode_status, unicode_detail) = if config.icons == IconMode::Ascii {
        (
            DoctorStatus::Info,
            format!("not required with icons = \"ascii\" · {locale}"),
        )
    } else if unicode_supported {
        (DoctorStatus::Pass, locale.clone())
    } else {
        (DoctorStatus::Warning, locale.clone())
    };
    check("Unicode", unicode_status, &unicode_detail);
    let configured_icons = match config.icons {
        IconMode::Nerd => "nerd",
        IconMode::Unicode => "unicode",
        IconMode::Ascii => "ascii",
    };
    let nerd_font_detail = if config.icons == IconMode::Nerd {
        "selected explicitly; terminal font cannot be detected reliably".to_string()
    } else {
        format!("safe fallback active; icons = \"{configured_icons}\"")
    };
    check("Nerd Font", DoctorStatus::Info, &nerd_font_detail);
    println!(
        "\n'-' means optional, disabled, or not applicable. Unavailable providers degrade independently; missing ICMP is never proof that a host is offline."
    );
    Ok(())
}

async fn run_vendor(paths: &AppPaths, config: &mut Config, command: VendorCommand) -> Result<()> {
    use wireseer_tui::network::vendor::{VendorDatabase, import, update_official};
    match command {
        VendorCommand::Import { source } => {
            let destination = paths.data_dir.join("vendors.csv");
            import(&source, &destination)?;
            let database = VendorDatabase::load(&destination)?;
            config.vendor_database = Some(destination.clone());
            config.save(&paths.config_file)?;
            println!(
                "Imported {} vendor prefixes to {}",
                database.len(),
                destination.display()
            );
        }
        VendorCommand::Update => {
            let destination = paths.data_dir.join("vendors.csv");
            println!("Downloading official IEEE MA-L, MA-M, and MA-S registries...");
            let database = update_official(destination.clone()).await?;
            config.vendor_database = Some(destination.clone());
            config.save(&paths.config_file)?;
            println!(
                "Updated {} IEEE vendor prefixes at {}",
                database.len(),
                destination.display()
            );
        }
        VendorCommand::Status => {
            let path = config
                .vendor_database
                .as_ref()
                .context("no vendor database configured; run `wireseer vendor update`")?;
            let database = VendorDatabase::load(path)?;
            println!("{} prefixes · {}", database.len(), path.display());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoctorStatus {
    Pass,
    Info,
    Warning,
}

fn check(label: &str, status: DoctorStatus, detail: &str) {
    let marker = match status {
        DoctorStatus::Pass => "+",
        DoctorStatus::Info => "-",
        DoctorStatus::Warning => "!",
    };
    println!("{marker} {label:<20} {detail}");
}

fn persist_now(store: &mut HistoryStore, event: PersistenceEvent) -> Result<()> {
    match event {
        PersistenceEvent::Device(device) => store.upsert_device(&device),
        PersistenceEvent::DeviceAndEvent { device, event } => {
            store.upsert_device(&device)?;
            store.record_event(&event)
        }
        PersistenceEvent::DevicesAndEvents(values) => {
            for (device, event) in values {
                store.upsert_device(&device)?;
                store.record_event(&event)?;
            }
            Ok(())
        }
        PersistenceEvent::DeviceEventAlert {
            device,
            event,
            alert,
        } => {
            store.upsert_device(&device)?;
            store.record_event(&event)?;
            store.record_alert(&alert)
        }
        PersistenceEvent::Alert(alert) => store.record_alert(&alert),
        PersistenceEvent::ScanFinished => Ok(()),
    }
}

enum HistoryCommand {
    Persist(Box<PersistenceEvent>),
    Flush(std_mpsc::Sender<Result<()>>),
    Stop,
}

struct HistorySink {
    sender: std_mpsc::Sender<HistoryCommand>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HistorySink {
    fn new(mut store: HistoryStore) -> Self {
        let (sender, receiver) = std_mpsc::channel();
        let thread = thread::spawn(move || {
            while let Ok(command) = receiver.recv() {
                match command {
                    HistoryCommand::Persist(event) => {
                        if let Err(error) = persist_now(&mut store, *event) {
                            tracing::error!(%error, "history write failed");
                        }
                    }
                    HistoryCommand::Flush(reply) => {
                        let _ = reply.send(Ok(()));
                    }
                    HistoryCommand::Stop => break,
                }
            }
        });
        Self {
            sender,
            thread: Some(thread),
        }
    }
}

impl PersistenceSink for HistorySink {
    fn submit(&mut self, event: PersistenceEvent) {
        if self
            .sender
            .send(HistoryCommand::Persist(Box::new(event)))
            .is_err()
        {
            tracing::error!("history writer stopped unexpectedly");
        }
    }

    fn flush(&mut self) -> Result<()> {
        let (sender, receiver) = std_mpsc::channel();
        self.sender
            .send(HistoryCommand::Flush(sender))
            .context("request history flush")?;
        receiver.recv().context("wait for history flush")??;
        Ok(())
    }
}

impl Drop for HistorySink {
    fn drop(&mut self) {
        let _ = self.sender.send(HistoryCommand::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_override_preserves_the_configured_value_for_restore() {
        let mut config = Config::default();
        let persisted = apply_icon_override(&mut config, Some(IconMode::Nerd));
        assert_eq!(config.icons, IconMode::Nerd);
        assert_eq!(persisted, Some(IconMode::Unicode));
    }
}
