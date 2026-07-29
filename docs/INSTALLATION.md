# Installation

Wireseer is a terminal application. It runs on macOS, Linux, and Windows; Homebrew is the primary
packaging path for macOS and Linux. The minimum supported Rust version for source builds is 1.85.

## Homebrew

After the project tap is published, install the formula directly from the tap:

```bash
brew install mr-lexus/tap/wireseer
```

This form trusts only the requested formula and automatically adds the tap. If the tap has already
been added, `brew install wireseer` and subsequent `brew upgrade wireseer` work normally.

Verify the installation before scanning:

```bash
wireseer --version
wireseer doctor
```

The formula builds the tagged, SHA-256-verified source archive with Homebrew's Rust toolchain. It
does not download code during installation other than the locked Rust dependencies represented by
`Cargo.lock`. SQLite is bundled into the resulting binary; no separate SQLite or network scanner is
installed.

To remove the executable while keeping Wireseer's local data:

```bash
brew uninstall wireseer
```

Homebrew intentionally does not delete application data. Review the paths printed by
`wireseer config path` before removing local configuration, inventory, or logs manually.

## Build from source

With Rust 1.85 or newer installed:

```bash
git clone https://github.com/mr-lexus/wireseer.git wireseer
cd wireseer
cargo install --locked --path .
wireseer doctor
```

`--locked` makes Cargo use the dependency versions in `Cargo.lock`, matching the Homebrew build.

You can also install a published crate when one is available:

```bash
cargo install --locked wireseer-tui
```

The package is named `wireseer-tui`; the installed executable is `wireseer`.

## Prebuilt release binaries

Each stable GitHub release contains the source archive and these native binary archives:

| Platform | Architecture | Rust target | Archive |
| --- | --- | --- | --- |
| macOS | Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS | Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Linux, glibc | x86-64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux, glibc | ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| Linux, static musl | x86-64 | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Linux, static musl | ARM64 | `aarch64-unknown-linux-musl` | `.tar.gz` |
| Windows | x86-64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | `.zip` |

The GNU Linux builds target Ubuntu 22.04-era glibc or newer. Prefer a musl archive when running a
minimal distribution or an older/incompatible glibc userspace.

On macOS or Linux, select the archive matching `uname -m`, download it with `SHA256SUMS`, verify it,
and place the binary on `PATH`. For example, for Apple Silicon and version 0.1.0:

```bash
curl -LO https://github.com/mr-lexus/wireseer/releases/download/v0.1.0/wireseer-0.1.0-aarch64-apple-darwin.tar.gz
curl -LO https://github.com/mr-lexus/wireseer/releases/download/v0.1.0/SHA256SUMS
grep 'wireseer-0.1.0-aarch64-apple-darwin.tar.gz' SHA256SUMS | shasum -a 256 -c -
tar -xzf wireseer-0.1.0-aarch64-apple-darwin.tar.gz
install -m 0755 wireseer-0.1.0-aarch64-apple-darwin/wireseer "$HOME/.local/bin/wireseer"
wireseer --version
```

On Linux, `sha256sum -c` can be used instead of `shasum -a 256 -c`.

For Windows ARM64 in PowerShell, for example:

```powershell
$asset = "wireseer-0.1.0-aarch64-pc-windows-msvc.zip"
Invoke-WebRequest "https://github.com/mr-lexus/wireseer/releases/download/v0.1.0/$asset" -OutFile $asset
Invoke-WebRequest "https://github.com/mr-lexus/wireseer/releases/download/v0.1.0/SHA256SUMS" -OutFile SHA256SUMS
Get-FileHash $asset -Algorithm SHA256
Select-String $asset SHA256SUMS
Expand-Archive $asset -DestinationPath .
```

The displayed hash must exactly match the corresponding `SHA256SUMS` entry before the executable is
run. Move `wireseer.exe` into a directory on the user `PATH` after verification.

Public GitHub releases also carry a signed build-provenance attestation. With GitHub CLI installed:

```bash
gh attestation verify wireseer-0.1.0-aarch64-apple-darwin.tar.gz -R mr-lexus/wireseer
```

Release archives are not Apple Developer ID- or Authenticode-signed in version 0.1.0. Homebrew is
the preferred macOS installation path; for direct downloads, verify both the checksum and GitHub
provenance before bypassing any operating-system warning.

## First run

Opening the TUI starts a normal scan on the most likely active IPv4 interface:

```bash
wireseer
```

Only scan a network you own or are authorized to inspect. To inspect capabilities without sending
discovery traffic, use `wireseer doctor`. To choose an interface or subnet explicitly, see the
[user guide](USER_GUIDE.md) and [configuration reference](CONFIGURATION.md).

## Platform notes

### macOS

No administrator privileges are expected for the default providers. macOS firewall prompts or
local policy can limit multicast discovery; `doctor` reports provider availability. Neighbor-cache
reading degrades gracefully because the Linux `/proc/net/arp` adapter is not available.

### Linux

The default build uses ordinary TCP/UDP sockets and reads `/proc/net/arp`; it does not need raw
socket capabilities or root. Homebrew on Linux is supported by the same formula.

### Windows

Use the matching x86-64 or ARM64 release ZIP, or install with Cargo from PowerShell. The TUI supports
Windows Terminal. Neighbor-cache reading degrades gracefully; the remaining providers continue.

## Upgrade and migration

Before upgrading, an optional structured export provides a portable backup:

```bash
wireseer export --kind devices --format json --output devices-backup.json
wireseer export --kind history --format json --output history-backup.json
```

Database migrations run automatically on startup and retain existing inventory. Downgrades are not
guaranteed to understand a database written by a newer release, so back up before moving backwards.

## Troubleshooting an installation

```bash
wireseer --version
wireseer doctor
brew info wireseer
brew reinstall --build-from-source mr-lexus/tap/wireseer
```

For formula failures, include the Wireseer version, macOS/Linux version, CPU architecture, and the
relevant `brew` error in a report. For runtime issues, include `wireseer doctor`; review it first for
network identifiers you do not want to share.
