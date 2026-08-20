# cc-switch-cli

`cc-switch-cli` is an external Linux command-line tool for switching between Codex providers
already stored in [CC Switch](https://github.com/farion1231/cc-switch). It does not modify the
CC Switch source code or create, edit, or delete providers.

```bash
cc-switch-cli codex --name 'XXX'
```

## Compatibility

- Linux
- Validated against CC Switch `3.19.2`
- CC Switch SQLite schema `16`
- Codex providers

The CLI does not inspect or reject the installed CC Switch package version. It validates SQLite
schema `16` before reading or switching providers. GUI synchronization still expects a tray menu
layout compatible with CC Switch `3.19.2`; incompatible layouts fail with a GUI error instead of
falling back to direct writes.

## Usage

```bash
# Exact, case-sensitive name matching
cc-switch-cli codex --name 'XXX'

# Use the full UUID; providers with duplicate names require this form
cc-switch-cli codex --id 'xxx-xxx-xxx-xxx'

# List providers and show the current provider
cc-switch-cli codex --list
cc-switch-cli codex --current

# JSON output
cc-switch-cli --json codex --current
cc-switch-cli --json codex --name 'XXX'
```

Selecting the provider that is already active succeeds without writing any files. An explicit
switch preserves the Codex proxy-enabled state and disables auto-failover, matching the behavior
of selecting a provider from the CC Switch tray menu.

## GUI Synchronization

When CC Switch is running, the CLI activates its actual tray menu item through the Linux
StatusNotifier/dbusmenu D-Bus interface. The GUI process therefore remains responsible for the
switch, locking, proxy takeover, database updates, tray refresh, and frontend events. The CLI does
not write to the database concurrently.

When CC Switch is not running, the CLI calls `ProviderService::switch` from upstream commit
`43eaf07355af145aebfee301801779e824d4c221`. This reuses the official switching logic from version
3.19.2. If the GUI is running but its tray D-Bus interface is unavailable, the CLI returns an error
instead of falling back to direct writes.

Each switch waits up to 10 seconds and verifies:

- SQLite `providers.is_current`
- `currentProviderCodex` in `~/.cc-switch/settings.json`
- The target tray item's `toggle-state` when the GUI is running

Output is limited to provider names, UUIDs, the selected backend, and non-sensitive warning codes.
Credentials, URLs, and provider configuration are never printed.

## Build and Install

Building requires Rust `1.85` or newer and the Linux system development libraries required by CC
Switch and Tauri.

```bash
cargo test --locked
cargo build --release --locked
install -Dm755 target/release/cc-switch-cli ~/.local/bin/cc-switch-cli
```

Make sure `~/.local/bin` is included in `PATH`.

## Exit Codes

| Exit code | Meaning |
| --- | --- |
| `0` | Success, including a no-op |
| `2` | Invalid command-line arguments reported by Clap |
| `3` | Database, schema, or state-read error |
| `4` | Provider not found or ambiguous provider name |
| `5` | GUI or D-Bus error |
| `6` | Switch failure or failure to converge within 10 seconds |
