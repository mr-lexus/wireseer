use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use futures::StreamExt;

const IEEE_REGISTRIES: &[(&str, &str)] = &[
    ("MA-L", "https://standards-oui.ieee.org/oui/oui.csv"),
    ("MA-M", "https://standards-oui.ieee.org/oui28/mam.csv"),
    ("MA-S", "https://standards-oui.ieee.org/oui36/oui36.csv"),
];
const MAX_REGISTRY_BYTES: usize = 16 * 1024 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_ATTEMPTS: usize = 3;

/// An in-memory, offline OUI index. It accepts IEEE-style CSV files and simple
/// `AA:BB:CC Vendor name` lists. Lookups never contact a network service.
#[derive(Debug, Clone, Default)]
pub struct VendorDatabase {
    entries: Vec<VendorPrefix>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VendorPrefix {
    value: u64,
    bits: u8,
    vendor: String,
}

impl VendorDatabase {
    pub fn load(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("read vendor database at {}", path.display()))?;
        Self::parse(&source)
    }

    pub async fn load_non_blocking(path: PathBuf) -> Result<Self> {
        tokio::task::spawn_blocking(move || Self::load(&path))
            .await
            .context("vendor database worker stopped")?
    }

    pub fn parse(source: &str) -> Result<Self> {
        let mut entries = Vec::new();
        for raw in source.lines() {
            let line = raw.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.to_ascii_lowercase().starts_with("registry,")
            {
                continue;
            }
            let Some((prefix, vendor)) = split_entry(line) else {
                continue;
            };
            let Some((value, bits)) = parse_prefix(&prefix) else {
                continue;
            };
            if vendor.is_empty() {
                continue;
            }
            entries.push(VendorPrefix {
                value,
                bits,
                vendor,
            });
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.bits));
        anyhow::ensure!(
            !entries.is_empty(),
            "vendor database contains no recognized prefixes"
        );
        Ok(Self { entries })
    }

    #[must_use]
    pub fn lookup(&self, mac: &str) -> Option<&str> {
        let mac = parse_mac(mac)?;
        let first_octet = u8::try_from(mac >> 40).ok()?;
        if first_octet & 0x03 != 0 {
            return None;
        }
        self.entries
            .iter()
            .find(|entry| {
                let shift = 48_u8.saturating_sub(entry.bits);
                mac >> shift == entry.value
            })
            .map(|entry| entry.vendor.as_str())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn split_entry(line: &str) -> Option<(String, String)> {
    if line.contains(',') {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(line.as_bytes());
        let record = reader.records().next()?.ok()?;
        let first = record.get(0)?.trim();
        let (prefix, vendor) = if matches!(first, "MA-L" | "MA-M" | "MA-S" | "IAB") {
            (record.get(1)?, record.get(2)?)
        } else {
            (first, record.get(1)?)
        };
        return Some((prefix.trim().to_string(), vendor.trim().to_string()));
    }
    let (prefix, vendor) = line.split_once(char::is_whitespace)?;
    Some((prefix.trim().to_string(), vendor.trim().to_string()))
}

fn parse_prefix(value: &str) -> Option<(u64, u8)> {
    let (raw, explicit_bits) = value
        .trim()
        .split_once('/')
        .map_or((value.trim(), None), |(raw, bits)| {
            (raw, bits.parse::<u8>().ok())
        });
    let hexadecimal = raw
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<String>();
    if hexadecimal.len() < 6 || hexadecimal.len() > 12 {
        return None;
    }
    let bits = explicit_bits.unwrap_or(u8::try_from(hexadecimal.len() * 4).ok()?);
    if bits == 0 || bits > 48 {
        return None;
    }
    let parsed = u64::from_str_radix(&hexadecimal, 16).ok()?;
    let source_bits = u8::try_from(hexadecimal.len() * 4).ok()?;
    Some((parsed >> source_bits.saturating_sub(bits), bits))
}

fn parse_mac(value: &str) -> Option<u64> {
    let hexadecimal = value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<String>();
    (hexadecimal.len() == 12)
        .then(|| u64::from_str_radix(&hexadecimal, 16).ok())
        .flatten()
}

pub async fn update_official(destination: PathBuf) -> Result<VendorDatabase> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("Lantern/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build IEEE registry HTTP client")?;
    let mut combined = String::new();
    for &(registry, url) in IEEE_REGISTRIES {
        let bytes = download_registry(&client, registry, url).await?;
        let text = std::str::from_utf8(&bytes)
            .with_context(|| format!("IEEE {registry} registry is not UTF-8"))?;
        combined.push_str(text);
        combined.push('\n');
    }

    let database = VendorDatabase::parse(&combined)?;
    let destination_copy = destination.clone();
    tokio::task::spawn_blocking(move || replace_database(&destination_copy, &combined))
        .await
        .context("vendor database writer stopped")??;
    Ok(database)
}

async fn download_registry(client: &reqwest::Client, registry: &str, url: &str) -> Result<Vec<u8>> {
    let mut last_error = None;
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        match download_registry_once(client, registry, url).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                tracing::warn!(
                    registry,
                    attempt,
                    attempts = DOWNLOAD_ATTEMPTS,
                    %error,
                    "IEEE registry download attempt failed"
                );
                last_error =
                    Some(error.context(format!("attempt {attempt} of {DOWNLOAD_ATTEMPTS}")));
            }
        }
        if attempt < DOWNLOAD_ATTEMPTS {
            tokio::time::sleep(Duration::from_secs(if attempt == 1 { 1 } else { 3 })).await;
        }
    }

    let error = last_error.unwrap_or_else(|| anyhow::anyhow!("download did not start"));
    Err(error.context(format!(
        "download IEEE {registry} registry after {DOWNLOAD_ATTEMPTS} attempts"
    )))
}

async fn download_registry_once(
    client: &reqwest::Client,
    registry: &str,
    url: &str,
) -> Result<Vec<u8>> {
    let response = tokio::time::timeout(RESPONSE_TIMEOUT, client.get(url).send())
        .await
        .with_context(|| {
            format!(
                "IEEE {registry} did not respond within {} seconds",
                RESPONSE_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| format!("request IEEE {registry} registry"))?
        .error_for_status()
        .with_context(|| format!("request IEEE {registry} registry"))?;
    if let Some(length) = response.content_length() {
        anyhow::ensure!(
            length <= MAX_REGISTRY_BYTES as u64,
            "IEEE {registry} registry exceeds the {MAX_REGISTRY_BYTES}-byte limit"
        );
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next())
            .await
            .with_context(|| {
                format!(
                    "IEEE {registry} sent no data for {} seconds",
                    STREAM_IDLE_TIMEOUT.as_secs()
                )
            })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.with_context(|| format!("read IEEE {registry} registry"))?;
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_REGISTRY_BYTES,
            "IEEE {registry} registry exceeds the {MAX_REGISTRY_BYTES}-byte limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn replace_database(destination: &Path, source: &str) -> Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).context("create vendor database directory")?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vendors.csv");
    let suffix = std::process::id();
    let temporary = parent.join(format!(".{file_name}.{suffix}.tmp"));
    let backup = parent.join(format!(".{file_name}.{suffix}.backup"));
    anyhow::ensure!(
        !temporary.exists() && !backup.exists(),
        "temporary vendor database files already exist"
    );
    fs::write(&temporary, source).with_context(|| {
        format!(
            "write downloaded vendor database at {}",
            temporary.display()
        )
    })?;

    let had_existing = destination.exists();
    if had_existing {
        fs::rename(destination, &backup)
            .with_context(|| format!("back up vendor database at {}", destination.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        if had_existing {
            fs::rename(&backup, destination).context("restore previous vendor database")?;
        }
        let _ = fs::remove_file(&temporary);
        return Err(error)
            .with_context(|| format!("publish vendor database at {}", destination.display()));
    }
    if had_existing {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

pub fn import(source: &Path, destination: &Path) -> Result<()> {
    // Validate before copying so a malformed import never replaces a working index.
    VendorDatabase::load(source)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).context("create vendor database directory")?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "copy vendor database from {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ieee_csv_and_plain_oui_lists() {
        let database = VendorDatabase::parse(
            r#"Registry,Assignment,Organization Name,Organization Address
MA-L,001122,"Example Networks, Inc.","1 Main St, Example City"
AC:BB:CC Acme Devices
"#,
        )
        .expect("database");
        assert_eq!(
            database.lookup("00:11:22:33:44:55"),
            Some("Example Networks, Inc.")
        );
        assert_eq!(database.lookup("ac-bb-cc-00-00-01"), Some("Acme Devices"));
        assert_eq!(database.lookup("02:11:22:33:44:55"), None);
        assert_eq!(database.lookup("de:ad:be:ef:00:00"), None);
    }

    #[test]
    fn longest_prefix_wins() {
        let database = VendorDatabase::parse("00:11:22 Parent\n0011223/28 Child\n").expect("db");
        assert_eq!(database.lookup("00:11:22:3a:00:00"), Some("Child"));
        assert_eq!(database.lookup("00:11:22:4a:00:00"), Some("Parent"));
    }

    #[test]
    fn replaces_existing_database_without_stale_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let destination = directory.path().join("vendors.csv");
        fs::write(&destination, "00:11:22 Old Vendor\n").expect("old database");
        replace_database(&destination, "AC:BB:CC New Vendor\n").expect("replace database");
        let database = VendorDatabase::load(&destination).expect("updated database");
        assert_eq!(database.lookup("ac:bb:cc:00:00:01"), Some("New Vendor"));
        assert_eq!(
            fs::read_dir(directory.path()).expect("directory").count(),
            1
        );
    }
}
