use crate::error::AppError;
use rusqlite::Connection;

const PROVIDERS_V1_QUERY: &str = "SELECT id, name, is_current FROM providers \
     WHERE app_type = 'codex' \
     ORDER BY \
        CASE WHEN sort_index IS NULL THEN 1 ELSE 0 END, \
        sort_index, \
        created_at, \
        CASE WHEN sort_index IS NULL AND created_at IS NULL THEN name END, id";

const CURRENT_PROVIDER_V1_QUERY: &str = "SELECT id FROM providers \
     WHERE app_type = 'codex' AND is_current = 1 \
     ORDER BY id LIMIT 1";

const PROVIDERS_V1_COLUMNS: &[&str] = &[
    "id",
    "app_type",
    "name",
    "is_current",
    "sort_index",
    "created_at",
];

// Only direct writers need these additional checks before upstream initialization.
// GUI-backed profiles validate the provider layout they actually read.
const DIRECT_REQUIREMENTS: &[(i32, LayoutRequirement)] = &[
    (17, LayoutRequirement::Table("session_usage_dedup")),
    (
        18,
        LayoutRequirement::TableColumns {
            table: "session_log_sync",
            columns: &["last_byte_offset", "last_tail_fingerprint"],
        },
    ),
    (
        19,
        LayoutRequirement::TableColumns {
            table: "mcp_servers",
            columns: &["enabled_mcode"],
        },
    ),
    (
        19,
        LayoutRequirement::TableColumns {
            table: "skills",
            columns: &["enabled_mcode"],
        },
    ),
    (
        20,
        LayoutRequirement::TableColumns {
            table: "mcp_servers",
            columns: &["enabled_pi"],
        },
    ),
    (
        21,
        LayoutRequirement::TableColumns {
            table: "mcp_servers",
            columns: &["enabled_agy"],
        },
    ),
    (
        21,
        LayoutRequirement::TableColumns {
            table: "skills",
            columns: &["enabled_agy"],
        },
    ),
    (
        22,
        LayoutRequirement::TableColumns {
            table: "mcp_servers",
            columns: &["enabled_kimi"],
        },
    ),
    (
        22,
        LayoutRequirement::TableColumns {
            table: "skills",
            columns: &["enabled_kimi"],
        },
    ),
    (
        23,
        LayoutRequirement::TableColumns {
            table: "mcp_servers",
            columns: &["enabled_omp"],
        },
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaVersion(i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectGeneration {
    V16,
    V17,
    V18,
    V20,
    V23,
    InstalledGui,
}

impl DirectGeneration {
    // A schema number describes SQLite, not the device-local settings format.
    // Inspect the actual adapter's deserializer before letting it rewrite that file.
    pub fn supports_settings(self, text: &str) -> bool {
        match self {
            Self::V16 => settings_are_understood::<cc_switch_schema16::AppSettings>(text),
            Self::V17 => settings_are_understood::<cc_switch_schema17::AppSettings>(text),
            Self::V18 => settings_are_understood::<cc_switch_schema18::AppSettings>(text),
            Self::V20 => settings_are_understood::<cc_switch_schema20::AppSettings>(
                text.strip_prefix('\u{feff}').unwrap_or(text),
            ),
            Self::V23 => settings_are_understood::<cc_switch_schema23::AppSettings>(
                text.strip_prefix('\u{feff}').unwrap_or(text),
            ),
            Self::InstalledGui => true,
        }
    }
}

fn settings_are_understood<T: serde::de::DeserializeOwned>(text: &str) -> bool {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let mut ignored = false;
    let result: Result<T, _> = serde_ignored::deserialize(&mut deserializer, |_| ignored = true);
    result.is_ok() && !ignored && deserializer.end().is_ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurrentState {
    DatabaseOrLocal,
    Local,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutRequirement {
    Table(&'static str),
    TableColumns {
        table: &'static str,
        columns: &'static [&'static str],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompatibilityProfile {
    direct_generation: DirectGeneration,
    current_state: CurrentState,
}

impl SchemaVersion {
    #[cfg(test)]
    pub const fn new(number: i32) -> Self {
        Self(number)
    }

    pub fn detect(connection: &Connection) -> Result<Self, AppError> {
        let schema = Self(read_schema_number(connection)?);
        schema.validate(connection)?;
        Ok(schema)
    }

    pub const fn number(self) -> i32 {
        self.0
    }

    pub const fn providers_query(self) -> &'static str {
        PROVIDERS_V1_QUERY
    }

    pub const fn current_provider_query(self) -> &'static str {
        CURRENT_PROVIDER_V1_QUERY
    }

    pub const fn direct_generation(self) -> DirectGeneration {
        match self.profile() {
            Some(profile) => profile.direct_generation,
            None => DirectGeneration::InstalledGui,
        }
    }

    pub fn warning(self) -> Option<String> {
        match self.profile() {
            None => Some(format!(
                "unverified_schema: CC Switch database schema {number} has not been validated; \
                 attempting compatible reads and switching through the installed CC Switch GUI \
                 (started if needed)",
                number = self.0
            )),
            _ => None,
        }
    }

    pub const fn uses_live_state(self) -> bool {
        matches!(self.current_state(), CurrentState::Live)
    }

    pub fn pointers_converged(
        self,
        database: Option<&str>,
        local: Option<&str>,
        target: &str,
    ) -> bool {
        database == Some(target)
            && (local == Some(target)
                || (local.is_none() && self.current_state() == CurrentState::DatabaseOrLocal))
    }

    const fn current_state(self) -> CurrentState {
        match self.profile() {
            Some(profile) => profile.current_state,
            None => CurrentState::Live,
        }
    }

    // Read layout is stable from schema 1 onward. Only state semantics and the
    // writer vary; see docs/schema-support.md for the upstream history and limits.
    const fn profile(self) -> Option<CompatibilityProfile> {
        use CurrentState::{DatabaseOrLocal, Live, Local};
        use DirectGeneration::{InstalledGui, V16, V17, V18, V20, V23};
        let (direct_generation, current_state) = match self.0 {
            1 => (InstalledGui, DatabaseOrLocal),
            2..=15 => (InstalledGui, Local),
            16 => (V16, Local),
            17 => (V17, Local),
            18 => (V18, Local),
            19 => (InstalledGui, Live),
            20 => (V20, Live),
            21..=22 => (InstalledGui, Live),
            23 => (V23, Live),
            _ => return None,
        };
        Some(CompatibilityProfile {
            direct_generation,
            current_state,
        })
    }

    pub fn validate(self, connection: &Connection) -> Result<(), AppError> {
        require_table(connection, self, "providers")?;
        require_columns(connection, self, "providers", PROVIDERS_V1_COLUMNS)?;
        if self.direct_generation() != DirectGeneration::InstalledGui {
            for (introduced, requirement) in DIRECT_REQUIREMENTS {
                if *introduced <= self.0 {
                    requirement.validate(connection, self)?;
                }
            }
        }
        Ok(())
    }
}

impl LayoutRequirement {
    fn validate(self, connection: &Connection, schema: SchemaVersion) -> Result<(), AppError> {
        match self {
            Self::Table(table) => require_table(connection, schema, table),
            Self::TableColumns { table, columns } => {
                require_table(connection, schema, table)?;
                require_columns(connection, schema, table, columns)
            }
        }
    }
}

pub fn read_schema_number(connection: &Connection) -> Result<i32, AppError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(state_error)
}

fn require_table(
    connection: &Connection,
    schema: SchemaVersion,
    table: &str,
) -> Result<(), AppError> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, bool>(0),
        )
        .map_err(state_error)?;
    if exists {
        return Ok(());
    }
    Err(layout_error(schema, format!("missing table {table:?}")))
}

fn require_columns(
    connection: &Connection,
    schema: SchemaVersion,
    table: &str,
    required: &[&str],
) -> Result<(), AppError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(state_error)?;
    let columns = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(state_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(state_error)?;
    let missing = required
        .iter()
        .filter(|required| !columns.iter().any(|column| column == **required))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(layout_error(
        schema,
        format!("table {table:?} is missing columns: {}", missing.join(", ")),
    ))
}

fn layout_error(schema: SchemaVersion, detail: String) -> AppError {
    AppError::IncompatibleSchemaLayout {
        schema: schema.number(),
        detail,
    }
}

fn state_error(error: impl std::fmt::Display) -> AppError {
    AppError::State(error.to_string())
}
