# Schema support and verification

The CLI supports Codex provider reads and GUI switching for SQLite schemas **1–23**.
The latest reviewed release is **CC Switch 4.0.21**, commit
`c4db63c60d2a1ddcb4bd9f81152b9c6126413659`. This concerns SQLite `PRAGMA user_version`,
not the package version or the pre-SQLite JSON configuration format.

## Supported operations

| Schemas | List/current/no-op | Switch using installed GUI | Direct switch with GUI closed |
| --- | --- | --- | --- |
| 1–15 | Yes | Yes | Starts GUI |
| 16 | Yes | Yes | Bundled 3.19.2 adapter |
| 17 | Yes | Yes | Bundled 3.20.0 adapter |
| 18 | Yes | Yes | Bundled 3.20.1 adapter |
| 19 | Yes | Yes | Starts GUI |
| 20 | Yes | Yes | Bundled 4.0.15 adapter |
| 21–22 | Yes | Yes | Starts GUI |
| 23 | Yes | Yes | Bundled 4.0.21 adapter |
| Unknown, including 0 and 24+ | Attempt with warning | Attempt with warning | Starts GUI |

GUI switching requires a graphical session, session D-Bus, a StatusNotifier watcher, and a visible
Codex tray section. When closed, `cc-switch` must be on `PATH`. The GUI remains running afterward.
Direct switching also requires settings fully understood by the bundled adapter; otherwise the
CLI warns and uses the GUI. Failed reads, disabled/ambiguous tray items, failed activation, and
failure to converge remain errors with nonzero exit codes.

The installed GUI must be appropriate for its database. Starting a newer GUI against an old database
can trigger CC Switch's own migration. The CLI detects a schema change and stops before activating a
provider; retry after the GUI finishes. The CLI never changes `user_version` to force compatibility,
and never invokes a newer bundled writer on an older schema.

## Historical evidence

The provider projection (`id`, `app_type`, `name`, `is_current`, `sort_index`, `created_at`) exists
from the first versioned SQLite database onward. Subsequent migrations change other tables or add
provider fields that the read projection does not need. The source history below records the
relevant migration milestones; abbreviated commits belong to
[farion1231/cc-switch](https://github.com/farion1231/cc-switch).

| Schema | Main migration change | Source commit |
| --- | --- | --- |
| 1 | Initial versioned SQLite storage | `a7ca6fb985e5` |
| 2 | Proxy/usage tables and skill storage changes | `b1103c8a5942` |
| 3 | Unified skill storage | `ff03ca1e632f` |
| 4 | OpenCode MCP/skill flags | `5658d93924a3` |
| 5 | Pricing configuration and request-model tracking | `785e1b5add5e` |
| 6 | Daily usage rollups | `bf40b0138c82` |
| 7 | Skill content hashes and update timestamps | `e3179ad9e451` |
| 8 | Session usage tracking and sync cursors | `154342ca00c4` |
| 9 | Pricing data refresh | `bb7c83c214b5` |
| 10 | Hermes MCP/skill flags | `a2e9e1938b8c` |
| 11 | Pricing dimensions in usage records/rollups | `42828566831b` |
| 12 | Project profiles | `8f018a2d45a0` |
| 13 | Input-token cache semantics | `f991726ff0cf` |
| 14 | Grok Build proxy configuration, an intermediate step to 15 | `1c0ee0c58ab9` |
| 15 | Grok Build MCP/skill flags | `1c0ee0c58ab9` |
| 16 | Rebuild Codex session usage | `eff1e0ccfc0e` |
| 17 | Session usage deduplication ledger | `40d747c009bf` |
| 18 | Byte offset and tail fingerprint in session cursors | `bcee61be4330` |
| 19 | MiniMax Code MCP/skill flags | `06082e189d65` |
| 20 | Pi MCP flag | `d455dd85720a` |
| 21 | Antigravity MCP/skill flags | `c7dceb803684` |
| 22 | Kimi MCP/skill flags | `e4873bc8aa25` |
| 23 | Oh My Pi MCP flag | `3d7cf6aa32e1` |

Schema numbers are not complete format fingerprints. Development history temporarily used schema 6
for OpenClaw flags (`7f46a0b9104c`, reverted by `28b125b34f32`) and schema 14 for cache-write TTL
accounting (`13e7c1fcc435`, reverted by `6eb217b24257`). Their provider projection is unchanged.
GUI-backed profiles do not require those unrelated tables to resemble a later reuse of the number.

Current-provider and tray behavior also changed within a schema:

- Early schema 1 used SQLite alone. Device-local `currentProviderCodex` appeared in schema 1 at
  `ea7169abc05b`. The CLI accepts SQLite-only convergence for schema 1 when that pointer is absent;
  when present, both pointers must agree. Schemas 2 onward require both in direct mode.
- Old trays used disabled `─── Codex ───` headings. `164635f63890` changed these to plain `Codex`.
  Some schema-4/5 trays also prepended a localized Auto/Failover checkbox. The parser isolates the
  Codex section and handles that extra row without selecting a similarly named provider in another app.
- Submenus appeared during schema 6 at `fe525891d4d0`; usage labels were added during schema 10 at
  `dc04165f18d3`. Both bare `Codex` and `Codex · ...` submenu labels are recognized.
- Separate direct/proxy state appeared during schema 19 at `15c0b3ce23d8`. For schemas 19 onward,
  `live-state.json` proxy routes take precedence when present; absent live state falls back to
  direct pointers. A pending operation prevents success. Earlier schemas ignore leftover live state.

## Verification scope

Tests exercise reads, no-op selection, and D-Bus switching for every number from 1 through 23 on
temporary databases and a private session bus. Historical fixtures reproduce the tables/columns
used by the CLI, including old skill layouts and later migration additions. They omit unrelated
usage/pricing data and are not complete copies of historical installations. They are maintained
independently of the production compatibility checks.

Direct-switch tests initialize real databases through each bundled upstream adapter, switch synthetic
providers, and check that schema numbers and expected settings survive. Tests also ensure that
GUI-backed schemas cannot enter any bundled writer, unknown schemas/settings still fall back with
warnings, and actual failures stay errors. Legacy tray tests cover duplicate names, Auto/Failover
name collisions, and isolation from other applications' rows.

These are source reviews and automated fixtures, not a claim that every historical GUI binary has
been run. The optional live acceptance check verifies the installed GUI's selected tray item against
stored state without changing providers. Future releases can change behavior while keeping a schema
number; review their settings, tray, and current-state semantics before refreshing an adapter.
