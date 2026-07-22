# Lantern

![Lantern dashboard](docs/screenshots/dashboard.svg)

Lantern is a calm, local-first terminal application for discovering and understanding devices
on a network you own or are authorized to inspect. It combines an asynchronous scanner,
explainable device fingerprints, durable SQLite history, baseline comparisons, alerts, and a
responsive keyboard-driven interface.

Lantern does not call `nmap`, `arp`, `ping`, `netstat`, `ip`, or `ifconfig`. Discovery data stays
on the machine where Lantern runs.

## Highlights

- Designed TUI with wide, standard, compact, and safe undersized-terminal layouts.
- Incremental discovery while the UI remains responsive.
- Conservative TCP connect profiles with bounded concurrency, per-attempt timeouts, and
  immediate cancellation.
- Local-interface, Linux neighbor-cache, mDNS DNS-SD, SSDP/UPnP, and reverse-DNS observations.
- Optional HTTP metadata from one bounded `GET /`; no login, path guessing, crawling, or
  redirect following.
- Facts retain their discovery source; inferred device types show weighted evidence and never
  masquerade as confirmed identity.
- SQLite inventory, address/service state, global timeline, scan diff, and named baseline.
- Fuzzy device search and command palette, coarse status filters, sorting, multi-selection,
  mouse wheel support, and contextual help.
- Eleven built-in themes: Lantern Dark, Catppuccin Mocha/Latte, Dracula, Nord, Midnight Blue,
  Acid, Paper Light, High Contrast, Monochrome, and Color-Blind Friendly.
- Nerd Font, Unicode, and strict ASCII icon modes. Meaning never depends on an icon or color.
- JSON/XML/CSV export for devices, history, alerts, baselines, and comparisons; structured file
  logging; and a detailed `doctor` command.

## Discovery support

| Provider | Linux | Windows | macOS | Notes |
| --- | --- | --- | --- | --- |
| Local interface | yes | yes | yes | Portable IPv4 enumeration |
| TCP connect | yes | yes | yes | Conservative fixed profiles; no SYN/raw scan |
| Reverse DNS | yes | yes | yes | Uses the configured system resolver |
| mDNS / DNS-SD | yes | yes | yes | One service-enumeration multicast query; may degrade if port 5353 cannot be shared |
| SSDP / UPnP | yes | yes | yes | One bounded M-SEARCH request |
| ARP neighbors | yes | graceful | graceful | Linux reads `/proc/net/arp`; other platforms continue without it |
| ICMP | graceful | graceful | graceful | Raw adapter is not enabled; ICMP silence is never treated as offline proof |
| NetBIOS | graceful | graceful | graceful | Reserved provider boundary, currently disabled |
| TLS metadata | graceful | graceful | graceful | Model and persistence are ready; certificate adapter is not enabled |

“Graceful” means the provider reports an actionable status while every other provider continues.
Run `lantern doctor` on the target machine for the authoritative capability report.

## Install

Lantern uses stable Rust (MSRV 1.85).

```bash
git clone <repository-url> lantern
cd lantern
cargo install --path .
lantern doctor
lantern
```

For development:

```bash
cargo build
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Linux, Windows, and macOS are first-class targets. SQLite is built into the binary through
`rusqlite`'s bundled SQLite feature. No system SQLite package or scanning executable is needed.

## Start here

Open the interactive application:

```bash
lantern
```

Lantern selects the most likely active IPv4 interface and begins a normal scan. Devices appear
as soon as a provider returns positive evidence. Press `r` to rescan, `x` to cancel, `2` for the
device workspace, or `Ctrl+P` to discover commands.

For a non-interactive scan:

```bash
lantern scan
lantern scan --interface eth0
lantern scan --subnet 192.168.1.0/24 --mode quick
lantern scan --mode deep --format json
```

The subnet safety limit defaults to 1,024 hosts. A larger configured subnet is rejected before
any probe begins. Increase `host_limit` deliberately if your authorized network requires it.

Watch mode repeats only after the configured interval and after the previous scan has finished:

```bash
lantern watch
```

## Keyboard reference

Global navigation is consistent across screens.

The first footer row always shows all seven numbered workspaces and highlights the active one,
so navigation does not have to be memorized. The second row always starts with `q Quit`, `? Help`,
`Ctrl+P Commands`, and either `r Scan` or `x Cancel scan`; screen-specific actions follow when
space allows. Global actions have priority and remain visible at every supported terminal width.

`Ctrl+P` opens the complete command catalogue: type to filter it, use arrows or `j`/`k` to move,
`Home`/`End` and `PageUp`/`PageDown` for longer jumps, then press `Enter`. The palette scrolls
with the selection and shows both the visible range and the total command count.

| Key | Action |
| --- | --- |
| `1` … `7` | Dashboard, Devices, History, Compare, Alerts, Logs, Settings |
| `Ctrl+P` | Command palette with fuzzy action search |
| `?` | Contextual help |
| `j` / `k`, arrows | Next / previous item |
| `g` / `G`, Home / End | First / last item |
| `r` | Start a scan |
| `x` | Cancel the active scan |
| `q` | Quit; confirms when a scan is active |
| `Esc` | Close the top layer or return |

Device workspace:

| Key | Action |
| --- | --- |
| `Enter` | Open the selected inspector |
| `Space` | Toggle multi-selection |
| `/` | Fuzzy search across name, hostname, IP, MAC, vendor, and service names |
| `f` | Cycle all / online / offline / unknown / changed |
| `s` | Cycle status / name / address / vendor / latency / last-seen / confidence sort |
| `e` | Export guidance |

Device details:

| Key | Action |
| --- | --- |
| e | Confirm an exact identity/model locally, or clear it to restore automatic detection |
| t | Edit local tags |
| n | Edit a local note |

Settings:

| Key | Action |
| --- | --- |
| `j/k`, arrows, `Home/End` | Move through editable settings |
| `Enter` / `Space` | Change the selected setting |
| `t` | Open theme picker with live preview |
| `i` | Cycle Nerd Font / Unicode / ASCII icons |
| `a` | Toggle animation |
| `m` | Cycle scan mode |
| `c` | Switch between Detailed and Compact device rows |
| `s` | Confirm persistence on clean exit |

Detailed rows use the second line for device type and confidence, a MAC/interface hint, discovery
sources, last-seen age, and service endpoints. Compact rows keep the primary identity, address,
vendor, latency, and service names on one line. The selected density is persisted.

The device workspace responds to its actual width rather than only the terminal breakpoint.
On narrow windows the table keeps the full workspace and hides lower-priority columns before
identity or address become unusable. As space grows, services, vendor, and confidence return and
all flexible columns expand continuously. The inspector appears only when the table retains a
usable width and is capped on very wide terminals.

With mouse input enabled, click the numbered footer to switch screens, click settings to change
them, and use the wheel over device lists and menus. A device row is selected on the first click
and opened on the next; middle-click toggles multi-selection and right-click acts like `Esc`.
The mouse setting takes effect immediately.

Dialogs trap focus. `Esc` always closes the topmost overlay before affecting its underlying page.

## CLI

```text
lantern [--icons nerd|unicode|ascii]
lantern scan [--interface NAME] [--subnet CIDR] [--mode MODE] [--format table|json|xml|csv] [--output PATH]
lantern watch [--interface NAME] [--subnet CIDR] [--mode MODE]
lantern devices [--online] [--format table|json|xml|csv] [--output PATH]
lantern history [--limit N] [--format table|json|xml|csv] [--output PATH]
lantern alerts [--open] [--format table|json|xml|csv] [--output PATH]
lantern export --kind devices|history|alerts|baseline|comparison --format json|xml|csv [--output PATH]
lantern baseline create [--name NAME]
lantern baseline compare
lantern vendor update
lantern vendor import PATH
lantern vendor status
lantern config path
lantern config show
lantern config init [--force]
lantern doctor
```

Table output is intended for people. JSON, XML, and CSV output contains no headings, progress
messages, or decorations outside the selected serialization, so it can be piped directly into
`jq`, import tools, or another process. `--output -` is equivalent to stdout; any other path is
created or replaced. Successful file exports are silent. Errors go to stderr and return a nonzero
exit status. CSV files always contain headers, including when a query returns no records.

See [docs/CLI.md](docs/CLI.md) for the output contract, XML roots, and automation examples.

CLI help is currently English-only. Use `lantern <command> --help` for the exact options compiled
into your version. A terminal
does not expose whether its active font contains Nerd Font private-use glyphs, so Lantern never
enables them speculatively. Unicode is the safe default. Use `--icons nerd` for one run, set
`LANTERN_ICONS=nerd` in a terminal profile, or persist `icons = "nerd"` only after selecting a
Nerd Font in that terminal.

## Configuration

Lantern follows the platform configuration directories exposed by the operating system. Find
the exact path with:

```bash
lantern config path
lantern config init
lantern config show
```

See [config/lantern.example.toml](config/lantern.example.toml) for every stable setting and
[docs/CONFIGURATION.md](docs/CONFIGURATION.md) for behavior and safety limits.

The built-in defaults are intentionally moderate: normal mode, 96 concurrent connect attempts,
220 ms connect timeout, `/24`-friendly host limits, 30-second watch interval, SSDP/mDNS once per
scan, and HTTP/TLS metadata disabled.

## History and baselines

Each positive observation updates a stable device snapshot and its service/address timestamps.
After a completed scan, previously online devices without any observation in that scan transition
to offline. A device is never marked offline merely because it ignored ICMP.

Create a known-network baseline after reviewing the inventory:

```bash
lantern baseline create --name "Home network"
lantern baseline compare
```

The comparison separates unknown, missing, and changed devices. Baseline-unknown rows receive a
visible `!` marker even in monochrome and ASCII modes.

## Offline MAC vendor data

Lantern never sends a MAC address to a lookup API. Download and install the official IEEE MA-L,
MA-M, and MA-S public registries with:

```bash
lantern vendor update
lantern vendor status
```

The update downloads the registry files once, validates them, and stores the combined database
locally. To use another source instead, import an IEEE assignment CSV or a simple text file
containing `AA:BB:CC Vendor Name` lines:

```bash
lantern vendor import oui.csv
```

The file is validated before it replaces the local copy, longest-prefix matching is supported,
and loading happens away from the render loop. Manufacturer, hostname, advertised services, open
ports, and optional HTTP metadata are combined to infer a device type. Randomized/private MACs
have no public vendor assignment, so hostname and service evidence remain important.

Lantern keeps the broad type, inferred product family, platform, and supporting evidence separate.
For example, a hostname can support MacBook Pro or Apple iPhone, but it cannot prove an M1 chip
or an iPhone generation. Press e in device details to store an exact, user-confirmed identity;
that local value takes precedence and is never replaced by automatic inference. Ambiguous appliance
families stay generic until confirmed instead of borrowing a product category from a vendor name.
An unknown
prefix remains `Unknown`; it is not an error and does not trigger a lookup request during scans.

## Permissions

Ordinary TCP, DNS, SSDP, mDNS, and interface enumeration should not require administrator/root
access. On Linux, Lantern reads the kernel's public ARP neighbor table. Raw ICMP is not used by
the default build, so elevated privileges are neither required nor recommended.

Local firewalls can block multicast or outbound connects. Lantern reports the affected provider
as degraded and continues. It does not attempt to bypass policy.

## Security and privacy

Only use Lantern on networks you own or are explicitly authorized to inspect.

- No exploitation, authentication, credential use, brute force, stealth behavior, packet flood,
  form submission, path enumeration, or arbitrary command execution.
- TCP discovery uses the operating system's normal connect API.
- HTTP collection is opt-in, requests only `/`, reads at most 16 KiB, and follows zero redirects.
- Concurrency, timeouts, scan intervals, port sets, and subnet size are bounded and configurable.
- Discovered IPs, MACs, hostnames, services, notes, tags, and history stay in local files.
- Logs avoid packet bodies and never include credentials because Lantern never collects them.
- Cancellation is propagated to active providers; shutdown flushes the history writer and restores
  raw mode, alternate screen, mouse capture, and cursor state.

See [SECURITY.md](SECURITY.md) for threat boundaries and responsible reporting.

## Troubleshooting

Run `lantern doctor` first. It checks configuration, IPv4 interfaces/subnets, provider support,
database access, the vendor file, terminal color/Unicode hints, and the selected icon mode.

Common cases:

- **No usable interface:** specify `--interface`, or set `interface` in TOML. VPN and container
  adapters can otherwise look more active than the desired LAN.
- **Subnet rejected:** the CIDR exceeds `host_limit`. Use a narrower authorized CIDR rather than
  raising the limit casually.
- **mDNS degraded:** another daemon may prevent sharing UDP port 5353. TCP, DNS, ARP-cache, and
  SSDP discovery continue.
- **Few devices found:** many appliances expose no configured TCP service. Let mDNS/SSDP complete,
  interact with devices normally so the OS neighbor cache is populated, then rescan.
- **No colors despite a selected theme:** Lantern treats an explicit color theme as authoritative
  inside the TUI and overrides an inherited non-empty `NO_COLOR`. Choose Monochrome when colorless
  output is intentional; `lantern doctor` reports conflicts and terminals with `TERM=dumb`.
- **Garbled icons:** return to safe Unicode with `--icons unicode`, or select ASCII from Settings.
  Use Nerd mode only after configuring a Nerd Font in the active terminal profile.
- **Interrupted terminal:** modern shells usually recover with `reset`; please report any path
  that bypasses Lantern's restoration guard.

Structured logs are stored beside the SQLite database. The exact platform paths are printed by
`lantern config path` and `lantern doctor`.

## Architecture and development

The complete data flow, visual system, screen mockups, breakpoints, provider/event contracts,
SQLite schema, and phased checklist live in [docs/PRODUCT.md](docs/PRODUCT.md). A shorter code map
is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Contributions are welcome; start with [CONTRIBUTING.md](CONTRIBUTING.md). Tests use documentation
addresses and in-memory SQLite, never the developer's LAN.

## License

MIT — see [LICENSE](LICENSE).
