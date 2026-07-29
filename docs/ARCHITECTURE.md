# Architecture overview

Wireseer is an event-driven application with one owner for user-visible mutable state.

```text
interface selection ──> scan coordinator ──> bounded providers
                                                │
                                                v
                                         DiscoveryEvent
                                                │
input ──> action mapping ──> AppState reducer ──┼──> render snapshot
                                                ├──> diff / alerts
                                                └──> history writer thread ──> SQLite
```

## Boundaries

- `network` enumerates interfaces and runs providers. It has no TUI dependency.
- `devices` normalizes observations, merges facts, names services, and calculates weighted
  fingerprints. Lower-confidence inferences cannot overwrite observed or user data.
- `app` owns navigation, filters, selection, scan lifecycle, incremental merge, alerts, logs,
  and render state.
- `history` owns migrations, snapshots, events, scan diffs, and baselines.
- `tui` maps terminal input to app operations and renders immutable references. No widget performs
  network or database work.
- the binary composes the parts, gives SQLite to a serialized writer thread, and exposes CLI
  workflows.

The discovery channel is bounded. TCP jobs are consumed with `buffer_unordered`, capped by the
configured concurrency, and wrap every connect in both timeout and cancellation selection.
Multicast providers send one standards-based query and have sub-second receive windows. The
reverse resolver is separately bounded. A provider can emit degraded health without failing the
scan.

## Identity and confidence

An `Observation` is a timestamped claim from exactly one `DiscoverySource`. `Device::merge`
accumulates sources, services, and stronger missing facts. `UserMetadata` remains separate.
Fingerprinting returns a device type plus the evidence items and weights that produced its score.
The inspector marks inferred type with `~` and shows each evidence contribution.

Stable identity prefers a MAC when one has been observed and otherwise uses IPv4. The address and
service tables preserve first/last timestamps; snapshot JSON allows forward-compatible full-device
loading while normalized tables remain queryable.

## Rendering and terminal safety

Responsive class selection is a pure function tested at exact boundaries. The renderer uses
semantic theme tokens and stable constraints. The event loop draws only after input, discovery,
or a low-rate functional tick. A terminal guard restores raw mode, mouse capture, alternate screen,
and cursor on normal errors and `Drop`; a panic hook performs the same recovery before delegating.

For the full schema, screen designs, key model, and phased record, see [PRODUCT.md](PRODUCT.md).

