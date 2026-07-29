# Contributing to Wireseer

Thank you for improving Wireseer. Preserve its core promise: calm, explainable, local-only network
visibility without offensive behavior.

## Development

```bash
cargo build
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests must use documentation ranges (`192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`), parser
fixtures, in-memory SQLite, mocks, or loopback sockets. They must never scan the developer's LAN.

For visual changes, test at 160×40, 110×30, 80×24, and 54×12. Confirm focused, selected, empty,
loading, error, command-palette, help, monochrome, and ASCII states. Identity and IP must survive
before secondary columns. Regenerate the release screenshots with:

```bash
cargo run --example render_screenshots -- docs/screenshots
```

The screenshot generator must continue to use only the built-in network-free demo state and IETF
documentation addresses. Never capture a developer or user's inventory.

## Provider checklist

A discovery provider must have:

- a bounded request count and concurrency;
- a timeout around every network wait;
- cancellation at each wait;
- incremental typed observations with provenance;
- independent health/error reporting;
- no authentication, exploit, stealth, or destructive behavior;
- deterministic tests that do not depend on the real network;
- platform gating and actionable graceful degradation.

Do not have a provider mutate app state, write SQLite, or render. Extend `DiscoveryEvent` only when
an observation or provider-health event cannot carry the required information.

## Database changes

Add a numbered migration, preserve existing snapshots, enable foreign keys, and add migration
tests from both an empty database and the previous schema. Never rewrite a released migration.

## Pull requests

Keep changes cohesive, explain the user-visible outcome, list the validation commands, and include
an updated screenshot/SVG for layout work. Note traffic/privacy changes explicitly.

Release preparation and Homebrew packaging are documented in [docs/RELEASING.md](docs/RELEASING.md).
