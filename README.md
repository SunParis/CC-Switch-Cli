# cc-switch-cli

`cc-switch-cli` is an external Linux command-line tool for switching between Codex providers
already stored in [CC Switch](https://github.com/farion1231/cc-switch). It does not modify the
CC Switch source code or create, edit, or delete providers.

```bash
cc-switch-cli codex --name 'XXX'
```

## Compatibility

- Linux
- Validated against CC Switch `3.19.2`, `3.20.0`, `3.20.1`, and `4.0.6`
- Reviewed SQLite schemas: `16`, `17`, `18`, and `20`
- Codex providers

The CLI does not inspect or reject the installed CC Switch package version. Instead, it selects an
explicit compatibility profile from SQLite `PRAGMA user_version` and validates the tables and
columns required by that profile. An unknown schema emits an `unverified_schema` warning and attempts
the operation using the existing provider layout. There is no schema-number whitelist that blocks
unknown versions. Missing required data, unreadable layouts, and failed operations still return
errors and nonzero exit codes. An unrecognized package release that uses a reviewed schema does not
produce a version warning, because the package version is not inspected.

On an unknown schema (including `19`), listing, current-provider reads, and no-op switches work
without the GUI when the stored state is readable. A switch to another provider uses the installed
CC Switch tray interface. If CC Switch is closed, the CLI starts `cc-switch` from `PATH`, waits up to
10 seconds for its tray interface, and leaves it running. This path requires a graphical session
with a session D-Bus and StatusNotifier watcher. The CLI does not pass an unknown database to an
older bundled adapter, change its schema number, or migrate it itself. Startup and tray failures
remain errors; warning-only compatibility is an attempt, not a guarantee that every future layout
will work.

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

Selecting the provider that is already active succeeds without writing any files. Schemas 16–18
retain the legacy behavior: explicit switching preserves proxy-enabled state and disables
auto-failover. The 4.0.6 adapter preserves both settings, matching the current tray behavior.
When auto-failover is active in routing mode, manual provider selection is disabled; turn it off in
CC Switch first. Aggregation follows the provider choices exposed by CC Switch.

Compatibility warnings go to stderr. With `--json`, stderr warnings are JSON records with
`warning.kind` and `warning.message`; successful stdout responses also include a `warnings` array
(including lists, current-provider reads, and no-ops). Operation failures still emit an error record
and return a nonzero exit code, even if a compatibility warning was emitted first.

## GUI Synchronization

When CC Switch is running, the CLI activates its actual tray menu item through the Linux
StatusNotifier/dbusmenu D-Bus interface. The GUI process therefore remains responsible for the
switch, locking, proxy takeover, database updates, tray refresh, and frontend events. The CLI does
not write to the database concurrently.

When CC Switch is not running and the schema has a reviewed adapter, the profile selects the matching upstream
`ProviderService::switch` implementation:

- Schema `16`: CC Switch `3.19.2`, commit `43eaf07355af145aebfee301801779e824d4c221`
- Schema `17`: CC Switch `3.20.0`, commit `0b5da510168914b251481654a568c3ffacd62cf4`
- Schema `18`: CC Switch `3.20.1`, commit `3217f72596f2d1c0f879f0a05f83803825d9809f`
- Schema `20`: CC Switch `4.0.6`, commit `c261126630b3416e67b407da018cc614bbbe84cd`

Keeping separate direct adapters prevents a newer implementation from silently migrating an older
supported database. The CLI verifies `PRAGMA user_version` immediately before and after a direct
switch. If the GUI is running but its tray D-Bus interface is unavailable, the CLI returns an error
instead of falling back to direct writes.

Each switch waits up to 10 seconds for the relevant state to converge:

- In direct mode, SQLite `providers.is_current` and `currentProviderCodex` in
  `~/.cc-switch/settings.json` must both identify the target.
- In 4.x proxy mode, `apps.codex.proxy_route` in `~/.cc-switch/live-state.json` must identify
  an existing target provider. The direct provider remains independent. An unfinished `pending`
  operation prevents success.
- The target tray item's `toggle-state` when the GUI is running

Tray matching handles legacy menus and 4.0.6's truncated names, duplicate-name suffixes, filtered
provider lists, and project sections. A disabled, unavailable, or ambiguously identified provider
returns a GUI error.

Output is limited to provider names, UUIDs, the selected backend, and non-sensitive warning codes.
Credentials, URLs, and provider configuration are never printed.

## Extending Schema Support

Schema compatibility is centralized in `src/schema.rs`. Each supported schema explicitly selects a
normalized provider-table layout, validates its required database shape, and maps to a reviewed
direct-switch adapter. Reviewed schemas share the provider-table layout while using different
direct adapters. Schema `17` additionally requires its persistent session-usage
deduplication table. Schema `18` also requires the byte cursor and tail fingerprint columns added to
`session_log_sync` by the 17-to-18 migration. Schema `20` also checks `enabled_mcode` in `mcp_servers`
and `skills`, and `enabled_pi` in `mcp_servers`. Its current-provider reads account for device-local
proxy state. Unknown schemas use these current-state conventions when present and select the
installed GUI for writes.

Supporting a future schema requires:

1. Reviewing upstream migrations, current-provider semantics, and tray behavior, then adding its
   `SchemaVersion` profile and required shape checks.
2. Reusing an existing read layout or adding new normalized queries when provider storage changes.
3. Pinning a matching upstream direct adapter that does not migrate another supported schema.
4. Adding read and direct-switch fixtures that assert `user_version` remains unchanged.

Future schema numbers enter the warning-based fallback automatically. Reviewing a schema adds a
verified profile and direct adapter; it is not necessary just to attempt reads or GUI switching.

## Build and Install

Building requires Rust `1.85` or newer and the Linux system development libraries required by CC
Switch and Tauri. Integration tests also require `dbus-daemon`; they run a private bus, synthetic
provider data, and a stub executable. `CC_SWITCH_TEST_HOME` isolates the CLI and upstream adapters
under a temporary home directory without touching real configuration.

```bash
cargo test --locked
cargo build --release --locked
install -Dm755 target/release/cc-switch-cli ~/.local/bin/cc-switch-cli
```

Make sure `~/.local/bin` is included in `PATH`.

An optional read-only acceptance check compares the running GUI's selected tray item with its
stored current provider, without printing provider identities or switching anything:

```bash
cargo test --locked live_gui_current_provider_matches_stored_state -- --ignored
```

## Exit Codes

| Exit code | Meaning |
| --- | --- |
| `0` | Success, including a no-op |
| `2` | Invalid command-line arguments reported by Clap |
| `3` | Database, schema profile, schema-change, or state-read error |
| `4` | Provider not found or ambiguous provider name |
| `5` | GUI or D-Bus error |
| `6` | Switch failure or failure to converge within 10 seconds |
