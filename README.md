# cc-switch-cli

`cc-switch-cli` is an external Linux command-line tool for switching between Codex providers
already stored in [CC Switch](https://github.com/farion1231/cc-switch). It does not modify the
CC Switch source code or create, edit, or delete providers.

```bash
cc-switch-cli codex --name 'XXX'
```

## Compatibility

- Linux
- Validated against CC Switch `3.19.2` and `3.20.0`
- CC Switch SQLite schemas `16` and `17`
- Codex providers

The CLI does not inspect or reject the installed CC Switch package version. Instead, it selects an
explicit compatibility profile from SQLite `PRAGMA user_version` and validates the tables and
columns required by that profile. Unknown schema versions and malformed known schemas are rejected.
GUI synchronization expects a tray menu layout compatible with the validated CC Switch versions;
incompatible layouts fail with a GUI error instead of falling back to direct writes.

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

When CC Switch is not running, the compatibility profile selects the matching upstream
`ProviderService::switch` implementation:

- Schema `16`: CC Switch `3.19.2`, commit `43eaf07355af145aebfee301801779e824d4c221`
- Schema `17`: CC Switch `3.20.0`, commit `0b5da510168914b251481654a568c3ffacd62cf4`

Keeping separate direct adapters prevents the schema `17` implementation from silently migrating a
schema `16` database. The CLI verifies `PRAGMA user_version` immediately before and after a direct
switch. If the GUI is running but its tray D-Bus interface is unavailable, the CLI returns an error
instead of falling back to direct writes.

Each switch waits up to 10 seconds and verifies:

- SQLite `providers.is_current`
- `currentProviderCodex` in `~/.cc-switch/settings.json`
- The target tray item's `toggle-state` when the GUI is running

Output is limited to provider names, UUIDs, the selected backend, and non-sensitive warning codes.
Credentials, URLs, and provider configuration are never printed.

## Extending Schema Support

Schema compatibility is centralized in `src/schema.rs`. Each supported schema explicitly selects a
normalized provider-table layout, validates its required database shape, and maps to a reviewed
direct-switch adapter. Schemas `16` and `17` currently share the same provider layout while using
different direct adapters.

Supporting a future schema requires:

1. Adding its `SchemaVersion` profile and required shape checks.
2. Reusing an existing read layout or adding new normalized queries when provider storage changes.
3. Pinning a matching upstream direct adapter that does not migrate another supported schema.
4. Adding read and direct-switch fixtures that assert `user_version` remains unchanged.

Future schema numbers are never accepted by range. This makes an unsupported database fail closed
until its read and write behavior has been reviewed.

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
| `3` | Database, schema profile, schema-change, or state-read error |
| `4` | Provider not found or ambiguous provider name |
| `5` | GUI or D-Bus error |
| `6` | Switch failure or failure to converge within 10 seconds |
