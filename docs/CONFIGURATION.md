# Configuration reference

Lantern loads TOML from the operating system's application configuration directory. Use
`lantern config path` for the exact location and `lantern config init` to write current defaults.
Unknown fields are ignored for forward compatibility; known fields are validated before any scan.

## Network selection

- `interface`: optional exact interface name. It must have an IPv4 address.
- `subnet`: optional IPv4 CIDR. Without it, Lantern uses the selected interface's network.
- `scan_mode`: `quick`, `normal`, `deep`, `watch`, or `passive`.
- `refresh_interval_secs`: delay between completed watch scans. Scans never overlap.
- `host_limit`: hard preflight limit. Oversized subnets are rejected without sending traffic.

## Budgets

- `connect_timeout_ms`: timeout for each ordinary TCP connection; minimum 20 ms.
- `dns_timeout_ms`: timeout for each reverse lookup.
- `concurrency`: maximum simultaneous TCP connects, from 1 through 1,024.

Quick checks 10 common management/service ports. Normal checks 25 conservative LAN service ports.
Deep checks 37 ports and must be chosen explicitly. Passive sends no TCP connects; its local and
multicast providers still follow their individual enable flags.

## Providers

`enabled_protocols` switches providers independently. Disabling or failing one provider never
stops another. `local`, `tcp`, `reverse_dns`, `arp`, `mdns`, and `ssdp` have implementations in
this release. `icmp` and `netbios` are clean extension points and report unavailable when enabled.

`http_metadata` is separate because it performs an application-level request. It is off by
default. The collector makes one `GET /`, caps the response, does not authenticate, does not
submit forms, does not crawl, and records status/title/server/redirect target without following
the redirect. `tls_metadata` is modeled but its adapter is not enabled in this release.

## Appearance

- `theme`: `lantern_dark`, `catppuccin_mocha`, `catppuccin_latte`, `dracula`, `nord`,
  `midnight_blue`, `acid`, `paper_light`, `high_contrast`, `monochrome`, or `color_blind`.
  In Settings, select Theme (or press `t`). Arrow keys and the mouse wheel preview each palette
  immediately. Enter or a left click keeps it; Esc or right click restores the theme that was
  active before the picker opened. An explicit color theme overrides an inherited non-empty
  `NO_COLOR` inside the TUI; select `monochrome` when colorless output is intentional.
- `icons`: `unicode` is the safe default, `ascii` maximizes compatibility, and `nerd` uses
  private-use glyphs only when explicitly selected. Active terminal fonts cannot be detected
  reliably. `--icons MODE` or `LANTERN_ICONS=MODE` overrides this value for one run.
- `animations`: disables spinner/highlight motion when false.
- `compact_rows`: uses dense one-line device rows when true. The default Detailed mode uses its
  second line for type/confidence, MAC/interface, sources, last-seen age, and service endpoints.
- `mouse`: enables footer/menu clicks, device selection, settings activation, and list scrolling.
  Changes made in the TUI take effect immediately; every mouse action has a keyboard equivalent.
- `visible_columns`: preferred columns. Responsive breakpoints still remove secondary columns to
  preserve status, identity, and IP.

## Storage and logging

- `retention_days`: completed scans, events, observations, and inactive address/service rows older
  than this window are pruned during startup. Current inventory and active baselines are retained.
- `vendor_database`: optional local IEEE CSV or OUI text file. `lantern vendor update` installs the
  official IEEE public registries; `lantern vendor import` accepts another source. Both validate a
  local copy and update this field.
- `log_level`: tracing filter fallback (`error`, `warn`, `info`, `debug`, or `trace`). `RUST_LOG`
  takes precedence for developers.

Changes made in the TUI's safe settings page are written after an orderly exit. Advanced provider
and budget changes remain TOML-only so potentially noisy behavior is always deliberate.
