# cc-switch-cli

`cc-switch-cli` is an external Linux command-line tool for switching between Codex providers
already stored in [CC Switch](https://github.com/farion1231/cc-switch). It does not modify the
CC Switch source code or create, edit, or delete providers.

```bash
cc-switch-cli codex --name 'XXX'
```

## Compatibility

- Linux
- Validated against CC Switch `3.19.2`, `3.20.0`, `3.20.1`, and `4.0.15`
- Reviewed SQLite schemas: `16`, `17`, `18`, and `20`
- Codex providers

The CLI does not inspect or reject the installed CC Switch package version. Instead, it selects an
explicit compatibility profile from SQLite `PRAGMA user_version` and validates the tables and
columns required by that profile. An unknown schema emits an `unverified_schema` warning and attempts
the operation using the existing provider layout. There is no schema-number whitelist that blocks
unknown versions. Missing required data, unreadable layouts, and failed operations still return
errors and nonzero exit codes. An unrecognized package release that uses a reviewed schema does not
produce a version warning, because the package version is not inspected.

Settings compatibility is checked separately: a release can add settings without changing its
SQLite schema. CC Switch `4.0.15` still uses schema `20`; its adapter preserves the newer quota,
proxy client host, usage display, Codex stack, and local migration settings. If the selected adapter
would ignore a settings field (including a nested field), or cannot deserialize its value, the CLI
emits an `unverified_settings` warning and delegates switching to the installed GUI. It does not let
the older bundled writer discard these fields. Malformed JSON remains a state-read error; warnings
never include settings field names or values.

On an unknown schema (including `19`), or with settings the adapter cannot fully read, listing,
current-provider reads, and no-op switches work without the GUI when the stored state is readable.
A switch to another provider uses the installed
CC Switch tray interface. If CC Switch is closed, the CLI starts `cc-switch` from `PATH`, waits up to
10 seconds for its tray interface, and leaves it running. This path requires a graphical session
with a session D-Bus and StatusNotifier watcher. The CLI does not pass an unknown database to an
older bundled adapter, change its schema number, or migrate it itself. Startup and tray failures
remain errors. These checks detect unrecognized settings and required database structure; they
cannot detect every future change in behavior or guarantee compatibility with every release.

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
auto-failover. The 4.0.15 adapter preserves both settings, matching the current tray behavior.
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

When CC Switch is not running, the schema has a reviewed adapter, and that adapter can fully read
the settings, the profile selects the matching upstream
`ProviderService::switch` implementation:

- Schema `16`: CC Switch `3.19.2`, commit `43eaf07355af145aebfee301801779e824d4c221`
- Schema `17`: CC Switch `3.20.0`, commit `0b5da510168914b251481654a568c3ffacd62cf4`
- Schema `18`: CC Switch `3.20.1`, commit `3217f72596f2d1c0f879f0a05f83803825d9809f`
- Schema `20`: CC Switch `4.0.15`, commit `79e754ab1b6ed50da57dae1d0faecab4b3b52f1c`

Keeping separate direct adapters prevents a newer implementation from silently migrating an older
supported database. The CLI verifies `PRAGMA user_version` immediately before and after a direct
switch and rechecks settings compatibility before invoking the adapter. If incompatible settings
appear after backend selection, it returns an error before direct writes; retrying selects the GUI.
If the GUI is running but its tray D-Bus interface is unavailable, the CLI returns an error
instead of falling back to direct writes.

Each switch waits up to 10 seconds for the relevant state to converge:

- In direct mode, SQLite `providers.is_current` and `currentProviderCodex` in
  `~/.cc-switch/settings.json` must both identify the target.
- In 4.x proxy mode, `apps.codex.proxy_route` in `~/.cc-switch/live-state.json` must identify
  an existing target provider. The direct provider remains independent. An unfinished `pending`
  operation prevents success.
- The target tray item's `toggle-state` when the GUI is running

Tray matching handles legacy menus and 4.0.15's truncated names, duplicate-name suffixes, filtered
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

Each direct generation also checks settings using its own upstream `AppSettings` deserializer,
tracking ignored fields at every nesting level. This keeps the accepted settings format tied to
the pinned writer without maintaining a second list of fields. Settings that the writer does not
understand select the same GUI fallback as an unknown schema. This can also include retired keys
left over from earlier releases; the installed GUI handles its own settings migration.

Supporting a future schema or refreshing an adapter for a newer release requires:

1. Reviewing upstream migrations, settings, current-provider semantics, and tray behavior, even
   when `user_version` has not changed. Add a `SchemaVersion` profile and required shape checks
   when the database schema changes.
2. Reusing an existing read layout or adding new normalized queries when provider storage changes.
3. Pinning a matching upstream direct adapter that does not migrate another supported schema.
4. Adding read and direct-switch fixtures that assert `user_version` remains unchanged and newer
   settings survive a switch. Cover GUI fallback for unknown settings and nonzero errors when
   an operation fails.

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
