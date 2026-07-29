use std::{fmt, fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::ValueEnum;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub config_file: PathBuf,
    pub database_file: PathBuf,
    pub log_file: PathBuf,
}

impl AppPaths {
    #[must_use]
    pub fn discover() -> Self {
        let (config_dir, data_dir, cache_dir) = ProjectDirs::from("dev", "Wireseer", "wireseer")
            .map_or_else(
                || {
                    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                    (base.join("config"), base.join("data"), base.join("cache"))
                },
                |dirs| {
                    (
                        dirs.config_dir().to_path_buf(),
                        dirs.data_dir().to_path_buf(),
                        dirs.cache_dir().to_path_buf(),
                    )
                },
            );
        Self {
            config_file: config_dir.join("config.toml"),
            database_file: data_dir.join("wireseer.sqlite3"),
            log_file: data_dir.join("wireseer.log"),
            config_dir,
            data_dir,
            cache_dir,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir).context("create configuration directory")?;
        fs::create_dir_all(&self.data_dir).context("create data directory")?;
        fs::create_dir_all(&self.cache_dir).context("create cache directory")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    Quick,
    #[default]
    Normal,
    Deep,
    Watch,
    Passive,
}

impl ScanMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quick => "QUICK",
            Self::Normal => "NORMAL",
            Self::Deep => "DEEP",
            Self::Watch => "WATCH",
            Self::Passive => "PASSIVE",
        }
    }

    #[must_use]
    pub const fn ports(self) -> &'static [u16] {
        const QUICK: &[u16] = &[22, 80, 443, 445, 631, 3389, 5000, 5001, 8080, 9100];
        const NORMAL: &[u16] = &[
            22, 23, 53, 80, 139, 443, 445, 548, 631, 1883, 2049, 3306, 3389, 5000, 5001, 5432,
            5900, 6379, 8008, 8009, 8080, 8443, 8883, 9100, 32400,
        ];
        const DEEP: &[u16] = &[
            21, 22, 23, 25, 53, 80, 110, 139, 143, 443, 445, 548, 554, 631, 993, 995, 1883, 2049,
            3306, 3389, 5000, 5001, 5432, 5900, 6379, 8000, 8008, 8009, 8080, 8081, 8443, 8883,
            9000, 9001, 9100, 32400,
        ];
        match self {
            Self::Quick => QUICK,
            Self::Normal | Self::Watch => NORMAL,
            Self::Deep => DEEP,
            Self::Passive => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    #[default]
    WireseerDark,
    CatppuccinMocha,
    CatppuccinLatte,
    Dracula,
    Nord,
    MidnightBlue,
    Acid,
    PaperLight,
    HighContrast,
    Monochrome,
    ColorBlind,
}

impl ThemeName {
    pub const ALL: [Self; 11] = [
        Self::WireseerDark,
        Self::CatppuccinMocha,
        Self::CatppuccinLatte,
        Self::Dracula,
        Self::Nord,
        Self::MidnightBlue,
        Self::Acid,
        Self::PaperLight,
        Self::HighContrast,
        Self::Monochrome,
        Self::ColorBlind,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WireseerDark => "Wireseer Dark",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::MidnightBlue => "Midnight Blue",
            Self::Acid => "Acid",
            Self::PaperLight => "Paper Light",
            Self::HighContrast => "High Contrast",
            Self::Monochrome => "Monochrome",
            Self::ColorBlind => "Color-Blind Friendly",
        }
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::WireseerDark => "cold cyan signal on graphite",
            Self::CatppuccinMocha => "soft pastel dark",
            Self::CatppuccinLatte => "calm pastel light",
            Self::Dracula => "purple and cyan dark",
            Self::Nord => "cool arctic blue",
            Self::MidnightBlue => "electric blue on black",
            Self::Acid => "neon lime and magenta",
            Self::PaperLight => "warm low-glare light",
            Self::HighContrast => "maximum terminal contrast",
            Self::Monochrome => "no color dependency",
            Self::ColorBlind => "color-safe status accents",
        }
    }
}

impl fmt::Display for ThemeName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum IconMode {
    Nerd,
    #[default]
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub interface: Option<String>,
    pub subnet: Option<String>,
    pub scan_mode: ScanMode,
    pub refresh_interval_secs: u64,
    pub connect_timeout_ms: u64,
    pub dns_timeout_ms: u64,
    pub concurrency: usize,
    pub host_limit: usize,
    pub retention_days: u32,
    pub theme: ThemeName,
    pub icons: IconMode,
    pub animations: bool,
    pub compact_rows: bool,
    pub mouse: bool,
    pub http_metadata: bool,
    pub tls_metadata: bool,
    pub enabled_protocols: ProtocolConfig,
    pub visible_columns: Vec<String>,
    pub log_level: String,
    pub vendor_database: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolConfig {
    pub local: bool,
    pub tcp: bool,
    pub reverse_dns: bool,
    pub arp: bool,
    pub icmp: bool,
    pub mdns: bool,
    pub ssdp: bool,
    pub netbios: bool,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            local: true,
            tcp: true,
            reverse_dns: true,
            arp: true,
            icmp: false,
            mdns: true,
            ssdp: true,
            netbios: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interface: None,
            subnet: None,
            scan_mode: ScanMode::Normal,
            refresh_interval_secs: 30,
            connect_timeout_ms: 220,
            dns_timeout_ms: 750,
            concurrency: 96,
            host_limit: 1_024,
            retention_days: 90,
            theme: ThemeName::WireseerDark,
            icons: IconMode::Unicode,
            animations: true,
            compact_rows: false,
            mouse: true,
            http_metadata: false,
            tls_metadata: false,
            enabled_protocols: ProtocolConfig::default(),
            visible_columns: vec![
                "status".into(),
                "name".into(),
                "ip".into(),
                "vendor".into(),
                "latency".into(),
                "services".into(),
                "last_seen".into(),
            ],
            log_level: "info".into(),
            vendor_database: None,
        }
    }
}

impl Config {
    pub fn load(paths: &AppPaths, override_path: Option<&PathBuf>) -> Result<Self> {
        let path = override_path.unwrap_or(&paths.config_file);
        if !path.exists() {
            return Ok(Self::default());
        }
        let source = fs::read_to_string(path)
            .with_context(|| format!("read configuration at {}", path.display()))?;
        let config: Self = toml::from_str(&source)
            .with_context(|| format!("parse configuration at {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create configuration directory")?;
        }
        let source = toml::to_string_pretty(self).context("serialize configuration")?;
        fs::write(path, source)
            .with_context(|| format!("write configuration to {}", path.display()))
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.concurrency > 0 && self.concurrency <= 1_024,
            "concurrency must be between 1 and 1024"
        );
        anyhow::ensure!(
            self.connect_timeout_ms >= 20,
            "connect timeout must be at least 20 ms"
        );
        anyhow::ensure!(self.host_limit > 0, "host limit must be greater than zero");
        anyhow::ensure!(
            self.retention_days > 0,
            "retention must be greater than zero"
        );
        if let Some(subnet) = &self.subnet {
            subnet
                .parse::<ipnet::Ipv4Net>()
                .context("configured subnet is not valid IPv4 CIDR")?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let config = Config {
            compact_rows: true,
            ..Config::default()
        };
        let source = toml::to_string(&config).expect("serialize");
        let decoded: Config = toml::from_str(&source).expect("parse");
        assert_eq!(decoded.scan_mode, ScanMode::Normal);
        assert_eq!(decoded.concurrency, 96);
        assert!(decoded.compact_rows);
    }

    #[test]
    fn every_builtin_theme_round_trips_through_toml() {
        for theme in ThemeName::ALL {
            let config = Config {
                theme,
                ..Config::default()
            };
            let source = toml::to_string(&config).expect("serialize theme");
            let decoded: Config = toml::from_str(&source).expect("parse theme");
            assert_eq!(decoded.theme, theme, "failed to round-trip {theme}");
        }
    }

    #[test]
    fn rejects_zero_concurrency() {
        let config = Config {
            concurrency: 0,
            ..Config::default()
        };
        assert!(config.validate().is_err());
    }
}
